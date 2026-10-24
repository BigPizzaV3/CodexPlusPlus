  // 只保存结构化缓存，不读取虚拟列表里的 DOM 文本。数据跨热重注入保留，DOM 每次现取。
  const sideChatSaveState = window.__codexPlusSideChatSaveState
    || (window.__codexPlusSideChatSaveState = { entries: new Map(), manager: null, resolving: null, scanning: false });
  const sideChatSaveSelector = '[data-codex-plus-ext="side-chat-save"]';
  registerCodexPlusExtensionSelector(sideChatSaveSelector);
  document.querySelectorAll(sideChatSaveSelector).forEach(button => button.remove());

  function sideChatSaveEnabled() {
    return codexPlusSettings().sideChatSave === true;
  }

  async function sideChatSource(sourceId, boundary = {}) {
    const result = await postJson("/side-chat/source", { host_id: "local", source_id: sourceId, ...boundary });
    if (result?.status !== "ok" || (!result.source_ref && typeof result.content !== "string")) {
      throw Error(result?.message || "无法读取主会话历史");
    }
    return result;
  }

  // 两种原生客户端补丁共用此入口，避免再包装一层 sendRequest 原型。
  async function sendCodexSideChatNativeRequest(client, method, params, send) {
    if (client.hostId !== "local") return send();
    if (method === "thread/stop" || method === "thread/unsubscribe") {
      const result = await send();
      sideChatSaveState.entries.delete(params?.threadId);
      return result;
    }
    if (!sideChatSaveEnabled() || method !== "thread/fork" || params?.ephemeral !== true) return send();
    let source, error;
    try {
      source = await sideChatSource(params.threadId, {
        last_turn_id: params.lastTurnId, before_turn_id: params.beforeTurnId,
      });
    } catch (reason) {
      // 采集失败不阻止原生旁支创建；保存时明确报错，不用最新主历史冒充原副本。
      error = reason.message;
    }
    const result = await send();
    const id = result?.thread?.id;
    if (id) sideChatSaveState.entries.set(id, { source, error });
    return result;
  }

  function sideChatTurns(conversation) {
    const history = conversation.turnHistory;
    if (history?.kind === "canonical" && history.history.isComplete !== true) {
      throw Error("旁支历史尚未完整加载，暂不能保存");
    }
    if (conversation.turnsPagination?.hasLoadedOldest === false) {
      throw Error("旁支历史尚未完整加载，暂不能保存");
    }
    const canonical = history?.kind === "canonical" ? history.history : null;
    const cached = canonical
      ? canonical.islands.flatMap(island => island.entries.map(entry => canonical.entitiesByKey[entry.value]).filter(Boolean))
      : [];
    return [...new Map([...cached, ...(conversation.turns || [])]
      .filter(turn => turn.turnId).map(turn => [turn.turnId, turn])).values()];
  }

  function sideChatSaveReady(conversation) {
    if (conversation?.ephemeral !== true || conversation.sideConversation !== true || conversation.hostId !== "local") {
      throw Error("目前只支持保存本地临时旁支");
    }
    const turns = sideChatTurns(conversation);
    if (!turns.length) throw Error("旁支还没有可保存的对话");
    if (conversation.requests?.length || turns.some(turn => turn.status === "inProgress"
      || turn.items.some(item => item.status === "inProgress"))) {
      throw Error("请等待旁支本轮回复和工具执行结束后再保存");
    }
    return turns.map(({ turnId, status, error, items }) => ({ turnId, status, error, items }));
  }

  async function sideChatManager() {
    const cached = sideChatSaveState.manager?.deref();
    if (cached && cached.requestClient?.disposed !== true) return cached;
    if (sideChatSaveState.resolving) return sideChatSaveState.resolving;
    sideChatSaveState.resolving = (async () => {
      const module = await loadOptionalCodexAppModule("app-shared-");
      const registries = Object.values(module || {}).filter(value =>
        typeof value === "function" && typeof value.prototype?.getForHostId === "function");
      const stack = [window.__codexRoot?._internalRoot?.current], seen = new Set();
      while (stack.length && seen.size < 50000) {
        const fiber = stack.pop();
        if (!fiber || seen.has(fiber)) continue;
        seen.add(fiber);
        if (fiber.sibling) stack.push(fiber.sibling);
        if (fiber.tag === 22 && fiber.memoizedState !== null) continue;
        if (fiber.child) stack.push(fiber.child);
        for (let hook = fiber.memoizedState, count = 0; hook && count++ < 2000; hook = hook.next) {
          const scope = hook.memoizedState?.current;
          if (typeof scope?.get !== "function" || !scope.scope) continue;
          for (const Registry of registries) {
            try {
              const manager = new Registry(scope).getForHostId("local");
              if (typeof manager?.getConversation === "function" && manager.getHostId() === "local") {
                sideChatSaveState.manager = new WeakRef(manager);
                return manager;
              }
            } catch {}
          }
        }
      }
      throw Error("当前 Codex 版本的旁支接口尚未就绪");
    })().finally(() => { sideChatSaveState.resolving = null; });
    return sideChatSaveState.resolving;
  }

  async function sideChatSavedSource(conversation, entry) {
    // 旧版大小限制导致的采集失败，可以按旧旁支的时间校验重新读取。
    if (/^(会话文件超过分享大小限制|主会话历史超过 16 MiB 上限)$/.test(entry.error || "")) delete entry.error;
    if (entry.error) throw Error(entry.error);
    if (entry.source) return entry.source;
    // 安装功能之前已存在的旁支，只在父历史没有后续变化时恢复。
    if (!conversation.forkedFromId || !Number.isFinite(conversation.createdAt)) {
      throw Error("这条旁支没有创建时的主历史副本，请开启功能后新建旁支");
    }
    const source = await sideChatSource(conversation.forkedFromId, { check_timestamps: true });
    const timestamps = source.timestamps || source.content?.trim().split("\n").map(line => JSON.parse(line).timestamp);
    const unchanged = timestamps?.every(timestamp => {
      const at = Date.parse(timestamp);
      return Number.isFinite(at) && at <= conversation.createdAt;
    });
    if (!unchanged) throw Error("这条旧旁支创建后主历史已有变化，无法准确恢复原分叉位置；原旁支未改动");
    entry.source = source;
    return source;
  }

  function verifySavedSideChat(thread, expectedId, turns) {
    if (thread?.id !== expectedId || thread.ephemeral !== false || !thread.path) {
      throw Error("新会话恢复校验失败，原旁支已保留");
    }
    for (const turn of turns) {
      const actual = thread.turns?.find(value => value.id === turn.turnId);
      if (!actual || actual.status !== turn.status
        || turn.items.some(item => !actual.items.some(value => value.id === item.id))) {
        throw Error("新会话历史不完整，原旁支已保留");
      }
    }
  }

  async function openSavedSideChat(id) {
    const module = await loadOptionalCodexAppModule("app-shared-");
    const dispatcher = codexServiceTierDispatcherFromModule(module);
    if (typeof dispatcher?.dispatchHostMessage !== "function") return false;
    await dispatcher.dispatchHostMessage({ type: "navigate-to-route", path: `/local/${id}` });
    return true;
  }

  async function assignSavedSideChat(manager, id, assignment) {
    if (assignment == null) return;
    // 原生 fork 同时登记项目归属；cwd 和显示名称都不能替代项目 ID。
    if (typeof manager.runtimeSettings?.saveThreadProjectAssignment === "function") {
      await manager.runtimeSettings.saveThreadProjectAssignment({ threadId: id, assignment });
    } else if (typeof manager.threadWorkspaceStorage?.setProjectAssignment === "function") {
      await manager.threadWorkspaceStorage.setProjectAssignment(id, assignment);
    } else {
      throw Error("当前 Codex 版本无法登记项目归属，原旁支已保留");
    }
    const actual = await manager.getThreadProjectAssignment(id);
    if (actual?.projectKind !== assignment.projectKind || actual?.projectId !== assignment.projectId) {
      throw Error("新会话的项目归属校验失败，原旁支已保留");
    }
  }

  function sideChatTabs() {
    // 标签头、外层 controller 和内容区都可能有 data-tab-id，只认标签头直属的原生关闭按钮。
    return [...document.querySelectorAll('[data-tab-id^="sidechat:"]')]
      .filter(tab => tab.querySelector(':scope > [data-app-shell-tab-close-button]'));
  }

  function sideChatForkTitle(source, conversations) {
    const title = source?.title?.trim();
    if (!title) return null;
    // 与原生分叉一致：同一分叉链递增编号，标题连同后缀最多 60 字符。
    const threads = new Map([...conversations, source].map(thread => [thread.id, thread]));
    const parse = value => {
      const match = value?.trim().match(/^(.*) \((\d+)\)$/u);
      return match && Number(match[2]) >= 2 ? { base: match[1], number: Number(match[2]) } : null;
    };
    const numbered = (base, number) => {
      const suffix = ` (${number})`, limit = 60 - suffix.length;
      return `${base.length > limit ? `${base.slice(0, limit - 1).trimEnd()}…` : base}${suffix}`;
    };
    const ordinal = (value, base) => {
      const clean = value?.trim() || "", parsed = parse(clean);
      return clean === base ? 1 : parsed && numbered(base, parsed.number) === clean ? parsed.number : null;
    };
    const lineage = id => {
      const result = [], seen = new Set();
      for (let thread = threads.get(id); thread && !seen.has(thread.id); thread = threads.get(thread.forkedFromId)) {
        seen.add(thread.id); result.push(thread);
      }
      return result;
    };
    let base = title, root = source.id;
    const parsed = parse(title);
    if (parsed && source.forkedFromId) {
      const parents = lineage(source.forkedFromId);
      if (!parents.length) base = parsed.base;
      findRoot: for (const parent of parents) {
        for (const candidate of [parent.title?.trim() || "", parse(parent.title)?.base || ""]) {
          if (!candidate || numbered(candidate, parsed.number) !== title) continue;
          base = candidate; root = parent.id;
          for (const ancestor of lineage(parent.forkedFromId)) {
            if (ordinal(ancestor.title, base) == null) break;
            root = ancestor.id;
          }
          break findRoot;
        }
      }
    }
    let highest = 1;
    for (const thread of threads.values()) {
      if (lineage(thread.id).some(ancestor => ancestor.id === root)) {
        highest = Math.max(highest, ordinal(thread.title, base) || 0);
      }
    }
    return numbered(base, highest + 1);
  }

  async function saveSideChat(id) {
    let entry = sideChatSaveState.entries.get(id);
    if (!entry) { entry = {}; sideChatSaveState.entries.set(id, entry); }
    if (entry.busy) return;
    entry.busy = true;
    refreshSideChatSaveButtons();
    try {
      if (!sideChatSaveEnabled()) throw Error("旁支保存功能已关闭");
      const manager = await sideChatManager();
      const conversation = manager.getConversation(id);
      const turns = JSON.parse(JSON.stringify(sideChatSaveReady(conversation)));
      const fingerprint = JSON.stringify(turns);
      const source = await sideChatSavedSource(conversation, entry);
      const assignment = await manager.getThreadProjectAssignment(source.source_id);
      if (entry.fingerprint !== fingerprint) {
        entry.fingerprint = fingerprint;
        entry.saveId = crypto.randomUUID();
        entry.savedAt = new Date().toISOString();
        entry.written = null;
        entry.savedName = sideChatForkTitle(manager.getConversation(source.source_id), manager.getRecentConversations());
      }
      if (!entry.written) {
        const result = await postJson("/side-chat/save", {
          host_id: "local", side_id: id, source_id: source.source_id,
          ...(source.source_ref ? { source_ref: source.source_ref } : { source_content: source.content }),
          save_id: entry.saveId, saved_at: entry.savedAt, turns,
        });
        if (result?.status !== "ok") throw Error(result?.message || "保存失败，原旁支已保留");
        entry.written = result;
      }
      const target = entry.written;
      await manager.sendRequest("thread/resume", { threadId: target.session_id, path: target.path, excludeTurns: true,
        ...(conversation.latestModel ? { model: conversation.latestModel } : {}),
      });
      const read = await manager.sendRequest("thread/read", { threadId: target.session_id, includeTurns: true });
      verifySavedSideChat(read.thread, target.session_id, turns);
      await assignSavedSideChat(manager, target.session_id, assignment);
      const savedName = entry.savedName;
      if (savedName) await manager.setThreadTitle(target.session_id, savedName);
      await refreshRecentConversationsForHost();
      const opened = await openSavedSideChat(target.session_id);
      // 保存期间若用户又发了一条消息，只保留当前副本，不关闭有新增内容的原旁支。
      let unchanged = false;
      try { unchanged = JSON.stringify(sideChatSaveReady(manager.getConversation(id))) === fingerprint; } catch {}
      if (opened && unchanged) {
        const tab = sideChatTabs().find(tab => tab.getAttribute("data-tab-id") === `sidechat:${id}`);
        const close = [...tab?.querySelectorAll('button[aria-label]') || []]
          .find(button => /^(关闭|Close)/i.test(button.getAttribute("aria-label")));
        close?.click();
      }
      showToast(`已保存为普通会话${savedName ? `「${savedName}」` : ""}${opened ? "" : "，可从聊天列表打开"}`);
    } catch (error) {
      showToast(error.message || "保存失败，原旁支已保留");
    } finally {
      entry.busy = false;
      refreshSideChatSaveButtons();
    }
  }

  function refreshSideChatSaveButtons() {
    const tabs = sideChatTabs();
    if (!sideChatSaveEnabled()) {
      document.querySelectorAll(sideChatSaveSelector).forEach(button => button.remove());
      return;
    }
    if (!tabs.length || sideChatSaveState.scanning) return;
    sideChatSaveState.scanning = true;
    void sideChatManager().then(manager => {
      if (!sideChatSaveEnabled()) return;
      for (const tab of sideChatTabs()) {
        const id = tab.getAttribute("data-tab-id").slice(9);
        const conversation = manager.getConversation(id);
        if (conversation?.sideConversation !== true || conversation.hostId !== "local") continue;
        let button = tab.querySelector(sideChatSaveSelector);
        if (!button) {
          button = document.createElement("button");
          button.type = "button";
          button.className = "shrink-0 rounded px-1.5 text-xs text-token-text-secondary hover:text-token-text-primary";
          button.setAttribute("data-codex-plus-ext", "side-chat-save");
          button.setAttribute("aria-label", "保存为普通会话");
          // 原生关闭按钮绝对定位在右侧并使用 z-30，必须留出它的点击区域。
          button.style.cssText = "-webkit-app-region:no-drag;cursor:pointer;min-width:32px;position:relative;z-index:40;margin-inline-end:28px";
          installActionButtonEvents(tab, button, event => {
            event.preventDefault(); event.stopPropagation(); void saveSideChat(id);
          });
          tab.insertBefore(button, tab.lastElementChild);
        }
        const busy = sideChatSaveState.entries.get(id)?.busy === true;
        let reason = busy ? "正在保存…" : "保存为普通会话";
        try { sideChatSaveReady(conversation); button.disabled = busy; } catch (error) { button.disabled = true; reason = error.message; }
        const label = busy ? "保存中" : "保存";
        if (button.textContent !== label) {
          button.textContent = label;
          if (busy) {
            const spinner = document.createElement("span");
            spinner.className = "codex-plus-side-chat-save-spinner";
            spinner.setAttribute("aria-hidden", "true");
            button.appendChild(spinner);
          }
        }
        if (button.getAttribute("aria-busy") !== String(busy)) button.setAttribute("aria-busy", String(busy));
        if (button.getAttribute("aria-label") !== reason) button.setAttribute("aria-label", reason);
        button.removeAttribute("title");
        if (button.dataset.codexActionLabel !== reason) button.dataset.codexActionLabel = reason;
      }
    }).catch(() => {}).finally(() => { sideChatSaveState.scanning = false; });
  }

  if (window.__CODEX_PLUS_TEST_SIDE_CHAT__) {
    window.__codexPlusSideChatTest = { send: sendCodexSideChatNativeRequest, turns: sideChatTurns,
      ready: sideChatSaveReady, source: sideChatSavedSource, verify: verifySavedSideChat, title: sideChatForkTitle,
      save: saveSideChat, refresh: refreshSideChatSaveButtons, tabs: sideChatTabs, state: sideChatSaveState };
  }
