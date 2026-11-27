  // 仅接入消息菜单，不改原生模块、安装目录或对第三方拓展开放写 RPC。
  window.__codexPlusTurnRewindMenuCleanup?.();
  const turnRewindMenuMark = "builtin-turn-rewind-menu";
  let turnRewindMenuDisposed = false;
  let turnRewindObserver = null;
  let turnRewindRefreshTimer = 0;
  let turnRewindSaveServicePromise = null;
  let turnRewindRegistryReader = null;
  let turnRewindRegistryLoad = null;
  let turnRewindRegistryModule = null;
  let turnRewindRegistryRetryAt = 0;
  let turnRewindDesktopServices = null;
  let turnRewindDesktopServicesLoad = null;
  let turnRewindDesktopRetryAt = 0;

  // 兼容仍通过 React scope 提供本地 manager 的旧版桌面。
  function discoverTurnRewindRegistryReader() {
    if (!turnRewindRegistryModule || turnRewindRegistryReader) return;
    const candidates = Object.values(turnRewindRegistryModule).filter((value) => {
      const prototype = typeof value === "function" ? value.prototype : null;
      return prototype && ["getImplForHostId", "getForHostId", "getAll", "addRegistryCallback"]
        .every((name) => typeof prototype[name] === "function");
    });
    if (candidates.length === 1) turnRewindRegistryReader = candidates[0].prototype.getImplForHostId;
  }

  function prepareTurnRewindRegistryReader() {
    // Rolldown 的类导出是 live binding，import 完成时可能尚未初始化。
    // 后续菜单刷新只重读已缓存的 namespace，不重新 fetch 或主动运行模块初始化器。
    discoverTurnRewindRegistryReader();
    if (turnRewindRegistryLoad || Date.now() < turnRewindRegistryRetryAt) return;
    turnRewindRegistryLoad = loadCodexAppModule("app-server-manager-hooks-").then((module) => {
      turnRewindRegistryModule = module;
      discoverTurnRewindRegistryReader();
      scheduleTurnRewindMenuRefresh();
    }).catch(() => {
      turnRewindRegistryLoad = null;
      turnRewindRegistryRetryAt = Date.now() + 30000;
    });
  }

  function turnRewindIsBranchItem(item) {
    return /^(分支到新聊天|在新聊天中分支|从此处分支|Branch (?:in|into|to) (?:a )?new chat|Fork chat from here)$/i
      .test((item?.textContent || "").replace(/\s+/g, " ").trim());
  }

  function turnRewindManagerSupported(value) {
    const methods = ["getHostId", "getConversation", "getCompleteConversationTurns",
      "loadRemainingTurnItems", "sendFollowUpMessage", "addTurnCompletedListener",
      "addConversationStateCallback", "interruptConversation", "sendRequest",
      "applyRevertResponseToConversation"];
    try {
      return value && methods.every((name) => typeof value[name] === "function")
        && value.getHostId() === "local";
    } catch { return false; }
  }

  async function loadTurnRewindDesktopServices() {
    if (turnRewindDesktopServices || typeof codexHostServices !== "function"
        || Date.now() < turnRewindDesktopRetryAt) return turnRewindDesktopServices;
    turnRewindDesktopServicesLoad ||= codexHostServices().then((services) => {
      turnRewindDesktopServices = services;
      if (services?.appServerManagers) scheduleTurnRewindMenuRefresh();
      return services;
    }).catch(() => {
      turnRewindDesktopRetryAt = Date.now() + 30000;
      return null;
    }).finally(() => { turnRewindDesktopServicesLoad = null; });
    return turnRewindDesktopServicesLoad;
  }

  function disposeTurnRewindRpc(resource) {
    try { resource?.[Symbol.dispose]?.(); } catch { /* 连接断开时资源已释放。 */ }
  }

  function turnRewindViewReopener(trigger, threadId) {
    const fiberKey = Object.getOwnPropertyNames(trigger || {}).find((name) => name.startsWith("__reactFiber$"));
    let routeContext, reactRoot;
    for (let fiber = trigger?.[fiberKey], depth = 0; fiber && depth < 192; fiber = fiber.return, depth++) {
      const value = fiber.memoizedProps?.value;
      if (typeof value?.location?.key === "string" && ["POP", "PUSH", "REPLACE"].includes(value.navigationType)) {
        routeContext = fiber.type;
      }
      if (fiber.tag === 3) reactRoot = fiber.stateNode;
    }
    if (!routeContext || !reactRoot) return null;
    const readLocation = () => {
      const stack = [reactRoot.current];
      for (let scanned = 0; stack.length && scanned < 1024; scanned++) {
        const fiber = stack.pop();
        if (!fiber) continue;
        if (fiber.type === routeContext) return fiber.memoizedProps?.value?.location;
        if (fiber.sibling) stack.push(fiber.sibling);
        if (fiber.child && !(fiber.tag === 22 && fiber.memoizedState)) stack.push(fiber.child);
      }
      return null;
    };
    const original = readLocation();
    if (original?.pathname !== `/local/${threadId}`) return null;
    const navigate = (path, state) => window.dispatchEvent(new MessageEvent("message", {
      data: { type: "navigate-to-route", path, state, replace: true },
      origin: window.location.origin, source: window,
    }));
    return async () => {
      const current = readLocation();
      if (current?.key !== original.key || current.pathname !== original.pathname) return null;
      const marker = crypto.randomUUID();
      const parked = () => {
        const location = readLocation();
        return location?.pathname === "/" && location.state?.codexPlusRewindRefresh === marker;
      };
      navigate("/", { codexPlusRewindRefresh: marker });
      // 等离开原生聊天视图的清理函数释放它持有的历史订阅。
      await new Promise((resolve) => setTimeout(resolve, 0));
      if (!parked()) return null;
      return () => {
        if (parked()) navigate(original.pathname + original.search + original.hash, original.state);
      };
    };
  }

  function turnRewindDesktopManager(client, threadId, parkView) {
    let onConversationState;
    return {
      getHostId: () => "local",
      async getConversation(id) {
        const state = await client.getConversation(id);
        return state?.id === id && !state.ephemeral && state.threadAccess !== "shared" ? state : null;
      },
      async getStreamRole(id) {
        return { role: await client.hasLocalHistorySubscription(id) ? "owner" : "follower" };
      },
      async getCompleteConversationTurns(id) {
        await client.ensureConversationHistoryLoaded(id, [], { force: true });
        const history = await client.getLoadedConversationHistory(id);
        if (history?.isComplete !== true || !Array.isArray(history.turns)
            || history.turns.some((turn) => turn.itemsPagination?.hasLoadedOldest === false)) {
          throw Error("无法读取此聊天的完整执行记录，原对话保留。");
        }
        return history.turns;
      },
      async validateHistory(id, expectedIds) {
        // 宿主缓存可能落后于其他窗口，或还显示已经截断的轮次。写入前读服务端的轮次 ID。
        const cursors = new Set();
        let index = 0, cursor = null;
        do {
          const page = await client.sendRequest("thread/turns/list", {
            threadId: id, cursor, limit: 100, sortDirection: "asc", itemsView: "notLoaded",
          });
          if (!Array.isArray(page?.data)) throw Error("无法核对服务端会话历史，原对话保留。");
          for (const turn of page.data) {
            if (index >= expectedIds.length || turn.id !== expectedIds[index] || turn.status === "inProgress") {
              throw Error("服务端会话历史已变化或仍有任务执行，原对话保留。");
            }
            index += 1;
          }
          cursor = page.nextCursor ?? null;
          if (cursor !== null && (!page.data.length || cursors.has(cursor))) {
            throw Error("无法核对服务端会话历史，原对话保留。");
          }
          cursors.add(cursor);
        } while (cursor !== null);
        if (index !== expectedIds.length) throw Error("服务端会话历史已变化，原对话保留。");
      },
      addConversationStateCallback(listener) {
        onConversationState = listener;
        return () => { onConversationState = null; };
      },
      async addTurnCompletedListener(listener) {
        const pending = client.subscribe({
          type: "notification", key: { hostId: "local", threadId },
          methods: ["item/started", "item/completed", "turn/completed"],
          listener({ method, params }) {
            if (params?.threadId !== threadId) return;
            if (method === "turn/completed" && params.turn?.id) {
              void listener({ conversationId: threadId, turnId: params.turn.id });
            } else if (params.item?.type === "userMessage" && params.turnId) {
              onConversationState?.(threadId, { turns: [{ turnId: params.turnId, items: [params.item] }] });
            }
          },
        });
        try {
          const lease = await pending;
          return () => { disposeTurnRewindRpc(lease); disposeTurnRewindRpc(pending); };
        } catch (error) {
          disposeTurnRewindRpc(pending);
          throw error;
        }
      },
      sendFollowUpMessage: (id, options) => client.sendFollowUpMessage(id, options),
      interruptConversation: (id, reason, turnId) => client.interruptConversation(id, reason, turnId),
      sendRequest: (method, params) => client.sendRequest(method, params),
      async applyRevertResponseToConversation({ conversationId, response, revertedTurns, retainedTurnId }) {
        if (response?.thread?.id !== conversationId) throw Error("回退响应的会话不匹配。");
        const removed = new Set(revertedTurns.map((turn) => turn.turnId));
        const hasRefreshedHistory = async () => {
          const history = await client.getLoadedConversationHistory(conversationId);
          if (!Array.isArray(history?.turns) || history.turns.some((turn) => removed.has(turn.turnId))) return false;
          return retainedTurnId === null ? history.turns.length === 0
            : history.turns.some((turn) => turn.turnId === retainedTurnId);
        };
        const state = await client.getConversation(conversationId);
        if (state?.resumeState === "resumed" && await hasRefreshedHistory()) return;
        if (!state || state.threadRuntimeStatus?.type === "active") throw Error("桌面会话当前无法刷新。");

        if (state.resumeState === "resumed" && await client.hasLocalHistorySubscription(conversationId)) {
          const latest = await client.sendRequest("thread/turns/list", {
            threadId: conversationId, limit: 1, sortDirection: "desc", itemsView: "notLoaded",
          });
          const current = await client.sendRequest("thread/read", { threadId: conversationId, includeTurns: false });
          if (!Array.isArray(latest?.data) || (latest.data[0]?.id ?? null) !== retainedTurnId
              || latest.data[0]?.status === "inProgress" || current?.thread?.id !== conversationId
              || current.thread.status?.type !== "idle") {
            throw Error("服务端会话已变化，无法刷新窗口。");
          }
          // 活动视图会让原生恢复流程保留旧轮次。先离开目标路由，再让宿主释放旧缓存。
          const restoreView = await parkView?.();
          try {
            const beforeUnsubscribe = await client.getConversation(conversationId);
            if (beforeUnsubscribe?.threadRuntimeStatus?.type !== "idle") {
              throw Error("桌面会话当前无法刷新。");
            }
            await client.unsubscribeInactiveConversation(conversationId);
          } finally {
            restoreView?.();
          }
        }
        const deadline = Date.now() + 8000;
        do {
          const current = await client.getConversation(conversationId);
          if (current?.resumeState === "resumed" && await hasRefreshedHistory()) return;
          await new Promise((resolve) => setTimeout(resolve, 200));
        } while (Date.now() < deadline);
        throw Error("桌面会话尚未同步回退后的历史。");
      },
    };
  }

  function turnRewindMenuOwners(node) {
    const menu = node?.closest?.('[role="menu"]');
    if (!menu) return [];
    const menuId = menu.getAttribute("id");
    const labels = (menu.getAttribute("aria-labelledby") || "").split(/\s+/).filter(Boolean);
    return [...document.querySelectorAll('[aria-haspopup]')].filter((trigger) => {
      if (!["menu", "true"].includes(trigger.getAttribute("aria-haspopup"))
          || trigger.getAttribute("aria-expanded") !== "true") return false;
      const controls = (trigger.getAttribute("aria-controls") || "").split(/\s+/).filter(Boolean);
      const triggerId = trigger.getAttribute("id");
      const controlsMenu = !!menuId && controls.includes(menuId);
      const labelsTrigger = !!triggerId && labels.includes(triggerId);
      if ((labelsTrigger && menuId && controls.length && !controlsMenu)
          || (controlsMenu && triggerId && labels.length && !labelsTrigger)) return false;
      return controlsMenu || labelsTrigger;
    });
  }

  // 原生菜单可能经过深层 Radix 包装或 portal。除了菜单自身，使用原生
  // aria-controls / aria-labelledby 的明确关联回到触发按钮，不按 DOM 顺序猜目标。
  function turnRewindContextFromNode(node, details = {}, identityOnly = false) {
    const identities = new Map();
    const managers = new Set();
    const owners = turnRewindMenuOwners(node);
    Object.assign(details, { ownerCount: owners.length, fiberFoundCount: 0, maxAncestorDepth: 0,
      scanLimitReached: false, menuScanLimitReached: false, messageBoundaryResolved: false,
      registryReaderReady: !!turnRewindRegistryReader, identityCount: 0, managerCount: 0 });
    if (owners.length > 1) return null;
    const seenValues = new Set();
    let inspectedValues = 0;
    const inspect = (root) => {
      const queue = [{ value: root, depth: 0 }];
      for (; queue.length && inspectedValues < 4096;) {
        const { value, depth } = queue.shift();
        if (!value || typeof value !== "object" || seenValues.has(value)) continue;
        seenValues.add(value);
        inspectedValues += 1;
        if (turnRewindManagerSupported(value)) { managers.add(value); continue; }
        try {
          if (typeof value.getImplForHostId === "function") {
            const manager = value.getImplForHostId("local");
            if (turnRewindManagerSupported(manager)) managers.add(manager);
          }
          if (turnRewindRegistryReader && ["get", "set", "watch"].every((name) => typeof value[name] === "function")) {
            try {
              const manager = turnRewindRegistryReader.call({ scope: value }, "local");
              if (turnRewindManagerSupported(manager)) managers.add(manager);
            } catch { /* 非客户端 scope 不可用，不能尝试写入。 */ }
          }
          if (depth >= 4) continue;
          for (const name of Object.getOwnPropertyNames(value).slice(0, 32)) {
            if (["children", "_owner", "return", "child", "sibling", "stateNode", "alternate"].includes(name)) continue;
            const child = Object.getOwnPropertyDescriptor(value, name)?.value;
            if (child && typeof child === "object") queue.push({ value: child, depth: depth + 1 });
          }
        } catch { /* 原生 RPC stub 可能拒绝反射，保持不可用。 */ }
      }
    };
    const matchingManagers = (threadId) => [...managers].filter((manager) => {
      try { return !!manager.getConversation(threadId); } catch { return false; }
    });
    const seenFibers = new Set();
    for (const anchor of [...owners, node]) {
      const isOwner = anchor === owners[0];
      let messageBoundary = null;
      let fiber = null;
      for (let element = anchor, depth = 0; element && element !== document.body && depth < 8;
          element = element.parentElement, depth += 1) {
        const key = Object.getOwnPropertyNames(element).find((name) =>
          name.startsWith("__reactFiber$") || name.startsWith("__reactInternalInstance$"));
        if (key) { fiber = element[key]; break; }
      }
      if (fiber) details.fiberFoundCount += 1;
      let depth = 0;
      for (; fiber && !seenFibers.has(fiber) && depth < 192; depth += 1, fiber = fiber.return) {
        seenFibers.add(fiber);
        // pendingProps / alternate 可能属于另一轮，只读取已提交的 props。
        const props = fiber.memoizedProps;
        const threadId = props?.threadId || props?.conversationId;
        const turnId = props?.turnId;
        const hasIdentity = typeof threadId === "string" && typeof turnId === "string" && threadId && turnId;
        // 原生回答操作栏同时接收 threadId、turnId 和 onFork；它是触发按钮的消息边界。
        // 有明确菜单归属时，不把更上层页面/列表的 props 当成选中的消息。
        if (hasIdentity && (!isOwner || (!messageBoundary && typeof props.onFork === "function"))) {
          const key = JSON.stringify([threadId, turnId]);
          const previousHost = identities.get(key)?.hostId;
          if (previousHost && props.hostId && previousHost !== props.hostId) return null;
          const identity = { threadId, turnId, hostId: props.hostId || previousHost };
          identities.set(key, identity);
          if (isOwner) messageBoundary = identity;
        }
        if (!identityOnly) {
          inspect(props?.value);
          inspect(props?.scope);
          for (let hook = fiber.memoizedState, count = 0; hook && count < 48; hook = hook.next, count += 1) {
            inspect(hook.memoizedState);
          }
          for (let context = fiber.dependencies?.firstContext, count = 0; context && count < 32; context = context.next, count += 1) {
            inspect(context.memoizedValue);
          }
        }
        if (isOwner && messageBoundary && (identityOnly || matchingManagers(messageBoundary.threadId).length === 1)) {
          // 新版通过宿主取得接口，触发按钮一侧只需找到原生回答操作栏。
          details.messageBoundaryResolved = true;
          break;
        }
        if (!isOwner && owners.length === 1 && hasIdentity) break;
      }
      details.maxAncestorDepth = Math.max(details.maxAncestorDepth, depth);
      if (fiber && !seenFibers.has(fiber)) {
        if (isOwner || owners.length === 0) details.scanLimitReached = true;
        else details.menuScanLimitReached = true;
      }
    }
    details.identityCount = identities.size;
    details.managerCount = managers.size;
    // 必要的消息边界识别仍未完成时不能猜目标；菜单包装的可选扫描不替代原生归属。
    if (details.scanLimitReached || (owners.length === 1 && !details.messageBoundaryResolved) || identities.size !== 1) return null;
    const target = [...identities.values()][0];
    if (target.hostId && target.hostId !== "local") return null;
    if (identityOnly) return target;
    const matches = matchingManagers(target.threadId);
    details.matchCount = matches.length;
    return matches.length === 1 ? { ...target, manager: matches[0] } : null;
  }

  async function turnRewindNativeSaveService() {
    if (!turnRewindSaveServicePromise) {
      turnRewindSaveServicePromise = (async () => {
        const files = (await loadTurnRewindDesktopServices())?.workspaceFiles;
        if (typeof files?.saveCopy === "function") return files;
        for (const prefix of ["app-shared-", "app-initial-"]) {
          try {
            const module = await loadCodexAppModule(prefix);
            for (const candidate of Object.values(module || {})) {
              try {
                if (candidate && typeof candidate === "object"
                    && typeof candidate.workspaceFiles?.saveCopy === "function") return candidate.workspaceFiles;
              } catch { /* 跳过不支持此能力的 RPC stub。 */ }
            }
          } catch { /* 尝试下一个已知模块前缀。 */ }
        }
        throw Error("当前客户端没有可用的原生另存为接口，恢复记录仍保留在本机。");
      })().catch((error) => { turnRewindSaveServicePromise = null; throw error; });
    }
    return turnRewindSaveServicePromise;
  }

  codexPlusTurnRewind.setSaveCopy(async (request) => {
    const files = await turnRewindNativeSaveService();
    const operation = files.saveCopy(request);
    try { return await operation; }
    finally { operation?.[Symbol.dispose]?.(); }
  });

  function turnRewindAvailability(branch, identityOnly = false) {
    if (!codexPlusTurnRewindEnabled()) return { reason: "请先在 Codex++ 设置中开启轮次回退。" };
    const details = {};
    const target = turnRewindContextFromNode(branch, details, identityOnly);
    if (!target) {
      const reason = details.scanLimitReached
        ? "消息结构超出当前支持的安全识别范围，未执行回退。"
        : details.identityCount !== 1
        ? "无法唯一识别此消息的聊天和轮次，未执行回退。请重新打开此聊天后重试。"
        : "无法获取此聊天的原生回退接口，未执行回退。请确认通过最新 Codex++ 测试版启动客户端。";
      return { reason, details };
    }
    if (identityOnly) return codexPlusTurnRewind.getActive(target.threadId)
      ? { reason: "此聊天已有回退任务。", details }
      : { target, details };
    codexPlusTurnRewind.registerManager(target.threadId, target.manager);
    return codexPlusTurnRewind.canRun(target.threadId, target.turnId)
      ? { target, details }
      : { reason: "此聊天尚未加载完成、仍有任务执行，或已有回退任务。请等待结束后再试。", details };
  }

  function refreshTurnRewindMenus() {
    if (turnRewindMenuDisposed || !codexPlusTurnRewindEnabled()) return;
    if (!turnRewindDesktopServices?.appServerManagers) prepareTurnRewindRegistryReader();
    for (const menu of document.querySelectorAll('[role="menu"]')) {
      if (menu.hidden || menu.getAttribute("data-state") === "closed") continue;
      const branch = [...menu.querySelectorAll('[role="menuitem"]')].find(turnRewindIsBranchItem);
      if (!branch) continue;
      void loadTurnRewindDesktopServices();
      let button = menu.querySelector(`[data-codex-plus-ext="${turnRewindMenuMark}"]`);
      if (!button) {
        button = document.createElement("button");
        button.type = "button";
        button.className = `${branch.className || ""} codex-plus-turn-rewind-item`;
        button.setAttribute("data-codex-plus-ext", turnRewindMenuMark);
        button.setAttribute("role", "menuitem");
        button.textContent = "回退到本轮之前 · Codex++";
        button.style.cssText = "width:100%;text-align:start;cursor:pointer;";
        button.addEventListener("click", async (event) => {
          event.preventDefault();
          event.stopPropagation();
          if (button.disabled) return;
          // 点击时重读 React 身份；菜单节点可被复用，不能信任上次扫描的引用。
          const currentBranch = button.previousElementSibling;
          const menu = currentBranch?.closest('[role="menu"]');
          let operation, client;
          button.disabled = true;
          try {
            if (!menu || !turnRewindIsBranchItem(currentBranch)) throw Error("原生菜单已变化，请关闭菜单后重新打开再试。");
            const original = turnRewindAvailability(currentBranch, true).target;
            const services = original ? await loadTurnRewindDesktopServices() : null;
            const desktop = typeof services?.appServerManagers?.open === "function";
            if (desktop) {
              if (!codexPlusTurnRewind.supportsDesktopManager) throw Error("轮次回退已更新，请重启 Codex 后重试。");
              operation = services.appServerManagers.open("local");
              const connection = await operation;
              client = connection?.manager;
              if (connection?.status !== "ready" || !client || await client.getHostId() !== "local") {
                throw Error("本地会话管理器尚未连接，请稍后重试。");
              }
            }
            const availability = turnRewindAvailability(currentBranch, desktop);
            if (!availability.target) {
              if (typeof sendCodexPlusDiagnostic === "function") {
                void sendCodexPlusDiagnostic("turn_rewind_unavailable", availability.details || {});
              }
              throw Error(availability.reason);
            }
            const { target } = availability;
            if (button.previousElementSibling !== currentBranch || currentBranch.closest('[role="menu"]') !== menu
                || menu.hidden || menu.getAttribute("data-state") === "closed"
                || (original && (original.threadId !== target.threadId || original.turnId !== target.turnId))) {
              throw Error("原生菜单已变化，请关闭菜单后重新打开再试。");
            }
            // 自定义 DOM 项没有 Radix 的 onSelect；关闭原生菜单后释放模态焦点和指针锁。
            const parkView = client ? (
              // 消息操作栏可能离 React 根节点很远。优先从同一窗口的原生侧栏定位页面路由。
              turnRewindViewReopener(document.querySelector("nav[data-app-navigation-rail], aside.app-shell-left-panel"), target.threadId)
              || turnRewindViewReopener(turnRewindMenuOwners(currentBranch)[0], target.threadId)
            ) : null;
            menu.dispatchEvent(new KeyboardEvent("keydown", {
              key: "Escape", code: "Escape", bubbles: true, cancelable: true,
            }));
            if (client) {
              const manager = turnRewindDesktopManager(client, target.threadId, parkView);
              await codexPlusTurnRewind.run(target.threadId, target.turnId, { manager });
            } else {
              await codexPlusTurnRewind.run(target.threadId, target.turnId);
            }
          } catch (error) { showToast(String(error?.message || error), { type: "error" }); }
          finally {
            disposeTurnRewindRpc(client);
            disposeTurnRewindRpc(operation);
            button.disabled = false;
            scheduleTurnRewindMenuRefresh();
          }
        });
        button.addEventListener("pointerdown", (event) => event.stopPropagation());
        // 原生菜单的键盘事件由其 roving-focus 管理器消费；自己的项显式支持。
        button.addEventListener("keydown", (event) => {
          if (!["Enter", " "].includes(event.key) || button.disabled) return;
          event.preventDefault();
          event.stopPropagation();
          button.click();
        });
        branch.insertAdjacentElement("afterend", button);
      }
      const availability = turnRewindAvailability(branch, !!turnRewindDesktopServices?.appServerManagers);
      // 保留点击处理，以便接口不可用时解释原因；只在实际启动期间禁用原生按钮。
      button.setAttribute("aria-disabled", String(!availability.target));
      button.style.opacity = availability.target ? "1" : "0.55";
      button.title = availability.reason || "撤销本轮及后续轮次的可恢复操作；AI 报告全部完成后截断会话。";
    }
  }

  function scheduleTurnRewindMenuRefresh() {
    if (turnRewindMenuDisposed || turnRewindRefreshTimer) return;
    turnRewindRefreshTimer = setTimeout(() => {
      turnRewindRefreshTimer = 0;
      refreshTurnRewindMenus();
    }, 60);
  }

  function syncCodexPlusTurnRewind() {
    if (!codexPlusTurnRewindEnabled()) {
      turnRewindObserver?.disconnect();
      turnRewindObserver = null;
      clearTimeout(turnRewindRefreshTimer);
      turnRewindRefreshTimer = 0;
      document.querySelectorAll(`[data-codex-plus-ext="${turnRewindMenuMark}"]`).forEach((node) => node.remove());
      return;
    }
    if (!turnRewindDesktopServices?.appServerManagers) prepareTurnRewindRegistryReader();
    if (!turnRewindObserver && document.body) {
      turnRewindObserver = new MutationObserver((records) => {
        // 自己的菜单项与状态面板都带归属标记，避免注入引起自喂扫描。
        const touchesMenu = records.some((record) => {
          if (record.target?.closest?.('[data-codex-plus-ext]')) return false;
          const added = [...(record.addedNodes || [])];
          if (added.length && added.every((node) => node.nodeType === 1 && node.hasAttribute("data-codex-plus-ext"))) return false;
          return !!record.target?.closest?.('[role="menu"]') || added.some((node) =>
            node.nodeType === 1 && (node.matches?.('[role="menu"]') || node.querySelector?.('[role="menu"]')));
        });
        if (touchesMenu) scheduleTurnRewindMenuRefresh();
      });
      turnRewindObserver.observe(document.body, { childList: true, subtree: true });
      refreshTurnRewindMenus();
    }
  }

  window.__codexPlusTurnRewindMenuCleanup = () => {
    turnRewindMenuDisposed = true;
    turnRewindObserver?.disconnect();
    clearTimeout(turnRewindRefreshTimer);
    document.querySelectorAll(`[data-codex-plus-ext="${turnRewindMenuMark}"]`).forEach((node) => node.remove());
    // 在途回滚与恢复面板保留，不能因重注入失去停止/导出入口。
  };
