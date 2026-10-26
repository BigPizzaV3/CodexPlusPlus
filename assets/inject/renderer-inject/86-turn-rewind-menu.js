  // 仅接入消息菜单，不改原生模块、安装目录或对第三方拓展开放写 RPC。
  window.__codexPlusTurnRewindMenuCleanup?.();
  const turnRewindMenuMark = "builtin-turn-rewind-menu";
  let turnRewindObserver = null;
  let turnRewindRefreshTimer = 0;
  let turnRewindSaveServicePromise = null;
  let turnRewindRegistryReader = null;
  let turnRewindRegistryLoad = null;
  let turnRewindRegistryModule = null;
  let turnRewindRegistryRetryAt = 0;

  // 新版客户端的 React hooks 只暴露会话 facade，写接口仍在已有的原生 manager。
  // 复用注册表的只读 accessor 与 React 中已有的 scope，不创建 manager，不猜 atom key。
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
  function turnRewindContextFromNode(node, details = {}) {
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
          const identity = { threadId, turnId, hostId: props.hostId };
          identities.set(JSON.stringify([threadId, turnId]), identity);
          if (isOwner) messageBoundary = identity;
        }
        inspect(props?.value);
        inspect(props?.scope);
        for (let hook = fiber.memoizedState, count = 0; hook && count < 48; hook = hook.next, count += 1) {
          inspect(hook.memoizedState);
        }
        for (let context = fiber.dependencies?.firstContext, count = 0; context && count < 32; context = context.next, count += 1) {
          inspect(context.memoizedValue);
        }
        if (isOwner && messageBoundary && matchingManagers(messageBoundary.threadId).length === 1) {
          // 身份已由原生触发按钮及消息操作栏确定，接口也已确认；不再遍历整个应用树。
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
    const matches = matchingManagers(target.threadId);
    details.matchCount = matches.length;
    return matches.length === 1 ? { ...target, manager: matches[0] } : null;
  }

  async function turnRewindNativeSaveService() {
    if (!turnRewindSaveServicePromise) {
      turnRewindSaveServicePromise = (async () => {
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

  function turnRewindAvailability(branch) {
    if (!codexPlusTurnRewindEnabled()) return { reason: "请先在 Codex++ 设置中开启轮次回退。" };
    const details = {};
    const target = turnRewindContextFromNode(branch, details);
    if (!target) {
      const reason = details.scanLimitReached
        ? "消息结构超出当前支持的安全识别范围，未执行回退。"
        : details.identityCount !== 1
        ? "无法唯一识别此消息的聊天和轮次，未执行回退。请重新打开此聊天后重试。"
        : "无法获取此聊天的原生回退接口，未执行回退。请确认通过最新 Codex++ 测试版启动客户端。";
      return { reason, details };
    }
    codexPlusTurnRewind.registerManager(target.threadId, target.manager);
    return codexPlusTurnRewind.canRun(target.threadId, target.turnId)
      ? { target, details }
      : { reason: "此聊天尚未加载完成、仍有任务执行，或已有回退任务。请等待结束后再试。", details };
  }

  function refreshTurnRewindMenus() {
    if (!codexPlusTurnRewindEnabled()) return;
    prepareTurnRewindRegistryReader();
    for (const menu of document.querySelectorAll('[role="menu"]')) {
      if (menu.hidden || menu.getAttribute("data-state") === "closed") continue;
      const branch = [...menu.querySelectorAll('[role="menuitem"]')].find(turnRewindIsBranchItem);
      if (!branch) continue;
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
          // 点击时重读 React 身份；菜单节点可被复用，不能信任上次扫描的引用。
          const currentBranch = button.previousElementSibling;
          try {
            if (!turnRewindIsBranchItem(currentBranch)) throw Error("原生菜单已变化，请关闭菜单后重新打开再试。");
            const availability = turnRewindAvailability(currentBranch);
            if (!availability.target) {
              if (typeof sendCodexPlusDiagnostic === "function") {
                void sendCodexPlusDiagnostic("turn_rewind_unavailable", availability.details || {});
              }
              throw Error(availability.reason);
            }
            const { target } = availability;
            button.disabled = true;
            // 自定义 DOM 项没有 Radix 的 onSelect；关闭原生菜单后释放模态焦点和指针锁。
            currentBranch.closest('[role="menu"]').dispatchEvent(new KeyboardEvent("keydown", {
              key: "Escape", code: "Escape", bubbles: true, cancelable: true,
            }));
            await codexPlusTurnRewind.run(target.threadId, target.turnId);
          } catch (error) { showToast(String(error?.message || error), { type: "error" }); }
          finally { button.disabled = false; scheduleTurnRewindMenuRefresh(); }
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
      const availability = turnRewindAvailability(branch);
      // 保留点击处理，以便接口不可用时解释原因；只在实际启动期间禁用原生按钮。
      button.setAttribute("aria-disabled", String(!availability.target));
      button.style.opacity = availability.target ? "1" : "0.55";
      button.title = availability.reason || "撤销本轮及后续轮次的可恢复操作；AI 报告全部完成后截断会话。";
    }
  }

  function scheduleTurnRewindMenuRefresh() {
    if (turnRewindRefreshTimer) return;
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
    prepareTurnRewindRegistryReader();
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
    turnRewindObserver?.disconnect();
    clearTimeout(turnRewindRefreshTimer);
    document.querySelectorAll(`[data-codex-plus-ext="${turnRewindMenuMark}"]`).forEach((node) => node.remove());
    // 在途回滚与恢复面板保留，不能因重注入失去停止/导出入口。
  };
