  const dailyUiBookmarkPrefix = "bookmark:";
  const dailyUiSections = {
    projects: ["Projects", "项目", "專案"],
    recents: ["Recents", "Chats", "最近", "最近使用", "聊天"],
  };

  function dailyUiElement(tag, className, text) {
    const element = document.createElement(tag);
    element.setAttribute("data-codex-plus-ext", "builtin:daily-ui");
    if (className) element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  }

  function dailyUiRow(container, title, detail, ...controls) {
    const row = dailyUiElement("div", "codex-plus-row");
    const copy = dailyUiElement("div");
    copy.appendChild(dailyUiElement("div", "codex-plus-row-title", title));
    if (detail) copy.appendChild(dailyUiElement("div", "codex-plus-row-description", detail));
    row.append(copy, ...controls);
    container.appendChild(row);
  }

  function dailyUiButton(label, activate) {
    const button = dailyUiElement("button", "codex-plus-menu-trigger", label);
    button.type = "button";
    button.addEventListener("click", activate);
    return button;
  }

  function dailyUiSwitch(label, initial, change) {
    const button = dailyUiElement("button", "codex-plus-toggle");
    button.type = "button";
    button.setAttribute("role", "switch");
    button.setAttribute("aria-label", label);
    button.appendChild(dailyUiElement("span"));
    const set = (enabled) => {
      button.dataset.enabled = String(enabled);
      button.setAttribute("aria-checked", String(enabled));
    };
    set(initial);
    button.addEventListener("click", () => {
      const enabled = button.dataset.enabled !== "true";
      try { change(enabled); set(enabled); }
      catch { showToast("保存设置失败，请重试", { type: "error" }); }
    });
    return button;
  }

  function dailyUiApplyPlan(root) {
    if (!root || root.dataset.codexPlusDailyDraftSeen === "true") return;
    const editor = root.querySelector('[data-codex-composer][contenteditable="true"]');
    const controls = root.querySelector('[data-composer-leading-controls]');
    if (!editor || !controls || root.closest('[data-app-shell-active-page="false"]') || !root.getClientRects().length) return;
    root.dataset.codexPlusDailyDraftSeen = "true";
    const locale = String(document.documentElement.lang || navigator.language || "");
    if (!/^(?:en|zh)(?:-|$)/i.test(locale)) return;
    const activePlan = [...controls.querySelectorAll("button[aria-label]")].some((button) =>
      /^(?:Plan|Plan mode|计划|计划模式|规划|方案|計劃|計畫|規劃)$/i.test(button.getAttribute("aria-label").trim()));
    if (activePlan || editor.textContent.trim() || root.querySelector('[data-above-composer-conversation-id]')) return;
    window.dispatchEvent(new MessageEvent("message", { data: { type: "run-command", id: "composer.togglePlanMode" } }));
  }

  function dailyUiSidebarSelector(id) {
    if (id.startsWith("section:")) {
      const headings = dailyUiSections[id.slice(8)];
      return Array.isArray(headings) ? headings.map((heading) => `[data-app-action-sidebar-scroll] [data-app-action-sidebar-section-heading="${heading}"]`).join(",") : "";
    }
    if (!/^builtin:[a-z][a-z0-9-]{0,63}$/.test(id) || /:(?:home|new-chat|settings|search)$/.test(id)) return "";
    return `:is([data-app-action-sidebar-scroll],nav[data-app-navigation-rail]) [data-sidebar-destination="${id}"]:not([data-codex-plus-sidebar-nav] *)`;
  }

  function dailyUiSidebarChoices() {
    const choices = new Map();
    for (const node of document.querySelectorAll(':is([data-app-action-sidebar-scroll],nav[data-app-navigation-rail]) [data-sidebar-destination^="builtin:"]')) {
      const id = node.dataset.sidebarDestination;
      const label = (node.getAttribute("aria-label") || node.textContent || "").trim();
      if (!node.closest('[data-codex-plus-sidebar-nav]') && dailyUiSidebarSelector(id) && label) choices.set(id, label);
    }
    choices.set("section:projects", "项目列表");
    choices.set("section:recents", "最近会话");
    return choices;
  }

  function dailyUiProjectPath(raw) {
    const value = String(raw ?? "").trim();
    if (!value) return null;
    const valid = isWindowsPlatformForDailyUi() ? /^[A-Za-z]:[\\/]/.test(value) || /^\\\\[^\\]+\\[^\\]+/.test(value) : value.startsWith("/");
    if (!valid || /[\x00-\x1f]/.test(value)) throw new Error("请输入本机目录的绝对路径");
    return value;
  }

  function isWindowsPlatformForDailyUi() {
    return /win/i.test(navigator.userAgentData?.platform || navigator.platform || "");
  }

  async function dailyUiProjectControl(container) {
    const input = dailyUiElement("input", "codex-plus-width-input codex-plus-daily-project-path");
    input.type = "text";
    input.placeholder = "桌面默认目录";
    input.setAttribute("aria-label", "新建项目默认目录");
    const status = dailyUiElement("div", "codex-plus-row-description");
    status.setAttribute("role", "status");
    let previous;
    const settingKey = "defaultProjectParent";
    let busy = true;
    const update = () => { input.disabled = busy; save.disabled = busy || input.value.trim() === (previous || ""); };
    const save = dailyUiButton("保存目录", async () => {
      if (busy) return;
      busy = true;
      update();
      try {
        const value = dailyUiProjectPath(input.value);
        if ((await readCodexAppSetting(settingKey) ?? null) !== previous) throw new Error("目录已被更改，请重新打开设置");
        await writeCodexAppSetting(settingKey, value);
        if ((await readCodexAppSetting(settingKey) ?? null) !== value) throw new Error("桌面返回的目录与所选值不同");
        previous = value;
        input.value = value || "";
        status.textContent = "已保存，新建本地项目会使用此目录。";
      } catch (error) { status.textContent = error?.message || "保存目录失败"; }
      finally { busy = false; update(); }
    });
    input.addEventListener("input", update);
    dailyUiRow(container, "新建项目默认目录", "用于从头创建本地项目。清空并保存可恢复桌面默认目录。", input, save);
    container.appendChild(status);
    update();
    try {
      previous = await readCodexAppSetting(settingKey) ?? null;
      input.value = previous || "";
      busy = false;
      update();
    } catch { status.textContent = "读取本地桌面设置失败，请重新打开此页。"; }
  }

  function dailyUiBookmark(message) {
    const scroller = message?.element?.closest(".thread-scroll-container");
    if (!scroller || scroller.closest('[data-app-shell-active-page="false"]')) return null;
    const ref = currentSessionRef();
    const thread = normalizedCodexThreadUuid(message.conversationId || dailyUiMountedThread(scroller));
    const anchor = message.id;
    if (!thread || typeof anchor !== "string" || !anchor || anchor.length > 512 || !["assistant", "user"].includes(message.role)) return null;
    const host = ref.session_id === thread ? ref.host_id || null : null;
    const turn = normalizedCodexThreadUuid(message.turnId);
    const offset = Math.abs(scroller.scrollTop);
    const position = offset / Math.max(1, scroller.scrollHeight - scroller.clientHeight);
    return { thread, host, turn, anchor, role: message.role, title: Array.from(String(message.text || "").trim()).slice(0, 96).join(""), offset, position: Math.min(1, position), date: Date.now() };
  }

  function dailyUiMountedThread(scroller) {
    return normalizedCodexThreadUuid(scroller?.querySelector('[data-above-composer-conversation-id]')?.getAttribute('data-above-composer-conversation-id')
      || scroller?.querySelector('[data-response-annotation-conversation]')?.getAttribute('data-response-annotation-conversation'));
  }

  function dailyUiNavigateToMark(mark) {
    if (dailyUiMountedThread(currentThreadScroller()) === mark.thread) return true;
    if (mark.host === "local") {
      window.dispatchEvent(new MessageEvent("message", { data: { type: "navigate-to-route", path: `/local/${mark.thread}` } }));
      return true;
    }
    for (const row of sessionRows()) {
      const ref = sessionRefFromRow(row);
      if (ref.session_id === mark.thread && (!mark.host || ref.host_id === mark.host)) { row.click(); return true; }
    }
    showToast("请从原主机打开此会话，再打开书签以定位消息", { type: "warn" });
    return false;
  }

  function dailyUiBookmarkKey(mark) {
    return `${dailyUiBookmarkPrefix}${mark.thread}:${mark.role}:${encodeURIComponent(mark.anchor)}`;
  }

  function dailyUiPageDirection(scroller, turn) {
    if (!turn) return 0;
    const railItems = scroller.closest("main")?.querySelectorAll('button[data-thread-user-message-navigation-item-id]');
    if (!railItems?.length) return 0;
    const order = new Map(Array.from(railItems, (item, index) => [item.dataset.threadUserMessageNavigationItemId.split(":")[0], index]));
    const wanted = order.get(turn);
    if (wanted === undefined) return 0;
    const mounted = Array.from(scroller.querySelectorAll('[data-content-search-unit-key]'), (item) => order.get(item.dataset.contentSearchUnitKey.split(":")[0])).filter(Number.isFinite);
    if (!mounted.length) return 0;
    if (wanted < Math.min(...mounted)) return -1;
    if (wanted > Math.max(...mounted)) return 1;
    return 0;
  }

  function dailyUiScrollToMark(mark, done) {
    const deadline = Date.now() + 20000;
    let attempts = 0;
    let lastPositionAt = 0;
    let usedNavigationRail = false;
    let timer = 0;
    const cancel = () => { clearTimeout(timer); done(); };
    const locate = () => {
      const scroller = currentThreadScroller();
      if (scroller && dailyUiMountedThread(scroller) === mark.thread) {
        cancelThreadScrollRestoreForUserIntent(mark.thread);
        const attribute = mark.role === "assistant" ? "data-response-annotation-target" : "data-content-search-unit-key";
        const target = scroller.querySelector(`[${attribute}="${CSS.escape(mark.anchor)}"]`);
        if (target) { target.scrollIntoView({ block: "center", behavior: "instant" }); cancel(); return; }
        if (!usedNavigationRail && normalizedCodexThreadUuid(mark.turn)) {
          const rail = scroller.closest("main")?.querySelector(`button[data-thread-user-message-navigation-item-id^="${CSS.escape(mark.turn)}:"]`);
          if (rail) { usedNavigationRail = true; lastPositionAt = Date.now() + 800; rail.click(); }
        }
        if (Date.now() - lastPositionAt > 320 && scroller.scrollHeight > scroller.clientHeight) {
          const distance = scroller.scrollHeight - scroller.clientHeight;
          const direction = dailyUiPageDirection(scroller, mark.turn);
          const reversed = threadScrollIsReversed(scroller);
          const fraction = Math.min(1, Math.max(0, Number(mark.position) || 0));
          const stride = Math.ceil(Math.max(0, attempts - 1) / 2) * scroller.clientHeight * .85;
          const position = attempts === 1 && Number.isFinite(mark.offset) ? mark.offset
            : fraction * distance + (attempts % 2 === 0 ? stride : -stride);
          if (direction < 0) scroller.scrollTop = reversed ? -distance : 0;
          else if (direction > 0) scroller.scrollTop = reversed ? 0 : distance;
          else scroller.scrollTop = (reversed ? -1 : 1) * Math.min(distance, Math.max(0, position));
          attempts += 1;
          lastPositionAt = Date.now();
        }
      }
      if (Date.now() >= deadline) { showToast("未找到书签对应的消息，请确认原会话仍可用", { type: "warn" }); cancel(); return; }
      timer = setTimeout(locate, 90);
    };
    timer = setTimeout(locate, 50);
    return cancel;
  }

  function codexPlusDailyUiExtension(ctx) {
    let hidden = ctx.storage.get("hiddenSidebar", {});
    const sidebarStyle = dailyUiElement("style");
    function syncSidebar() {
      hidden = ctx.storage.get("hiddenSidebar", {});
      const selectors = Object.keys(hidden || {}).map(dailyUiSidebarSelector).filter(Boolean);
      const content = selectors.length ? `${selectors.join(",")} { display: none !important; }` : "";
      if (sidebarStyle.textContent !== content) sidebarStyle.textContent = content;
      if (content && !sidebarStyle.isConnected) document.head.appendChild(sidebarStyle);
      if (!content && sidebarStyle.isConnected) sidebarStyle.remove();
    }
    const plan = () => {
      if (ctx.storage.get("defaultPlan", false) === true) {
        for (const root of document.querySelectorAll('main [data-codex-composer-root][data-composer-placement="home"]')) dailyUiApplyPlan(root);
      }
    };
    let pendingJump;
    let unregisterAction;
    function syncActions() {
      if (ctx.storage.get("bookmarksEnabled", false) !== true) { unregisterAction?.(); unregisterAction = null; return; }
      if (unregisterAction) return;
      unregisterAction = ctx.registerMessageAction({ id: "toggle-bookmark", label: "添加或移除消息书签",
        icon: '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"><path d="M4 3.4c0-.8.5-1.2 1.2-1.2h5.6c.7 0 1.2.4 1.2 1.2v10.3L8 11l-4 2.7z"/></svg>', roles: ["user","assistant"],
        onActivate({ message }) {
          const mark = dailyUiBookmark(message);
          if (!mark) { showToast("这条消息尚无可定位的会话或消息编号", { type: "warn" }); return; }
          const key = dailyUiBookmarkKey(mark);
          try {
            const exists = ctx.storage.get(key) !== null;
            if (exists) ctx.storage.remove(key); else ctx.storage.set(key, mark);
            showToast(exists ? "已移除书签" : "已添加消息书签");
          } catch { showToast("无法保存书签", { type: "error" }); }
        },
      });
    }
    function bookmarks(container) {
      container.replaceChildren();
      const list = ctx.storage.keys().filter((key) => key.startsWith(dailyUiBookmarkPrefix)).map((key) => [key, ctx.storage.get(key)])
        .filter(([,mark]) => mark && normalizedCodexThreadUuid(mark.thread) === mark.thread && typeof mark.anchor === "string" && mark.anchor.length <= 512)
        .sort((left,right) => right[1].date - left[1].date);
      if (!list.length) container.appendChild(dailyUiElement("p", "codex-plus-row-description", "在消息操作栏点击书签即可收藏。"));
      for (const [key,mark] of list) {
        dailyUiRow(container, mark.title || (mark.role === "user" ? "用户消息" : "助手消息"), `会话 ${mark.thread.slice(0,8)}`,
          dailyUiButton("打开", () => {
            pendingJump?.();
            closeCodexPlusPage();
            if (dailyUiNavigateToMark(mark)) pendingJump = dailyUiScrollToMark(mark, () => { pendingJump = null; });
          }), dailyUiButton("移除", () => { ctx.storage.remove(key); bookmarks(container); }));
      }
    }
    ctx.registerSettingsSection({ id: "daily-ui", title: "界面与日常操作", order: 45, render({ container }) {
      container.replaceChildren();
      dailyUiRow(container, "新任务默认 Plan 模式", "在空白的新任务中开启计划模式，可在输入框里随时切换。",
        dailyUiSwitch("新任务默认 Plan 模式", ctx.storage.get("defaultPlan", false) === true, (enabled) => { ctx.storage.set("defaultPlan", enabled); plan(); }));
      void dailyUiProjectControl(container);
      const nav = dailyUiElement("section");
      container.appendChild(nav);
      const renderSidebar = () => {
        nav.replaceChildren(dailyUiElement("h3", "codex-plus-settings-section-title", "侧栏显示项目"));
        const choices = dailyUiSidebarChoices();
        for (const [id,label] of Object.entries(hidden || {})) if (dailyUiSidebarSelector(id) && !choices.has(id)) choices.set(id, String(label));
        for (const [id,label] of choices) dailyUiRow(nav, label, "", dailyUiSwitch(label, !Object.hasOwn(hidden || {}, id), (visible) => {
          const next = { ...ctx.storage.get("hiddenSidebar", {}) };
          if (visible) delete next[id]; else next[id] = label;
          ctx.storage.set("hiddenSidebar", next);
          syncSidebar();
        }));
        nav.appendChild(dailyUiButton("恢复全部侧栏项目", () => { ctx.storage.remove("hiddenSidebar"); syncSidebar(); renderSidebar(); }));
      };
      renderSidebar();
      const saved = dailyUiElement("div");
      dailyUiRow(container, "消息书签", "将消息位置和简短摘录保存在本机。", dailyUiSwitch("消息书签", ctx.storage.get("bookmarksEnabled",false) === true,
        (enabled) => { ctx.storage.set("bookmarksEnabled", enabled); syncActions(); bookmarks(saved); }));
      container.appendChild(saved);
      bookmarks(saved);
    } });
    ctx.onScan(plan, { phase: "light" });
    ctx.listen(window, "storage", () => { syncSidebar(); syncActions(); plan(); });
    ctx.listen(window, "wheel", () => pendingJump?.(), { capture: true, passive: true });
    ctx.listen(window, "touchstart", () => pendingJump?.(), { capture: true, passive: true });
    ctx.listen(window, "keydown", () => pendingJump?.(), { capture: true });
    ctx.effect(() => { syncSidebar(); syncActions(); plan(); return () => { sidebarStyle.remove(); pendingJump?.(); }; });
  }
