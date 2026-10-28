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

  async function refreshRecentConversationsForHost() {
    try {
      const signals = await loadOptionalCodexAppModule("app-server-manager-signals-");
      const sendRequest = Object.values(signals || {}).find((candidate) => {
        if (typeof candidate !== "function") return false;
        try {
          const source = Function.prototype.toString.call(candidate).replace(/\s+/g, "");
          return /^function[$\w]+\(e,t\)\{return[$\w]+\.sendRequest\(e,t\)\}$/.test(source);
        } catch {
          return false;
        }
      });
      if (typeof sendRequest !== "function") return false;
      await sendRequest("refresh-recent-conversations-for-host", { hostId: "local", sortKey: "updated_at" });
      return true;
    } catch (error) {
      window.__codexRecentConversationRefreshFailures = window.__codexRecentConversationRefreshFailures || [];
      window.__codexRecentConversationRefreshFailures.push(String(error?.stack || error));
      return false;
    }
  }

  /**
   * 同一时间最多显示几条 toast。
   *
   * 原来是「新 toast 顶掉旧 toast」的单例语义。拓展也能弹 toast 之后，单例会让
   * 第三方提示把「删除成功（可撤销）」这类关键反馈挤掉，所以改成有界队列：超出
   * 上限时挤掉最旧的一条，而不是最关键的当前一条。
   */
  const codexPlusToastLimit = 3;
  const codexPlusToastLifetimeMs = 10000;
  const codexPlusToastGapPx = 48;

  /**
   * 按当前 DOM 顺序重排所有提示的纵向位置。
   *
   * 必须在每次「新增」和「移除」之后都调用：位置只在插入那一刻算的话，一旦有
   * 人被挤掉或超时消失，剩下几条会停在自己的旧层号上，出现空档和重叠。
   */
  function layoutCodexPlusToasts() {
    document.querySelectorAll(".codex-delete-toast").forEach((node, index) => {
      node.style.bottom = `${18 + index * codexPlusToastGapPx}px`;
    });
  }

  /** 移除一条提示并立刻重排剩下的。 */
  function dismissCodexPlusToast(toast) {
    toast.remove();
    layoutCodexPlusToasts();
  }

  /**
   * 显示一条提示。
   *
   * `options.type` 取 info / success / warn / error，对应 styles 里的四条配色；
   * 不传则保持原先的默认外观。`options.undoToken` 会追加「撤销」按钮——这是
   * 内部删除流程用的，拓展一般用不到。
   */
  function showToast(message, options = {}) {
    // 兼容旧调用点：老签名是 showToast(message, undoToken)。第二个参数传字符串
    // 时按 undoToken 处理，传对象时按新签名处理。
    const settings = typeof options === "string" ? { undoToken: options } : (options || {});
    const undoToken = settings.undoToken;
    const type = typeof settings.type === "string" ? settings.type : "";
    const live = document.querySelectorAll(".codex-delete-toast");
    // 队列满时挤掉最旧的（DOM 顺序即插入顺序）。用 dismiss 而不是裸 remove，
    // 它会顺带重排剩下几条的位置。
    if (live.length >= codexPlusToastLimit) {
      for (let index = 0; index <= live.length - codexPlusToastLimit; index += 1) {
        dismissCodexPlusToast(live[index]);
      }
    }
    const toast = document.createElement("div");
    toast.className = "codex-delete-toast";
    if (type) toast.dataset.toastType = type;
    toast.textContent = message;
    if (undoToken) {
      const undo = document.createElement("button");
      undo.textContent = "撤销";
      undo.addEventListener("click", async () => {
        const result = await postJson("/undo", { undo_token: undoToken });
        toast.textContent = result.message || "撤销完成";
        if (result.status === "undone") {
          settings.onUndone?.();
          const refreshed = await refreshRecentConversationsForHost();
          if (!refreshed) window.location.reload();
        }
        setTimeout(() => dismissCodexPlusToast(toast), 5000);
      });
      toast.appendChild(undo);
    }
    document.body.appendChild(toast);
    // append 之后统一重排：此时这条才进入 DOM，索引才是它真实的层号。
    layoutCodexPlusToasts();
    setTimeout(() => dismissCodexPlusToast(toast), codexPlusToastLifetimeMs);
    return () => dismissCodexPlusToast(toast);
  }

  function shareBase64Url(bytes) {
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }

  function shareTextFromElement(element) {
    const clone = element.cloneNode(true);
    clone.querySelectorAll?.("button, textarea, input, select, [contenteditable='true'], .codex-delete-toast, .codex-plus-modal-overlay, .codex-plus-page-overlay, .codex-session-share-button").forEach((node) => node.remove());
    return String(clone.innerText || clone.textContent || "").replace(/\u00a0/g, " ").replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim();
  }

  function sessionShareMarkdown() {
    const ref = currentSessionRef();
    if (!ref.session_id) return { ref, markdown: "" };
    const root = conversationRoot();
    if (!root) return { ref, markdown: "" };
    const authored = Array.from(root.querySelectorAll("[data-message-author-role]"));
    const knownTurns = Array.from(root.querySelectorAll([
      '[data-testid="conversation-turn"]',
      '[data-testid*="message"]',
      '[data-message-content]',
      'main .prose',
      '[class*="message-bubble"]',
      '[class*="MessageBubble"]',
      '[class*="user-message"]',
      '[class*="UserMessage"]',
    ].join(",")));
    const turns = authored.length ? authored : knownTurns;
    const seen = new Set();
    const messages = turns.map((node) => {
      if (!(node instanceof HTMLElement) || seen.has(node)) return "";
      if (node.parentElement?.closest?.('[data-message-author-role], [data-testid="conversation-turn"]')) return "";
      seen.add(node);
      const text = shareTextFromElement(node);
      if (!text) return "";
      const role = String(node.getAttribute("data-message-author-role") || "").toLowerCase();
      const label = role === "user" ? "用户" : role === "assistant" ? "助手" : "消息";
      return { role: role === "user" || role === "assistant" ? role : "message", label, text };
    }).filter(Boolean);
    const title = String(ref.title || document.querySelector(selectors.threadTitle)?.textContent || "未命名会话").replace(/\s+/g, " ").trim();
    let content = messages.map((message) => `### ${message.label}\n\n${message.text}`).join("\n\n");
    if (!content) {
      const fallback = root.cloneNode(true);
      fallback.querySelectorAll?.([
        ".composer-footer", ".composer-surface-chrome", "form", "header", "nav", "aside",
        "button", "textarea", "input", "select", "[contenteditable='true']",
        ".codex-delete-toast", ".codex-plus-modal-overlay", ".codex-plus-page-overlay",
        ".codex-session-share-button",
      ].join(",")).forEach((node) => node.remove());
      content = String(fallback.innerText || fallback.textContent || "")
        .replace(/\u00a0/g, " ")
        .replace(/[ \t]+\n/g, "\n")
        .replace(/\n{3,}/g, "\n\n")
        .trim();
      if (content) messages.push({ role: "message", label: "会话", text: content });
    }
    const markdown = `# ${title || "未命名会话"}\n\n- 会话 ID：\`${ref.session_id}\`\n\n${content}`.slice(0, codexPlusShareMaxCharacters);
    return {
      ref,
      markdown: content ? markdown : "",
      session: content ? {
        version: 1,
        kind: "codex-session",
        session_id: ref.session_id,
        title: title || "未命名会话",
        messages: messages.map(({ role, text }) => ({ role, text })),
      } : null,
    };
  }

  async function encryptSessionShare(value) {
    const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, true, ["encrypt"]);
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const ciphertext = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, new TextEncoder().encode(value));
    const exportedKey = await crypto.subtle.exportKey("raw", key);
    return {
      key: shareBase64Url(new Uint8Array(exportedKey)),
      encrypted: {
        v: 1,
        iv: shareBase64Url(iv),
        ciphertext: shareBase64Url(new Uint8Array(ciphertext)),
      },
    };
  }

  async function createSessionShare() {
    const { ref, markdown, session } = sessionShareMarkdown();
    if (!ref.session_id) {
      showToast("当前页面还没有可分享的会话", null);
      return;
    }
    if (!markdown || !session) {
      showToast("当前会话还没有可分享的消息", null);
      return;
    }
    const shareWindow = window.open("about:blank", "_blank");
    const button = document.querySelector(`.${sessionShareButtonClass}`);
    if (button) {
      button.disabled = true;
      button.setAttribute("aria-busy", "true");
      button.textContent = "正在创建…";
    }
    try {
      let shareDocument = session;
      const nativeSession = await postJson("/session/export", {
        session_id: ref.session_id,
        title: session.title,
      });
      if (nativeSession?.status !== "ok" || nativeSession.kind !== "codex-rollout" || typeof nativeSession.content !== "string") {
        throw new Error(nativeSession?.message || "无法读取完整 Codex 会话文件");
      }
      shareDocument = { ...nativeSession, title: session.title };
      const encrypted = await encryptSessionShare(JSON.stringify(shareDocument));
      const payload = { ttl: 604800, encrypted: encrypted.encrypted };
      let result;
      let baseUrl = codexPlusShareBaseUrl;
      try {
        result = await postJson("/share/create", payload);
        if (result?.id) {
          baseUrl = codexPlusShareBaseUrl;
        } else if (result?.status !== "failed") {
          throw new Error(result?.message || "创建分享失败");
        }
      } catch (_) {
        result = null;
      }
      if (!result?.id) {
        let response;
        try {
          response = await fetch(`${baseUrl}/api/shares`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(payload) });
        } catch (_) {
          baseUrl = codexPlusShareFallbackBaseUrl;
          response = await fetch(`${baseUrl}/api/shares`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(payload) });
        }
        result = await response.json().catch(() => ({}));
        if (!response.ok || !result.id) throw new Error(result.error || `创建分享失败（HTTP ${response.status}）`);
      }
      const shareUrl = `${baseUrl}/?s=${encodeURIComponent(result.id)}#k=${encrypted.key}`;
      try {
        await navigator.clipboard.writeText(shareUrl);
      } catch (_) {
        const input = document.createElement("input");
        input.value = shareUrl;
        input.style.position = "fixed";
        input.style.opacity = "0";
        document.body.appendChild(input);
        input.select();
        document.execCommand("copy");
        input.remove();
      }
      showToast("会话分享链接已复制", null);
      if (shareWindow && !shareWindow.closed) shareWindow.location.href = shareUrl;
    } catch (error) {
      if (shareWindow && !shareWindow.closed) shareWindow.close();
      showToast(error?.message || "创建分享失败，请稍后重试", null);
    } finally {
      if (button) {
        button.disabled = false;
        button.removeAttribute("aria-busy");
        button.textContent = "分享会话";
      }
    }
  }

  function sessionSharePlacement() {
    // 会话操作锚点唯一时才挂载，不能把右侧审查/浏览器的 header 当作会话栏。
    const headerSelector = 'header, [data-app-shell-header-edge-scroll], [class*="_Header_"]';
    const anchors = Array.from(document.querySelectorAll('[data-testid="app-shell-header-context-menu-surface"]'))
      .filter((node) => visibleElement(node) && !node.closest('[data-codex-plus-ext]'));
    if (anchors.length !== 1) return null;
    const header = anchors[0].closest(headerSelector);
    if (!(header instanceof HTMLElement) || !visibleElement(header)) return null;
    const nativeShare = Array.from(header.querySelectorAll('button[aria-label="Share"], button[aria-label="分享"], button[aria-label*="Share"], button[aria-label*="分享"]'))
      .find((node) => visibleElement(node) && !node.closest('[data-codex-plus-ext]') && !node.classList.contains(sessionShareButtonClass) && node.closest(headerSelector) === header);
    const nativeGroup = nativeShare?.closest?.(".ms-auto");
    if (nativeGroup && header.contains(nativeGroup)) return nativeGroup;
    const groups = Array.from(header.querySelectorAll(".ms-auto"))
      .filter((node) => visibleElement(node) && !node.closest('[data-codex-plus-ext]') && node.closest(headerSelector) === header);
    return groups.length === 1 ? groups[0] : null;
  }

  function installSessionShareButton() {
    const existing = document.querySelectorAll(`.${sessionShareButtonClass}`);
    const ref = currentSessionRef();
    const actionGroup = codexPlusSettings().sessionShare && ref.session_id ? sessionSharePlacement() : null;
    if (!(actionGroup instanceof HTMLElement)) {
      existing.forEach((button) => button.remove());
      return;
    }
    let button = existing[0];
    existing.forEach((node) => { if (node !== button) node.remove(); });
    if (!button) {
      button = document.createElement("button");
      button.type = "button";
      button.className = `${sessionShareButtonClass} ${headerContextButtonClass}`;
      button.textContent = "分享会话";
      button.setAttribute("aria-label", "分享当前会话");
      button.setAttribute("data-codex-plus-ext", "session-share");
      button.dataset.codexSessionShareVersion = sessionShareButtonVersion;
      button.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        void createSessionShare();
      }, true);
    }
    button.setAttribute("data-codex-plus-ext", "session-share");
    button.style.position = "static";
    button.style.pointerEvents = "auto";
    button.style.webkitAppRegion = "no-drag";
    button.style.marginLeft = "";
    // 宿主重建时才搬动，避免每轮扫描再次触发 DOM mutation（issue #1960）。
    if (button.parentElement !== actionGroup) actionGroup.appendChild(button);
  }

  function sessionImportMarkdown(session) {
    const title = String(session?.title || "未命名会话").trim() || "未命名会话";
    const messages = Array.isArray(session?.messages) ? session.messages : [];
    const body = messages.map((message) => {
      const role = message?.role === "user" ? "用户" : message?.role === "assistant" ? "助手" : "消息";
      const text = String(message?.text || "").trim();
      return text ? `### ${role}\n\n${text}` : "";
    }).filter(Boolean).join("\n\n");
    return `# ${title}\n\n${body}`.trim();
  }

  function importSharedSessionIntoNewChat(session) {
    if (session?.kind === "codex-rollout" && typeof session.content === "string") {
      void postJson("/session/import", session).then((result) => {
        if (result?.status !== "ok") {
          showToast(result?.message || "原生会话导入失败", null);
          return;
        }
        void refreshRecentConversationsForHost();
        showToast("已导入完整 Codex 会话", null);
      }).catch((error) => showToast(error?.message || "原生会话导入失败", null));
      return;
    }
    const markdown = sessionImportMarkdown(session);
    if (!markdown) {
      showToast("分享内容为空，无法导入", null);
      return;
    }
    const newChat = Array.from(document.querySelectorAll("button")).find((button) => {
      if (!visibleElement(button) || isExtensionUiNode(button)) return false;
      const text = String(button.textContent || "").replace(/\s+/g, " ").trim();
      const label = button.getAttribute("aria-label") || "";
      return /^(新对话|New chat)$/i.test(text) || /^(新对话|New chat)$/i.test(label);
    });
    if (newChat instanceof HTMLElement) newChat.click();
    const deadline = Date.now() + 5000;
    const fill = () => {
      const editor = Array.from(document.querySelectorAll("textarea, [contenteditable='true']"))
        .filter((node) => visibleElement(node))
        .at(-1);
      if (!(editor instanceof HTMLElement)) {
        if (Date.now() < deadline) window.setTimeout(fill, 100);
        else showToast("无法找到 Codex 输入框，请手动打开新对话后重试", null);
        return;
      }
      editor.focus();
      if (editor instanceof HTMLTextAreaElement) {
        const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
        setter?.call(editor, markdown);
        editor.dispatchEvent(new Event("input", { bubbles: true }));
      } else {
        document.execCommand("insertText", false, markdown);
        editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: markdown }));
      }
      showToast("已导入完整会话内容，请发送以继续", null);
    };
    window.setTimeout(fill, newChat ? 350 : 0);
  }

  function installSessionShareImportListener() {
    window.removeEventListener("message", window.__codexSessionShareImportHandler);
    window.__codexSessionShareImportHandler = (event) => {
      if (!/^(https:\/\/share\.codexpp\.cc|https:\/\/codexpp-share\.pages\.dev)$/.test(event.origin || "") || event.data?.type !== "codexpp-import-session") return;
      const session = event.data?.session;
      if (!session || !["codex-session", "codex-rollout"].includes(session.kind)) return;
      if (session.kind === "codex-session" && !Array.isArray(session.messages)) return;
      if (session.kind === "codex-rollout" && typeof session.content !== "string") return;
      importSharedSessionIntoNewChat(session);
    };
    window.addEventListener("message", window.__codexSessionShareImportHandler);
  }

  function visibleElement(node) {
    if (!(node instanceof Element)) return false;
    const rect = node.getBoundingClientRect?.();
    return !!rect && rect.width > 0 && rect.height > 0;
  }

  function normalizedElementText(node) {
    return (node?.innerText || node?.textContent || "").replace(/\s+/g, " ").trim();
  }

  function escapeHtml(value) {
    return String(value)
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#39;");
  }

  function confirmDelete(title, hostId = "local") {
    document.querySelectorAll(".codex-delete-confirm-overlay").forEach((node) => node.remove());
    return new Promise((resolve) => {
      const overlay = document.createElement("div");
      overlay.className = "codex-delete-confirm-overlay";
      overlay.innerHTML = `
        <div class="codex-delete-confirm-content" role="dialog" aria-modal="true" aria-label="删除会话">
          <div class="codex-delete-confirm-title">删除会话</div>
          <div class="codex-delete-confirm-message">删除“${escapeHtml(title)}”？${hostId !== "local" ? "这将永久删除远端任务及其子任务，无法撤销。" : ""}</div>
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
    const currentId = locationThreadId();
    const currentIdentity = normalizedCodexThreadUuid(currentId) || currentId;
    const rowIdentity = normalizedCodexThreadUuid(ref.session_id) || ref.session_id;
    if (currentIdentity && currentIdentity !== rowIdentity) return false;
    // 新版原生 sidebarThreadRow 明确提供 active；false 不能被相同 pathname 覆盖。
    const nativeActive = row.getAttribute("data-app-action-sidebar-thread-active");
    if (nativeActive === "true" || nativeActive === "false") return nativeActive === "true";
    if (row.getAttribute("aria-current") === "page" || row.getAttribute("aria-current") === "true") return true;
    const href = rowHref(row);
    if (href) {
      try {
        const url = new URL(href, window.location.href);
        if (currentIdentity && url.href === window.location.href) return true;
      } catch {}
    }
    return !!currentIdentity && currentIdentity === rowIdentity;
  }

  function releaseDeleteFocus(row, button) {
    button.blur();
    if (row.contains(document.activeElement)) {
      document.activeElement.blur();
    }
  }

  function sameDeletedSessionRef(left, right) {
    return !!left?.session_id && !!left?.host_id && left.session_id === right?.session_id && left.host_id === right?.host_id;
  }

  /**
   * 已删除会话的侧栏行隐藏表（#2614）。
   *
   * 删除后只移除当前 DOM 行不够：通知 Codex 重读列表依赖内部模块，新版 Codex 里
   * 已找不到，Codex 内存中的列表仍含这条会话，进设置页再返回时 React 重建侧栏，
   * 它又会出现，直到重启。这里用一张样式表按原始 thread id 隐藏，React 重建出
   * 的同 id 行也一并隐藏；不走扫描调度，不会产生自喂扫描。撤销时移除对应规则。
   */
  const deletedSessionRowStyleId = "codex-plus-deleted-session-rows";
  const deletedSessionRowIds = window.__codexPlusDeletedSessionRowIds instanceof Set
    ? window.__codexPlusDeletedSessionRowIds
    : (window.__codexPlusDeletedSessionRowIds = new Set());
  // 扫描时由 JS 判定为已删会话的行打这个标记，样式表里统一隐藏（见 filterDeletedSessionRows）。
  const deletedSessionRowMarker = "data-codex-plus-deleted-session-row";

  /** 取 id 末尾的 thread uuid（兼容 `local:<uuid>`、`<host>:<uuid>` 等写法）；没有则返回空串。 */
  function deletedSessionUuid(id) {
    const match = String(id || "").trim().match(/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/i);
    return match ? match[1].toLowerCase() : "";
  }

  function renderDeletedSessionRowStyle() {
    let style = document.getElementById(deletedSessionRowStyleId);
    if (!deletedSessionRowIds.size) {
      style?.remove();
      return;
    }
    if (!style) {
      style = document.createElement("style");
      style.id = deletedSessionRowStyleId;
      style.setAttribute("data-codex-plus-ext", "deleted-session-rows");
      document.head.appendChild(style);
    }
    // Codex 的行属性是带前缀的编码值（`local:<thread>`），React 重建时同一会话
    // 可能换成另一种写法（裸 id / 带前缀 / 带 host 作用域）。只按删除时读到的原值
    // 精确匹配会漏掉这些变体，行就会重新出现（#2671）。thread uuid 全局唯一，
    // 用后缀匹配覆盖全部写法；拿不到 uuid 时才退回精确匹配原值。
    const rules = new Set();
    for (const id of deletedSessionRowIds) {
      const uuid = normalizedCodexThreadUuid(id);
      if (uuid) rules.add(`[data-app-action-sidebar-thread-id$="${CSS.escape(uuid)}"]{display:none !important}`);
      else rules.add(`[data-app-action-sidebar-thread-id="${CSS.escape(id)}"]{display:none !important}`);
    }
    // 属性里看不出 uuid 的重建行（临时 client-new-thread id、只在 href / React props
    // 里带 uuid 等）由扫描打标记后隐藏（#2671）。
    rules.add(`[${deletedSessionRowMarker}="true"]{display:none !important}`);
    style.textContent = Array.from(rules).join("\n");
  }

  /** 本次页面生命周期内已成功删除的会话 uuid 集合。 */
  function deletedSessionUuids() {
    const uuids = new Set();
    for (const id of deletedSessionRowIds) {
      const uuid = deletedSessionUuid(id);
      if (uuid) uuids.add(uuid);
    }
    return uuids;
  }

  /**
   * 侧栏重建时过滤已删会话（#2671）。
   *
   * 根因：本地删除走 Codex++ 后端直接删库/删文件，Codex 自己的会话列表缓存并不知道；
   * 以前靠 refresh-recent-conversations-for-host 让 Codex 重读列表，新版 Codex 已没有
   * 这个请求，于是折叠再展开项目分组时，Codex 按缓存把这条会话重新渲染出来。
   * 样式表只能按行属性匹配，这里再用与删除按钮同一套 sessionRefFromRow 解析行身份
   * （属性 / href / React props），凡是解析到已删 uuid 的行都打标记隐藏。
   * 只在标记状态变化时写属性，React 复用节点给别的会话时会把标记摘掉；
   * 标记属性不在扫描 observer 的 attributeFilter 里，不会引发自喂扫描。
   */
  function filterDeletedSessionRows() {
    const uuids = deletedSessionUuids();
    if (!uuids.size && !document.querySelector(`[${deletedSessionRowMarker}]`)) return;
    for (const row of sessionRows()) {
      const hidden = uuids.size > 0 && uuids.has(deletedSessionUuid(sessionRefFromRow(row).session_id));
      if (hidden && row.getAttribute(deletedSessionRowMarker) !== "true") row.setAttribute(deletedSessionRowMarker, "true");
      else if (!hidden && row.getAttribute(deletedSessionRowMarker) !== null) row.removeAttribute(deletedSessionRowMarker);
    }
  }

  /**
   * 记录一条已删除会话并刷新隐藏规则，返回实际新增的 id（供撤销时移除）。
   * rawThreadId 是删除时行上的原始属性值（可能为空：行在请求期间被 React 换掉）；
   * sessionId 是后端确认删除的规范 uuid，行不在了也照样记录。
   */
  function hideDeletedSessionRows(rawThreadId, sessionId = "") {
    const added = [];
    for (const id of [rawThreadId, sessionId]) {
      if (!id || isClientNewThreadId(id) || deletedSessionRowIds.has(id)) continue;
      deletedSessionRowIds.add(id);
      added.push(id);
    }
    if (added.length) {
      renderDeletedSessionRowStyle();
      filterDeletedSessionRows();
    }
    return added;
  }

  function unhideDeletedSessionRows(rawThreadIds) {
    let changed = false;
    for (const id of rawThreadIds || []) changed = deletedSessionRowIds.delete(id) || changed;
    if (changed) {
      renderDeletedSessionRowStyle();
      filterDeletedSessionRows();
    }
  }

  // 注入脚本重跑（页面未刷新）时按已记录的 id 恢复样式表；首次加载 Set 为空，不做任何事。
  if (deletedSessionRowIds.size) renderDeletedSessionRowStyle();

  /**
   * 从多个原生「新聊天」按钮里确定地挑一个（#2671）。
   *
   * 新版 Codex 侧栏和导航栏（navigation rail）各有一个新聊天按钮，二者都调用同一个
   * onStartChat，点哪个效果相同；以前候选不唯一就放弃跳转，界面停在已删会话。
   * 优先级：可见优先于不可见；同为可见/不可见时，侧栏列表里的优先于导航栏里的；
   * 再相同则按 DOM 顺序取第一个，保证每次挑中的是同一个。
   */
  function pickNewChatButton(candidates) {
    const rank = (candidate) => (visibleElement(candidate) ? 0 : 2)
      + (candidate.closest("nav[data-app-navigation-rail]") ? 1 : 0);
    let best = null;
    let bestRank = Infinity;
    for (const candidate of candidates) {
      const candidateRank = rank(candidate);
      if (candidateRank < bestRank) {
        best = candidate;
        bestRank = candidateRank;
      }
    }
    return best;
  }

  function removeDeletedRow(row, button, ref, requestContext) {
    // 删除响应期间，React 可能复用或替换 sidebar 行；不能删除其新身份的 DOM。
    if (!row.isConnected || !sameDeletedSessionRef(sessionRefFromRow(row), ref)) return null;
    releaseDeleteFocus(row, button);
    const activeRefs = sessionRows().map((candidate) => ({ row: candidate, ref: sessionRefFromRow(candidate) }))
      .filter((candidate) => isCurrentSessionRow(candidate.row, candidate.ref)).map((candidate) => candidate.ref);
    const shouldLeave = requestContext?.wasCurrent && requestContext.locationHref === window.location.href
      && activeRefs.length > 0 && activeRefs.every((active) => sameDeletedSessionRef(active, ref));
    let navigated = false;
    if (shouldLeave) {
      // 已审 native 的 New chat 按钮以 aria-label/newChatMessage 调用 onStartChat。
      // 只点原生侧栏/导航栏控件；不猜内部路由，也不强制 reload。
      const newChatButtons = Array.from(document.querySelectorAll("aside.app-shell-left-panel button, nav[data-app-navigation-rail] button"))
        .filter((candidate) => !candidate.disabled && !isExtensionUiNode(candidate)
          && candidate.closest("aside.app-shell-left-panel, nav[data-app-navigation-rail]")
          && /^(新聊天|新对话|New chat|New thread)$/i.test((candidate.getAttribute("aria-label") || candidate.textContent || "").trim()));
      // 新版 Codex 侧栏和导航栏各有一个新聊天按钮，候选不唯一时按固定优先级挑一个；
      // 侧栏折叠或被隐藏时按钮不可见，但仍在 DOM 里，React 的 onClick 照样能触发（#2671）。
      const visibleCount = newChatButtons.filter(visibleElement).length;
      const target = pickNewChatButton(newChatButtons);
      if (target) {
        target.click();
        navigated = true;
      }
      // 只记计数和挑选结果，不带会话 id / 标题。
      if (!target) {
        sendCodexPlusDiagnostic("delete_session_new_chat_missing", { candidateCount: 0, visibleCount: 0 });
      } else if (newChatButtons.length > 1) {
        sendCodexPlusDiagnostic("delete_session_new_chat_multiple", {
          candidateCount: newChatButtons.length,
          visibleCount,
          chosenVisible: visibleElement(target),
          chosenInNavigationRail: !!target.closest("nav[data-app-navigation-rail]"),
        });
      } else if (!visibleElement(target)) {
        sendCodexPlusDiagnostic("delete_session_new_chat_fallback", {
          candidateCount: newChatButtons.length,
          visibleCount,
        });
      }
    }
    // 原生导航也可能同步重建行，移除前再确认一次，保护被复用的节点。
    if (row.isConnected && sameDeletedSessionRef(sessionRefFromRow(row), ref)) row.remove();
    return shouldLeave && !navigated ? "会话已删除，请点击新聊天继续" : null;
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

  /**
   * 快速删除（#48）：设置「删除会话时不再确认」打开后，本机会话跳过确认弹窗。
   * 本机删除有备份并可在提示里撤销；远端 / SSH 会话是永久删除、无法撤销，始终要确认。
   */
  function sessionDeleteSkipsConfirm(hostId) {
    if (hostId !== "local") return false;
    try {
      return codexPlusSettings().sessionDeleteSkipConfirm === true;
    } catch {
      return false;
    }
  }

  function openDeleteConfirmForRow(row, button, ref, event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    releaseDeleteFocus(row, button);
    if (!ref.host_id) {
      const evidence = sessionHostEvidenceFromRow(row, ref.session_id);
      sendCodexPlusDiagnostic("delete_session_host_unresolved", {
        reason: evidence.reason, hostCount: evidence.hostCount, matchingMetadata: evidence.matchingMetadata,
        nativeHostAttributePresent: !!row.getAttribute("data-app-action-sidebar-thread-host-id"),
      });
      showToast("无法确定会话主机归属，请使用 Codex 原生会话管理", null);
      return;
    }
    ref = { session_id: ref.session_id, title: ref.title, host_id: ref.host_id };
    const confirmation = sessionDeleteSkipsConfirm(ref.host_id) ? Promise.resolve(true) : confirmDelete(ref.title, ref.host_id);
    return confirmation.then(async (confirmed) => {
      if (!confirmed) return;
      if (!row.isConnected || !sameDeletedSessionRef(sessionRefFromRow(row), ref)) {
        showToast("会话已变化，请重新选择后删除", null);
        return;
      }
      releaseDeleteFocus(row, button);
      const requestContext = { wasCurrent: isCurrentSessionRow(row, ref), locationHref: window.location.href };
      const result = await postJson("/delete", ref);
      if (result.status === "server_deleted" || result.status === "local_deleted") {
        if (result.session_id && result.session_id !== ref.session_id) {
          showToast("删除结果与请求会话不一致，未更新界面", null);
          return;
        }
        // 远端由原生同主机 thread/deleted 通知更新，不能移除可能已重用的本地 DOM。
        let navigationNotice = null;
        let hiddenRowIds = [];
        if (ref.host_id === "local") {
          // 在移除前读原始 id：行可能带 host 前缀，与规范化后的 session_id 不同。
          const rawThreadId = row.isConnected && sameDeletedSessionRef(sessionRefFromRow(row), ref)
            ? row.getAttribute("data-app-action-sidebar-thread-id") || ""
            : "";
          navigationNotice = removeDeletedRow(row, button, ref, requestContext);
          // 后端已确认删除 ref.session_id：即便行在请求期间被 React 换掉、读不到原始 id，
          // 也按规范 uuid 记下，侧栏重建出的同一会话照样隐藏（#2671）。
          hiddenRowIds = hideDeletedSessionRows(rawThreadId, ref.session_id);
          const refreshed = await refreshRecentConversationsForHost();
          // 新版 Codex 已没有重读列表的请求，缓存里仍留着这条会话，只能靠上面的隐藏兜底。
          if (!refreshed) sendCodexPlusDiagnostic("delete_session_list_refresh_unavailable", {});
        }
        showToast(navigationNotice || result.message || "删除成功", {
          undoToken: result.undo_token,
          onUndone: () => unhideDeletedSessionRows(hiddenRowIds),
        });
      } else {
        showToast(result.message || "删除失败", null);
      }
    });
  }

  async function exportMarkdown(ref) {
    const result = await postJson("/export-markdown", ref);
    if (result.status === "exported" && result.filename && typeof result.markdown === "string") {
      const saveResult = await saveMarkdown(result.filename, result.markdown);
      if (saveResult?.status === "cancelled") {
        showToast(saveResult.message || "导出已取消", null);
      } else {
        showToast(result.message || "导出成功", null);
      }
      return;
    }
    showToast(result.message || "导出失败", null);
  }

  function installDeleteButtonEventDelegation() {
    document.removeEventListener("click", window.__codexSessionDeleteDocumentDeleteHandler, true);
    const handler = (event) => {
      const button = event.target?.closest?.(`.${buttonClass}`);
      const row = button?.closest?.("[data-app-action-sidebar-thread-id]");
      if (!button || !row) return;
      const ref = sessionRefFromRow(row);
      if (!ref.session_id) {
        const placeholderId = row.getAttribute("data-app-action-sidebar-thread-id");
        if (isClientNewThreadId(placeholderId)) {
          event.preventDefault();
          event.stopPropagation();
          event.stopImmediatePropagation?.();
          showToast("会话仍在同步，请稍后重试", null);
        }
        return;
      }
      openDeleteConfirmForRow(row, button, ref, event);
    };
    window.__codexSessionDeleteDocumentDeleteHandler = handler;
    document.addEventListener("click", handler, true);
  }

  function actionGroupFromRow(row) {
    return row.querySelector(`.${actionGroupClass}`);
  }

  function nativeActionButtonsFromRow(row) {
    return [...row.querySelectorAll('button,[role="button"],a')]
      .filter((node) => !node.closest(`.${actionGroupClass}`))
      .filter((node) => {
        const rect = node.getBoundingClientRect();
        if (rect.width < 12 || rect.height < 12) return false;
        const label = [
          node.getAttribute("aria-label"),
          node.getAttribute("title"),
          node.dataset?.state,
          node.textContent,
        ]
          .filter(Boolean)
          .join(" ")
          .toLowerCase();
        if (/(pin|archive|置顶|归档)/i.test(label)) return true;
        const rowRect = row.getBoundingClientRect();
        return rect.left > rowRect.left + rowRect.width * 0.68;
      });
  }

  function syncActionGroupLayout(row, group) {
    syncActionGroupLayouts([{ row, group }]);
  }

  function syncActionGroupLayouts(entries, force = false) {
    // Batch each measurement phase across rows. A per-row write/read cycle
    // repeatedly lays out the app and delays the first resized background frame.
    const layouts = entries.filter(({ row, group }) =>
      row && group && (force || group.dataset.codexActionLayoutStable !== "true"))
      .map(({ row, group }) => {
        const rowRect = row.getBoundingClientRect();
        const nativeButtons = nativeActionButtonsFromRow(row);
        const native = nativeButtons.map(button => ({ button, rect: button.getBoundingClientRect() }))
          .filter(({ rect }) => rect.width > 0 && rect.height > 0)
          .sort((a, b) => a.rect.left - b.rect.left)[0];
        const right = native ? Math.max(28, Math.round(rowRect.right - native.rect.left + 8)) : 28;
        const titleLeft = row.querySelector(selectors.threadTitle)?.getBoundingClientRect().left || rowRect.left + 40;
        const renderScale = row.offsetWidth > 0 ? rowRect.width / row.offsetWidth : 1;
        return { row, group, native, right, effectiveRight: right, titleLeft, renderScale,
          groupWidth: Math.ceil(group.getBoundingClientRect().width || 96) };
      });
    for (const layout of layouts) {
      // 颜色与悬停态不再从原生按钮的计算样式抄：那样拿到的是静止态，悬停时
      // 既不提亮也不变底色。外观改由按钮直接套用原生类名（syncNativeSessionActionStyle）。
      layout.group.style.setProperty("--codex-session-actions-right", `${layout.right}px`);
    }
    for (const layout of layouts) {
      if (!layout.native) continue;
      const groupRight = layout.group.getBoundingClientRect().right;
      const targetRight = layout.native.rect.left - 2;
      if (Number.isFinite(groupRight) && Number.isFinite(targetRight)) {
        layout.effectiveRight = Math.max(0, layout.right +
          (groupRight - targetRight) / Math.max(0.1, layout.renderScale));
      }
    }
    for (const { group, native, effectiveRight } of layouts) {
      if (native) group.style.setProperty("--codex-session-actions-right", `${effectiveRight}px`);
    }
    for (const layout of layouts) {
      const finalGroupLeft = layout.group.getBoundingClientRect().left;
      layout.titleMaxWidth = Math.max(24,
        (finalGroupLeft - layout.titleLeft - 8) / Math.max(0.1, layout.renderScale));
    }
    for (const { row, group, effectiveRight, groupWidth, titleMaxWidth } of layouts) {
      row.style.setProperty("--codex-session-title-mask", `${effectiveRight + groupWidth + 12}px`);
      row.style.setProperty("--codex-session-title-max-width", `${titleMaxWidth}px`);
      group.dataset.codexActionLayoutStable = "true";
    }
  }
