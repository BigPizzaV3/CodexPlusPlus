  function ensureConversationViewRuntime() {
    if (conversationViewState.runtimeStarted) return;
    conversationViewState.ro = conversationViewState.ro || new ResizeObserver(() => scheduleConversationViewAlign());
    conversationViewState.mo = conversationViewState.mo || new MutationObserver(() => scheduleConversationViewAlign());
    if (document.body && !conversationViewState.moObserved) {
      conversationViewState.mo.observe(document.body, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ["class", "hidden", "data-state", "aria-hidden"],
      });
      conversationViewState.moObserved = true;
    }
    conversationViewState.runtimeStarted = true;
  }

  function refreshConversationView() {
    if (!codexPlusSettings().conversationView) {
      cleanupConversationView();
      return;
    }
    ensureConversationViewRuntime();
    scheduleConversationViewAlign();
  }


  const officialUsageWindowMarker = "data-codex-plus-official-usage-window";
  // 重新注入会替换局部配置；已有 Query 钩子必须通过同一个运行时读取新策略。
  const officialUsageRuntime = window.__codexPlusOfficialUsageRuntime ||= {
    rawPayloads: new WeakMap(),
    rewriteDepth: 0,
  };
  officialUsageRuntime.pendingPublications ||= new WeakMap();
  officialUsageRuntime.rewrite = rewriteTrackedOfficialUsagePayload;
  window.__codexPlusOfficialUsageWindowCleanup?.();

  function isOfficialLowQuotaSidebarCard(node) {
    if (node?.nodeType !== Node.ELEMENT_NODE || node.getAttribute("role") !== "status") return false;
    const className = typeof node.className === "string" ? node.className : "";
    if (!className.includes("rounded-2xl") || !className.includes("ring-border")) return false;
    const content = node.textContent || "";
    return content.includes("usage remaining")
      || (content.includes("剩余") && content.includes("使用量"))
      || content.includes("重新加入 Plus")
      || content.includes("Rejoin Plus");
  }

  function isOfficialLowQuotaComposerBanner(node) {
    return isOfficialLowQuotaUpsellBanner(node) || isOfficialLowQuotaComposerAside(node);
  }

  function isOfficialLowQuotaUpsellBanner(node) {
    if (node?.nodeType !== Node.ELEMENT_NODE || node.getAttribute("role") !== "status") return false;
    const labelledBy = node.getAttribute("aria-labelledby") || "";
    const describedBy = node.getAttribute("aria-describedby") || "";
    if (!labelledBy.startsWith("upsell-banner-title-") || !describedBy.startsWith("upsell-banner-description-")) return false;
    const content = node.textContent || "";
    return content.includes("Codex 和工作使用额度已用完")
      || content.includes("You’re out of Codex and Work usage")
      || content.includes("You're out of Codex and Work usage")
      || content.includes("立即升级以获取更多使用量")
      || content.includes("Upgrade for more now");
  }

  function isOfficialLowQuotaComposerAside(node) {
    if (node?.nodeType !== Node.ELEMENT_NODE || node.tagName !== "ASIDE") return false;
    const className = typeof node.className === "string" ? node.className : "";
    if (!className.includes("rounded-3xl")) return false;
    const content = node.textContent || "";
    if (content.length > 400) return false;
    return content.includes("Codex 和工作使用额度已用完")
      || content.includes("You’re out of Codex and Work usage")
      || content.includes("You're out of Codex and Work usage");
  }

  function isOfficialLowQuotaWindow(node) {
    return isOfficialLowQuotaSidebarCard(node) || isOfficialLowQuotaComposerBanner(node);
  }

  let officialUsageWindowObserver = null;
  let officialUsageWindowHidden = false;
  let officialUsageWindowStartPending = false;
  let officialUsageWindowActive = true;
  const officialUsageWindowDisplays = new WeakMap();

  function restoreOfficialUsageWindow(node) {
    if (node.getAttribute(officialUsageWindowMarker) !== "hidden") return;
    node.removeAttribute(officialUsageWindowMarker);
    const display = officialUsageWindowDisplays.get(node);
    if (display?.value) node.style.setProperty("display", display.value, display.priority);
    else node.style.removeProperty("display");
    officialUsageWindowDisplays.delete(node);
  }

  function syncOfficialUsageWindow(node) {
    if (!isOfficialLowQuotaWindow(node)) {
      restoreOfficialUsageWindow(node);
      return;
    }
    if (node.getAttribute(officialUsageWindowMarker) !== "hidden") {
      officialUsageWindowDisplays.set(node, {
        value: node.style.getPropertyValue("display"),
        priority: node.style.getPropertyPriority("display"),
      });
      node.setAttribute(officialUsageWindowMarker, "hidden");
      node.style.setProperty("display", "none", "important");
    }
  }

  function hideOfficialUsageWindowsWithin(root) {
    if (typeof Node === "undefined" || !root || root.nodeType !== Node.ELEMENT_NODE) return;
    const nodes = [root, ...root.querySelectorAll(`[role="status"], aside, [${officialUsageWindowMarker}]`)];
    for (const node of nodes) syncOfficialUsageWindow(node);
  }

  function restoreOfficialUsageWindows() {
    for (const node of document.querySelectorAll(`[${officialUsageWindowMarker}="hidden"]`)) {
      restoreOfficialUsageWindow(node);
    }
  }

  function startOfficialUsageWindowBlock() {
    if (officialUsageWindowObserver || typeof MutationObserver !== "function" || !document.body) return;
    officialUsageWindowObserver = new MutationObserver((records) => {
      if (!officialUsageWindowActive || !officialUsageWindowHidden) return;
      const changedContainers = new Set();
      for (const record of records) {
        // React 可只更新已有文本或插入卡片内部节点，因此也检查变更目标的祖先。
        let parent = record.target?.nodeType === Node.ELEMENT_NODE ? record.target : record.target?.parentElement;
        for (; parent; parent = parent.parentElement) {
          if (parent.matches?.(`[role="status"], aside, [${officialUsageWindowMarker}]`)) changedContainers.add(parent);
        }
        for (const node of record.addedNodes || []) {
          if (node?.nodeType !== Node.ELEMENT_NODE) continue;
          hideOfficialUsageWindowsWithin(node);
        }
      }
      for (const node of changedContainers) syncOfficialUsageWindow(node);
    });
    officialUsageWindowObserver.observe(document.body, {
      childList: true, subtree: true, characterData: true,
      attributes: true, attributeFilter: ["role", "class", "aria-labelledby", "aria-describedby"],
    });
    hideOfficialUsageWindowsWithin(document.body);
  }

  function stopOfficialUsageWindowBlock() {
    officialUsageWindowObserver?.disconnect();
    officialUsageWindowObserver = null;
    restoreOfficialUsageWindows();
  }

  function startOfficialUsageWindowsAfterReady() {
    officialUsageWindowStartPending = false;
    if (officialUsageWindowActive) syncOfficialUsageWindowMode(officialUsagePolicyKey());
  }

  window.__codexPlusOfficialUsageWindowCleanup = () => {
    officialUsageWindowActive = false;
    officialUsageWindowHidden = false;
    document.removeEventListener("DOMContentLoaded", startOfficialUsageWindowsAfterReady);
    stopOfficialUsageWindowBlock();
  };

  // 卡片不读用量字段。观察器只在开关打开时挂一次；心跳不查页面。
  // 关掉或离开官登时只按标记恢复，不再整页重认。
  function syncOfficialUsageWindowMode(key) {
    if (!officialUsageWindowActive) return;
    const hide = key === "official-hide";
    if (!hide) {
      if (!officialUsageWindowHidden && !officialUsageWindowObserver) return;
      officialUsageWindowHidden = false;
      officialUsageWindowStartPending = false;
      stopOfficialUsageWindowBlock();
      return;
    }
    officialUsageWindowHidden = true;
    if (officialUsageWindowObserver) return;
    if (!document.body) {
      if (officialUsageWindowStartPending) return;
      officialUsageWindowStartPending = true;
      document.addEventListener("DOMContentLoaded", startOfficialUsageWindowsAfterReady, { once: true });
      return;
    }
    startOfficialUsageWindowBlock();
  }

  function scanLightweight() {
    installStyle();
    installCodexServiceTierDispatcherPatch();
    installCodexAppServerClientPrototypePatch();
    installCodexRemoteSessionRecoveryListener();
    if (window.__codexPlusRemoteSessionRecoveryDispatcher) {
      installCodexRemoteSessionDispatcherSubscription(
        window.__codexPlusRemoteSessionRecoveryDispatcher,
        "existing-renderer"
      );
    }
    installCodexPlusNavigationEntries();
    // 拓展注册的图标栏入口与内置入口走同一条刷新路径：rail 渲染晚于注入，
    // 所以要每轮扫描都补一次（内部靠 id 幂等，不会重复插入）。
    refreshCodexPlusRailNavigation();
    installCodexPlusPageNavigationCloseHandler();
    installSessionShareImportListener();
    scheduleBackendHeartbeat();
    installDeleteButtonEventDelegation();
    updateThreadScrollHandlers();
    installThreadScrollProgrammaticScrollGuard();
    installThreadScrollNavigationCapture();
    installThreadScrollUserIntentCapture();
    installThreadScrollRouteHooks();
    scheduleThreadScrollSync(true);
    refreshCodexServiceTierControls();
  }

  function officialUsagePolicy() {
    // 新一代设置尚未返回时沿用最后一次真实配置，避免重新注入短暂恢复额度锁。
    if (!codexPlusBackendSettingsLoaded && officialUsageRuntime.lastPolicy) return officialUsageRuntime.lastPolicy;
    const profile = codexRemoteSessionActiveProfile();
    const official = String(profile?.relayMode || "") === "official";
    const mixed = official && profile?.officialMixApiKey === true;
    const policy = {
      official,
      hideAlerts: mixed,
      unlockSend: mixed,
    };
    if (codexPlusBackendSettingsLoaded) officialUsageRuntime.lastPolicy = policy;
    return policy;
  }

  function officialUsagePolicyKey(policy = officialUsagePolicy()) {
    if (!policy.hideAlerts && !policy.unlockSend) return "off";
    return "official-hide";
  }

  function isOfficialUsageStatus(value) {
    if (!value || typeof value !== "object" || Array.isArray(value)) return false;
    const rateLimit = value.rate_limit;
    if (!rateLimit || typeof rateLimit !== "object" || typeof rateLimit.allowed !== "boolean") return false;
    return typeof value.plan_type === "string"
      || typeof value.user_id === "string"
      || typeof value.account_id === "string";
  }

  function isImageGenerationUpsell(value) {
    return String(value?.banner_type || "") === "image_generation_limit_reached";
  }

  function isMainRateLimitQueryKey(queryKey) {
    return Array.isArray(queryKey)
      && queryKey[0] === "rate-limit-status"
      && queryKey[1] !== "image-generation";
  }

  // 低额度提示和发送锁都只看当前是不是官登混入 Key。纯官登不改这份用量。
  // 桌面端发送按钮读 rate_limit.allowed；limit_reached 为 true 也会被当成已用完。
  // 混入时在查询发布前把 allowed 写成 true，并清掉 limit_reached。
  // 百分比、重置时间、账号、积分和消费上限不动。图片额度横幅单独留下。
  function rewriteOfficialUsageStatus(value, policy = officialUsagePolicy()) {
    if (!policy.official || (!policy.hideAlerts && !policy.unlockSend) || !isOfficialUsageStatus(value)) return null;
    const next = { ...value };
    let changed = false;
    if (value.rate_limit_reached_type != null) {
      next.rate_limit_reached_type = null;
      changed = true;
    }
    if (value.model_picker_upsell != null) {
      next.model_picker_upsell = null;
      changed = true;
    }
    const rateLimit = value.rate_limit;
    if (policy.unlockSend && (rateLimit.allowed !== true || rateLimit.limit_reached === true)) {
      next.rate_limit = { ...rateLimit, allowed: true, limit_reached: false };
      changed = true;
    }
    if (policy.hideAlerts) {
      if (value.sidebar_usage_warnings != null) {
        next.sidebar_usage_warnings = null;
        changed = true;
      }
      if (value.rate_limit_warning != null) {
        next.rate_limit_warning = null;
        changed = true;
      }
      if (value.rate_limit_upsell != null && !isImageGenerationUpsell(value.rate_limit_upsell)) {
        next.rate_limit_upsell = null;
        changed = true;
      }
    }
    return changed ? next : null;
  }

  function rewriteOfficialUsagePayload(value, policy = officialUsagePolicy()) {
    if (!value || typeof value !== "object") return value;
    if (isOfficialUsageStatus(value)) return rewriteOfficialUsageStatus(value, policy) || value;
    if (value.usage && value.usage !== value && isOfficialUsageStatus(value.usage)) {
      const usage = rewriteOfficialUsageStatus(value.usage, policy);
      return usage ? { ...value, usage } : value;
    }
    return value;
  }

  function rewriteTrackedOfficialUsagePayload(value) {
    const raw = officialUsageRuntime.rawPayloads.get(value) || value;
    if (officialUsagePolicyKey() === "off") return raw;
    const next = rewriteOfficialUsagePayload(value);
    if (next !== value) officialUsageRuntime.rawPayloads.set(next, raw);
    return next;
  }

  function looksLikeQueryClient(value) {
    return !!value
      && typeof value.getQueryCache === "function"
      && typeof value.setQueryData === "function";
  }

  // 图片额度使用同一条 /wham/usage，只能靠查询键 image-generation 排除。
  // 这里只在第一次挂上缓存时找客户端，不进每轮 DOM 扫描。
  function queryClientFromFiber(fiber) {
    const seen = new Set();
    const stack = [fiber];
    let visited = 0;
    while (stack.length && visited < 8000) {
      const node = stack.pop();
      if (!node || typeof node !== "object" || seen.has(node)) continue;
      seen.add(node);
      visited += 1;
      const props = node.memoizedProps || node.pendingProps;
      if (looksLikeQueryClient(props?.client)) return props.client;
      if (looksLikeQueryClient(props?.value)) return props.value;
      if (looksLikeQueryClient(node.stateNode)) return node.stateNode;
      const state = node.memoizedState;
      if (state && typeof state === "object" && looksLikeQueryClient(state.memoizedState)) return state.memoizedState;
      if (node.child) stack.push(node.child);
      if (node.sibling) stack.push(node.sibling);
    }
    return null;
  }

  let officialUsageClient = null;

  function findCodexQueryClient() {
    const explicit = window.__REACT_QUERY_CLIENT__ || window.__codexQueryClient;
    if (looksLikeQueryClient(explicit)) return explicit;
    if (looksLikeQueryClient(officialUsageClient)) return officialUsageClient;
    const roots = [document.getElementById?.("root"), document.body, document.documentElement].filter(Boolean);
    for (const root of roots) {
      let key = "";
      try {
        key = Object.keys(root).find((name) => name.startsWith("__reactContainer$") || name.startsWith("__reactFiber$")) || "";
      } catch {
        key = "";
      }
      if (!key) continue;
      let fiber = root[key];
      if (fiber?.stateNode?.current) fiber = fiber.stateNode.current;
      const client = queryClientFromFiber(fiber);
      if (client) {
        officialUsageClient = client;
        return client;
      }
    }
    return null;
  }

  function mainRateLimitQueries(client) {
    const cache = client.getQueryCache?.();
    if (cache && typeof cache.findAll === "function") {
      return cache.findAll({ queryKey: ["rate-limit-status"] }).filter((query) => isMainRateLimitQueryKey(query?.queryKey));
    }
    if (typeof client.getQueriesData === "function") {
      return client.getQueriesData({ queryKey: ["rate-limit-status"] })
        .filter(([queryKey]) => isMainRateLimitQueryKey(queryKey))
        .map(([queryKey, data]) => ({ queryKey, state: { data } }));
    }
    return [];
  }

  // Query.setData 是 GET /wham/usage 和 SSE snapshot 共用的发布点。
  // 在通知订阅者之前改写，RK 第一次读到的 allowed 就是结果。
  function patchOfficialUsageQueryPublication(client) {
    const cache = client.getQueryCache?.();
    if (!cache) return;
    const listed = typeof cache.getAll === "function"
      ? cache.getAll()
      : (typeof cache.findAll === "function" ? cache.findAll({ queryKey: ["rate-limit-status"] }) : []);
    const query = listed.find((item) => typeof Object.getPrototypeOf(item)?.setData === "function");
    if (!query) return;
    const proto = Object.getPrototypeOf(query);
    if (typeof proto.setData !== "function" || proto.setData.__codexPlusUsagePublication) return;
    const original = proto.setData;
    function codexPlusPublishUsageData(data, ...rest) {
      if (!isMainRateLimitQueryKey(this?.queryKey)) return original.call(this, data, ...rest);
      const raw = officialUsageRuntime.rawPayloads.get(data) || data;
      const next = officialUsageRuntime.rewrite(data);
      const previousPublication = officialUsageRuntime.pendingPublications.get(this);
      const publication = { raw };
      officialUsageRuntime.pendingPublications.set(this, publication);
      officialUsageRuntime.rewriteDepth += 1;
      try {
        const stored = original.call(this, next, ...rest);
        // TanStack 结构共享可能返回另一对象；快照绑定实际缓存对象，不绑定输入副本。
        // 若订阅者已嵌套发布更新，沿用它登记的快照，不能用外层旧值覆盖。
        if (publication.raw === raw && stored && typeof stored === "object") {
          if (next !== raw) officialUsageRuntime.rawPayloads.set(stored, raw);
          else officialUsageRuntime.rawPayloads.delete(stored);
        }
        if (previousPublication) previousPublication.raw = publication.raw;
        return stored;
      } finally {
        officialUsageRuntime.rewriteDepth -= 1;
        if (previousPublication) officialUsageRuntime.pendingPublications.set(this, previousPublication);
        else officialUsageRuntime.pendingPublications.delete(this);
      }
    }
    codexPlusPublishUsageData.__codexPlusUsagePublication = true;
    proto.setData = codexPlusPublishUsageData;
  }

  function patchOfficialUsageQueryClient(client) {
    if (!client || typeof client.setQueryData !== "function") return;
    patchOfficialUsageQueryPublication(client);
    if (client.__codexPlusUsageRewrite) return;
    const original = client.setQueryData;
    client.setQueryData = function codexPlusSetUsageQueryData(queryKey, updater, ...rest) {
      if (!isMainRateLimitQueryKey(queryKey)) {
        return original.call(this, queryKey, updater, ...rest);
      }
      const nextUpdater = typeof updater === "function"
        ? (previous) => {
          // setData 的同步订阅者可能马上写回，此时结构共享对象尚未返回。
          const query = this.getQueryCache?.()?.find?.({ queryKey, exact: true });
          const publishing = query && officialUsageRuntime.pendingPublications.get(query);
          const raw = publishing ? publishing.raw : (officialUsageRuntime.rawPayloads.get(previous) || previous);
          return officialUsageRuntime.rewrite(updater(raw));
        }
        : officialUsageRuntime.rewrite(updater);
      officialUsageRuntime.rewriteDepth += 1;
      try {
        return original.call(this, queryKey, nextUpdater, ...rest);
      } finally {
        officialUsageRuntime.rewriteDepth -= 1;
      }
    };
    const cache = client.getQueryCache?.();
    if (cache && typeof cache.subscribe === "function") {
      cache.subscribe((event) => {
        if (officialUsageRuntime.rewriteDepth > 0) return;
        const query = event?.query;
        if (!isMainRateLimitQueryKey(query?.queryKey)) return;
        const current = query.state?.data;
        const next = officialUsageRuntime.rewrite(current);
        if (next === current) return;
        client.setQueryData(query.queryKey, next);
      });
    }
    client.__codexPlusUsageRewrite = true;
  }

  function rewriteCachedOfficialUsage(client) {
    if (!client || !officialUsagePolicy().official) return;
    for (const query of mainRateLimitQueries(client)) {
      const current = query.state?.data;
      const next = officialUsageRuntime.rewrite(current);
      if (next !== current) client.setQueryData(query.queryKey, next);
    }
  }

  function invalidateMainRateLimitQueries(client) {
    if (!client || typeof client.invalidateQueries !== "function") return;
    for (const query of mainRateLimitQueries(client)) {
      try {
        Promise.resolve(client.invalidateQueries({ queryKey: query.queryKey, exact: true })).catch(() => {});
      } catch {
      }
    }
  }

  let officialUsagePolicyApplied = "";
  let officialUsageClientTimer = null;

  function syncOfficialUsagePolicy() {
    const key = officialUsagePolicyKey();
    syncOfficialUsageWindowMode(key);
    const client = findCodexQueryClient();
    if (client) {
      officialUsageClient = client;
      patchOfficialUsageQueryClient(client);
      if (officialUsageClientTimer) {
        clearTimeout(officialUsageClientTimer);
        officialUsageClientTimer = null;
      }
    } else if (!officialUsageClientTimer) {
      let attempts = 0;
      const retry = () => {
        attempts += 1;
        officialUsageClientTimer = null;
        if (findCodexQueryClient()) {
          syncOfficialUsagePolicy();
          return;
        }
        if (attempts < 20) officialUsageClientTimer = setTimeout(retry, 300);
      };
      officialUsageClientTimer = setTimeout(retry, 300);
      return;
    } else {
      return;
    }
    const recoveredHomeReads = window.__codexPlusComposerReadiness?.tick(client, officialUsagePolicy().unlockSend) || 0;
    if (recoveredHomeReads > 0) {
      sendCodexPlusDiagnostic("composer_home_read_retried", { count: recoveredHomeReads });
    }
    if (key === officialUsagePolicyApplied) {
      if (key !== "off") rewriteCachedOfficialUsage(client);
      return;
    }
    const previous = officialUsagePolicyApplied;
    officialUsagePolicyApplied = key;
    if (key === "off") {
      // 先同步恢复真实用量；断网或刷新悬挂时也不能沿用混入模式的解锁结果。
      for (const query of mainRateLimitQueries(client)) {
        const raw = officialUsageRuntime.rawPayloads.get(query.state?.data);
        if (raw) client.setQueryData(query.queryKey, raw);
      }
      if (previous) invalidateMainRateLimitQueries(client);
      return;
    }
    rewriteCachedOfficialUsage(client);
    if (previous) invalidateMainRateLimitQueries(client);
  }

  if (window.__CODEX_PLUS_TEST_RATE_LIMIT_UNLOCK__) {
    window.__codexPlusRateLimitUnlockTest = {
      setBackendSettings: (settings) => {
        codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
        codexPlusBackendSettingsLoaded = true;
      },
      setHideAlerts: (hidden) => {
        window.__CODEX_PLUS_HIDE_OFFICIAL_USAGE_ALERT__ = hidden === true;
      },
      install: () => syncOfficialUsagePolicy(),
      policyKey: () => officialUsagePolicyKey(),
      isRateLimitQueryKey: (queryKey) => isMainRateLimitQueryKey(queryKey),
      rewrite: (value) => rewriteOfficialUsagePayload(value),
    };
  }

  function sessionCopyMenuRow(menu) {
    const triggerId = menu?.getAttribute?.("aria-labelledby") || "";
    const trigger = triggerId ? document.getElementById(triggerId) : null;
    const labeledTrigger = trigger && /^(聊天操作|Chat actions)$/i.test(trigger.getAttribute("aria-label") || "")
      ? trigger
      : null;
    const fallbackTrigger = lastSessionActionTrigger?.isConnected
      ? lastSessionActionTrigger
      : document.querySelector('button[aria-label="聊天操作"], button[aria-label="Chat actions"]');
    const row = (labeledTrigger || fallbackTrigger)?.closest?.(selectors.sidebarThread)
      || [...document.querySelectorAll(selectors.sidebarThread)]
        .find((candidate) => candidate.getAttribute("data-app-action-sidebar-thread-selected") === "true");
    const ref = row ? sessionRefFromRow(row) : null;
    if (!row || !ref?.session_id || isClientNewThreadId(ref.session_id)) return null;
    return row;
  }

  function looksLikeSessionActionMenu(menu) {
    if (!(menu instanceof HTMLElement) || menu.hidden) return false;
    if (menu.matches(`.${moreMenuClass}, .${codexPlusMenuFloatingClass}, #${codexPlusMenuId}`)) return false;
    const text = normalizedElementText(menu);
    const hasRename = /(?:重命名|rename)/i.test(text);
    const hasOtherSessionAction = /(?:置顶|取消置顶|pin|unpin|归档|archive|删除|delete|移动|move)/i.test(text);
    return hasRename || hasOtherSessionAction;
  }

  function rememberSessionActionTrigger(event) {
    const trigger = event.target?.closest?.('button[aria-label="聊天操作"], button[aria-label="Chat actions"]');
    if (trigger) lastSessionActionTrigger = trigger;
  }

  function sessionCopyMenuScopes(scope = document) {
    const root = scope?.querySelectorAll ? scope : document;
    const menus = [];
    if (scope instanceof HTMLElement && scope.matches?.('[role="menu"]')) menus.push(scope);
    root.querySelectorAll?.('[role="menu"]').forEach((menu) => menus.push(menu));
    return Array.from(new Set(menus));
  }

  function sessionCopyMenuItemIcon() {
    return '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="2"></rect><path d="M15 9V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h3"></path></svg>';
  }

  function sessionCopyMenuActivationIsDuplicate(target) {
    if (!(target instanceof HTMLElement)) return false;
    const now = Date.now();
    const activatedAt = Number(target.dataset.codexSessionCopyActivatedAt || 0);
    if (activatedAt && now - activatedAt < 600) return true;
    target.dataset.codexSessionCopyActivatedAt = String(now);
    return false;
  }

  async function selectSessionRowForAction(row) {
    if (!(row instanceof HTMLElement) || !row.isConnected) return false;
    const targetId = row.getAttribute("data-app-action-sidebar-thread-id") || "";
    if (!targetId) return false;
    if (row.getAttribute("data-app-action-sidebar-thread-selected") !== "true") row.click();

    const deadline = Date.now() + sessionCopyMenuActivationTimeoutMs;
    while (Date.now() < deadline) {
      const selected = [...document.querySelectorAll(selectors.sidebarThread)]
        .find((candidate) => candidate.getAttribute("data-app-action-sidebar-thread-selected") === "true");
      if (selected?.getAttribute("data-app-action-sidebar-thread-id") === targetId) return true;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return false;
  }

  function dispatchNativePointerClick(node) {
    if (!(node instanceof HTMLElement)) return;
    node.focus?.();
    if (typeof PointerEvent === "function") {
      node.dispatchEvent(new PointerEvent("pointerdown", {
        bubbles: true,
        cancelable: true,
        composed: true,
        button: 0,
        buttons: 1,
        pointerId: 1,
        pointerType: "mouse",
        isPrimary: true,
      }));
      node.dispatchEvent(new PointerEvent("pointerup", {
        bubbles: true,
        cancelable: true,
        composed: true,
        button: 0,
        buttons: 0,
        pointerId: 1,
        pointerType: "mouse",
        isPrimary: true,
      }));
    }
    node.click();
  }

  async function waitForSessionElement(resolveElement, timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const element = resolveElement();
      if (element) return element;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return null;
  }

  function visibleSessionRenameDialog() {
    return [...document.querySelectorAll('[role="dialog"]')]
      .filter(visibleElement)
      .find((dialog) => dialog.querySelector('input[aria-label="聊天标题"], input[aria-label="Chat title"]')) || null;
  }

  function closeSessionRenameDialog(dialog) {
    const cancelButton = [...dialog?.querySelectorAll?.("button") || []]
      .find((button) => /^(取消|Cancel)$/i.test(normalizedElementText(button)));
    if (cancelButton) {
      cancelButton.click();
      return;
    }
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true }));
  }

  function sessionActionTrigger(row) {
    const direct = row?.querySelector?.('button[aria-label="聊天操作"], button[aria-label="Chat actions"]');
    if (direct instanceof HTMLElement && visibleElement(direct)) return direct;
    if (lastSessionActionTrigger instanceof HTMLElement && lastSessionActionTrigger.isConnected && visibleElement(lastSessionActionTrigger)) {
      return lastSessionActionTrigger;
    }
    const candidates = Array.from(document.querySelectorAll([
      'button[aria-label="聊天操作"]',
      'button[aria-label="Chat actions"]',
      'button[aria-label="会话操作"]',
      'button[aria-label="Thread actions"]',
      'button[aria-label="更多"]',
      'button[aria-label="More"]',
      'button[aria-label="More options"]',
      'button[aria-label="更多操作"]',
      'button[aria-label="More actions"]',
      'button[aria-label="会话选项"]',
      'button[aria-label="Conversation options"]',
      'button[aria-label="Thread options"]',
      'button[aria-haspopup="menu"]',
    ].join(","))).filter((button) => {
      if (!(button instanceof HTMLElement) || !visibleElement(button) || isExtensionUiNode(button)) return false;
      const header = button.closest?.(selectors.appHeader);
      return !!header || /聊天操作|Chat actions|会话操作|Thread actions|更多|More|选项|options/i.test(button.getAttribute("aria-label") || "");
    });
    if (candidates.length) return candidates.at(-1);
    const header = document.querySelector(selectors.appHeader);
    const iconButtons = Array.from(header?.querySelectorAll?.("button") || [])
      .filter((button) => button instanceof HTMLElement && visibleElement(button) && !isExtensionUiNode(button))
      .filter((button) => {
        const text = normalizedElementText(button);
        return text === "..." || text === "⋯" || text === "···" || button.getAttribute("aria-expanded") != null;
      });
    return iconButtons.at(-1) || null;
  }

  async function activateSessionAutoRenameMenuItem(event) {
    if (event?.type === "keydown" && !["Enter", " "].includes(event.key)) return;
    const item = event?.currentTarget;
    event?.preventDefault?.();
    event?.stopPropagation?.();
    event?.stopImmediatePropagation?.();
    if (sessionCopyMenuActivationIsDuplicate(item)) return;
    closeSessionMoreMenus();

    const row = item?.__codexSessionAutoRenameRow;
    if (!(row instanceof HTMLElement) || !row.isConnected) {
      showToast("找不到要重命名的会话", null);
      return;
    }
    if (!await selectSessionRowForAction(row)) {
      showToast("会话加载超时，请稍后重试", null);
      return;
    }

    const trigger = sessionActionTrigger(row);
    if (!(trigger instanceof HTMLElement)) {
      showToast("找不到 Codex 原生重命名入口", null);
      return;
    }
    lastSessionActionTrigger = trigger;
    dispatchNativePointerClick(trigger);

    let renameItem = await waitForSessionElement(() => {
      return sessionCopyMenuScopes()
        .filter((menu) => visibleElement(menu) && looksLikeSessionActionMenu(menu))
        .flatMap((menu) => [...menu.querySelectorAll('[role="menuitem"]')])
        .find((candidate) => /^(重命名|Rename)$/i.test(normalizedElementText(candidate))) || null;
    }, 1200);
    if (!renameItem) {
      trigger.click();
      renameItem = await waitForSessionElement(() => {
        return [...document.querySelectorAll('[role="menuitem"]')]
          .filter(visibleElement)
          .find((candidate) => /^(重命名|Rename)$/i.test(normalizedElementText(candidate))) || null;
      }, 1200);
    }
    if (!(renameItem instanceof HTMLElement)) {
      showToast("无法打开 Codex 原生重命名入口", null);
      return;
    }
    renameItem.click();

    const dialog = await waitForSessionElement(visibleSessionRenameDialog, 3000);
    if (!(dialog instanceof HTMLElement)) {
      showToast("无法打开 Codex 原生重命名窗口", null);
      return;
    }
    const titleInput = dialog.querySelector('input[aria-label="聊天标题"], input[aria-label="Chat title"]');
    const initialTitle = titleInput?.value || "";
    showToast("正在使用 Codex 生成会话名称…", null);

    const suggestionButton = await waitForSessionElement(() => {
      const currentDialog = visibleSessionRenameDialog();
      if (!currentDialog) return null;
      return [...currentDialog.querySelectorAll("button")]
        .filter(visibleElement)
        .find((button) => {
          const text = normalizedElementText(button);
          return button.classList.contains("text-info")
            && !!text
            && text !== initialTitle
            && !/^(取消|保存|Cancel|Save)$/i.test(text);
        }) || null;
    }, sessionAutoRenameTimeoutMs);
    if (!(suggestionButton instanceof HTMLElement)) {
      closeSessionRenameDialog(visibleSessionRenameDialog());
      showToast("Codex 未能生成新名称，请稍后重试", null);
      return;
    }

    suggestionButton.click();
    const renamedInput = await waitForSessionElement(() => {
      const input = visibleSessionRenameDialog()?.querySelector('input[aria-label="聊天标题"], input[aria-label="Chat title"]');
      return input?.value?.trim() && input.value.trim() !== initialTitle.trim() ? input : null;
    }, 1500);
    const activeDialog = visibleSessionRenameDialog();
    const saveButton = [...activeDialog?.querySelectorAll?.("button") || []]
      .find((button) => /^(保存|Save)$/i.test(normalizedElementText(button)));
    if (!renamedInput || !(saveButton instanceof HTMLElement) || saveButton.disabled) {
      closeSessionRenameDialog(activeDialog);
      showToast("Codex 未能应用新名称，请稍后重试", null);
      return;
    }
    saveButton.click();
    showToast("已自动重命名当前会话", null);
  }

  async function activateSessionCopyMenuItem(event) {
    if (event?.type === "keydown" && !["Enter", " "].includes(event.key)) return;
    const item = event?.currentTarget;
    event?.preventDefault?.();
    event?.stopPropagation?.();
    event?.stopImmediatePropagation?.();
    if (sessionCopyMenuActivationIsDuplicate(item)) return;
    const row = item?.__codexSessionCopyRow;
    if (!(row instanceof HTMLElement) || !row.isConnected) {
      showToast("找不到要复制的会话", null);
      return;
    }
    if (!await selectSessionRowForAction(row)) {
      showToast("会话加载超时，请稍后重试", null);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 350));
    const forkButtons = [...document.querySelectorAll('button[aria-label="从这里创建聊天分支"], button[aria-label="Fork from here"]')]
      .filter(visibleElement)
      .filter((button) => !isExtensionUiNode(button));
    const forkButton = forkButtons.at(-1);
    if (!forkButton) {
      showToast("当前会话没有可用的官方分支入口", null);
      return;
    }
    forkButton.click();
  }

  function createSessionCopyMenuItem(referenceItem, row) {
    const item = document.createElement("div");
    item.className = referenceItem?.className || "no-drag outline-hidden rounded-lg px-[var(--padding-row-x)] py-[var(--padding-row-y)] text-sm text-default group cursor-interaction flex flex-col";
    item.classList.add(sessionCopyMenuItemClass);
    item.setAttribute("role", referenceItem?.getAttribute("role") || "menuitem");
    item.setAttribute("tabindex", referenceItem?.getAttribute("tabindex") || "-1");
    item.setAttribute("data-orientation", referenceItem?.getAttribute("data-orientation") || "vertical");
    item.setAttribute("data-codex-session-copy-menu", "true");
    item.dataset.codexSessionCopyVersion = sessionCopyMenuItemVersion;
    item.__codexSessionCopyRow = row;
    item.innerHTML = `<div class="flex w-full items-center gap-1.5"><span class="inline-flex h-5 w-5 shrink-0 items-center justify-center opacity-75 group-focus:opacity-100 group-hover:opacity-100">${sessionCopyMenuItemIcon()}</span><span class="flex-1 min-w-0 truncate">原地复制会话 - Codex++</span></div>`;
    item.addEventListener("pointerup", activateSessionCopyMenuItem, true);
    item.addEventListener("click", activateSessionCopyMenuItem, true);
    item.addEventListener("keydown", activateSessionCopyMenuItem, true);
    return item;
  }

  function refreshSessionCopyMenuItems(scope = document) {
    sessionCopyMenuScopes(scope).forEach((menu) => {
      if (!(menu instanceof HTMLElement) || isExtensionUiNode(menu)) return;
      if (menu.matches(`.${moreMenuClass}, .${codexPlusMenuFloatingClass}, #${codexPlusMenuId}`)) {
        menu.querySelectorAll(`.${sessionCopyMenuItemClass}`).forEach((item) => item.remove());
        return;
      }
      const row = sessionCopyMenuRow(menu);
      if (!looksLikeSessionActionMenu(menu)) return;
      const existing = menu.querySelector(`.${sessionCopyMenuItemClass}`);
      if (!row) {
        existing?.remove();
        return;
      }
      if (existing) {
        existing.__codexSessionCopyRow = row;
        return;
      }
      const referenceItem = menu.querySelector('[role="menuitem"]');
      menu.appendChild(createSessionCopyMenuItem(referenceItem, row));
    });
  }

  function scanDeferred() {
    if (codexPluginMarketplacePatchEnabled()) {
      const pluginUnlockStrategy = codexPluginUnlockStrategy();
      const settings = codexPlusSettings();
      logCodexPluginUnlockStrategy(pluginUnlockStrategy);
      if ((pluginUnlockStrategy === "modern" || pluginUnlockStrategy === "unknown") && settings.pluginMarketplaceUnlock) {
        const marketplaceRequestPatchStrategy = codexPluginMarketplaceRequestPatchStrategy();
        installPluginBuildFlavorFilterPatch();
        if (marketplaceRequestPatchStrategy === "bridge") {
          installPluginMarketplaceBridgePatch();
        } else if (marketplaceRequestPatchStrategy === "client") {
          installPluginMarketplaceRequestPatch();
        } else {
          installPluginMarketplaceWindowEventPatchOnly();
          installPluginMarketplaceBridgePatch();
          installPluginMarketplaceRequestPatch();
        }
      }
    }
    refreshDreamSkin();
    refreshThreadIdBadges();
    sessionRows().forEach(tryAttachButton);
    updateDeleteButtonOffsets();
    archivedPageRows().forEach(attachArchivedPageDeleteButton);
    refreshConversationView();
    installCodexServiceTierBadge();
    installSessionShareButton();
    scheduleThreadScrollSync();
    refreshCodexModelWhitelistFromScan(window.__codexSessionDeleteLastMutations);
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
    void installDictationSupportPatch();
    runScanStep(scanLightweight);
    requestAnimationFrame(() => runScanStep(scanDeferred));
  }

