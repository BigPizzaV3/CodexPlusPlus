  // 只补同一 canonical island 内被已有内容夹住的空轮次，不预取正常分页边界。
  const previousHistoryTurnRecovery = window.__codexPlusHistoryTurnRecoveryRuntime;
  previousHistoryTurnRecovery?.dispose?.();
  const historyTurnRecoveryPollMs = 5000;
  const historyTurnRecoveryRetryMs = 30000;
  // 看门狗重注入时仍共享正在进行的读取与尝试账本，不能重置成另一条并发加载链。
  const historyTurnRecoveryLedger = previousHistoryTurnRecovery?.ledger || {
    enabled: false, flight: null, attempts: new Map(), notices: new Set(),
    api: null, discoveryAttempts: 0, discoveryAt: 0,
  };
  const historyTurnRecoveryState = {
    disposed: false, timer: null, ledger: historyTurnRecoveryLedger,
  };

  function historyTurnRecoveryEnabled() {
    return codexPlusSettings().historyTurnRecovery === true;
  }

  function historyTurnRecoveryTarget() {
    if (document.hidden || /[?&]initialRoute=/.test(window.location.search)) return null;
    const ref = currentSessionRef();
    const id = normalizedCodexThreadUuid(ref?.session_id);
    // 原生使用内存路由，地址栏可以一直是 index.html；以侧栏归属与已挂载 composer 交叉核对。
    if (!id || ref.host_id !== "local") return null;
    const marker = document.querySelector('[data-codex-composer-root][data-composer-placement="thread"] [data-above-composer-conversation-id]');
    const mountedId = marker?.getAttribute("data-above-composer-conversation-id")?.replace(/^local:/, "");
    if (normalizedCodexThreadUuid(mountedId) !== id) return null;
    return { id, marker };
  }

  function historyTurnRecoverySignal(module, source) {
    // AppScope families 没有公开标签；不能盲调用每个 family 的 scope.get（可能创建资源）。
    // 由原生默认管理器的空值守卫定位 raw manager family；hostState.client 是 RPC 包装，
    // 没有本地 pagination。按 named exports 取对象，不执行源码，也不硬编码压缩导出名。
    const definition = /\(\{get:([\w$]+)\}\)=>\{let ([\w$]+)=\1\(([\w$]+),[\w$]+\);if\(\2==null\)throw Error\((?:`Missing default AppServerManager`|"Missing default AppServerManager"|'Missing default AppServerManager')\);return \2\}/g;
    const definitions = [...source.matchAll(definition)];
    if (definitions.length !== 1) return null;
    const localName = definitions[0][3];
    const aliases = [];
    for (const block of source.matchAll(/\bexport\s*\{([^}]*)\}/g)) {
      for (const binding of block[1].split(",")) {
        const parts = binding.trim().match(/^([\w$]+)(?:\s+as\s+([\w$]+))?$/);
        if (parts?.[1] === localName) aliases.push(parts[2] || parts[1]);
      }
    }
    if (aliases.length !== 1) return null;
    const signal = module?.[aliases[0]];
    return signal?.kind === "signal-family" && signal.scope?.__scopeBrand === "AppScope"
      && typeof signal.resolve === "function" && typeof signal.read === "function" ? signal : null;
  }

  async function discoverHistoryTurnRecoveryApi() {
    const module = await loadOptionalCodexAppModule("app-initial-");
    if (!module) return null;
    const url = codexAppAssetUrl("app-initial-") || await codexAppAssetUrlFromScriptText("app-initial-");
    const parsed = new URL(url);
    if (parsed.protocol !== "app:" || parsed.hostname !== "-" || !/^\/assets\/app-initial-[^/]+\.js$/.test(parsed.pathname)) return null;
    const controller = new AbortController();
    const timer = window.setTimeout(() => controller.abort(), 8000);
    try {
      const response = await fetch(url, { signal: controller.signal });
      if (!response.ok) return null;
      const source = await response.text();
      if (source.length > 16 * 1024 * 1024) return null;
      return historyTurnRecoverySignal(module, source);
    } finally {
      window.clearTimeout(timer);
    }
  }

  function historyTurnRecoveryManager(signal, marker) {
    const roots = [marker, document.querySelector("main"), document.getElementById("root")].filter(Boolean);
    for (const root of roots) {
      const key = Object.getOwnPropertyNames(root).find((name) => name.startsWith("__reactFiber$"));
      let fiber = key ? root[key] : null;
      for (let depth = 0; fiber && depth < 40; depth += 1, fiber = fiber.return) {
        let hook = fiber.memoizedState;
        const scopes = [fiber.memoizedProps?.value];
        for (let index = 0; hook && index < 96; index += 1, hook = hook.next) scopes.push(hook.memoizedState?.current);
        for (const scope of scopes) {
          if (scope?.scope !== signal.scope || typeof scope.get !== "function"
              || !(scope.node?.familyBindings instanceof Map)
              || !scope.node.familyBindings.get(signal)?.has("local")) continue;
          try {
            const manager = scope.get(signal, "local");
            if (manager?.hostId === "local" && manager.getHostId?.() === "local"
                && typeof manager.getConversation === "function"
                && typeof manager.pagination?.itemPagination?.loadTurnItems === "function") return manager;
          } catch {}
        }
      }
    }
    return null;
  }

  function historyTurnRecoveryIslands(conversation) {
    const history = conversation?.turnHistory?.history;
    if (conversation?.turnHistory?.kind !== "canonical" || !Array.isArray(history?.islands)
        || !history.entitiesByKey || history.islands.length > 100) return null;
    const islands = [];
    const seen = new Set();
    for (const island of history.islands) {
      if (!Array.isArray(island?.entries)) return null;
      const turns = [];
      for (const entry of island.entries) {
        const turn = history.entitiesByKey[entry?.value];
        if (!turn || !normalizedCodexThreadUuid(turn.turnId) || seen.has(turn.turnId) || seen.size >= 2000
            || !Array.isArray(turn.items) || !Array.isArray(turn.params?.input)) return null;
        seen.add(turn.turnId);
        turns.push(turn);
      }
      islands.push(turns);
    }
    return islands;
  }

  function historyTurnRecoveryHasContent(turn) {
    return turn.items.length > 0 || turn.params.input.length > 0;
  }

  function historyTurnRecoveryBusy(conversation, islands) {
    return conversation.resumeState !== "resumed"
      || conversation.threadRuntimeStatus?.type === "active"
      || (conversation.unconfirmedTurnSubmissions != null && (!Array.isArray(conversation.unconfirmedTurnSubmissions)
        || conversation.unconfirmedTurnSubmissions.some((submission) => submission.terminal !== true)))
      || !Array.isArray(conversation.requests) || conversation.requests.length > 0
      || conversation.turnsPagination?.isLoadingOlder !== false
      || islands.some((turns) => turns.some((turn) => turn.status === "inProgress"
        || turn.itemsPagination?.isLoadingOlder !== false));
  }

  function historyTurnRecoveryCandidates(islands, threadId = "") {
    const result = [];
    for (const turns of islands) {
      let first = -1, last = -1;
      turns.forEach((turn, index) => {
        if (historyTurnRecoveryHasContent(turn)) {
          if (first < 0) first = index;
          last = index;
        }
      });
      for (let index = first + 1; first >= 0 && index < last; index += 1) {
        const turn = turns[index];
        const pending = historyTurnRecoveryLedger.attempts.get(`${threadId}/${turn.turnId}`);
        if ((!historyTurnRecoveryHasContent(turn) || (pending && !pending.done))
            && ["completed", "interrupted", "failed"].includes(turn.status)
            && turn.itemsPagination.hasLoadedOldest === false && turn.itemsPagination.evicted !== true
            && turn.itemsPagination.reconnect == null) result.push(turn);
      }
    }
    return result;
  }

  function historyTurnRecoveryNotice(kind) {
    if (historyTurnRecoveryLedger.notices.has(kind)) return;
    historyTurnRecoveryLedger.notices.add(kind);
    const zh = (document.documentElement?.lang || navigator.language || "").toLowerCase().startsWith("zh");
    const text = kind === "unsupported"
      ? zh ? "当前 Codex 的历史加载接口尚不兼容，空轮次补载已安全暂停。" : "History turn recovery is paused: this Codex version is not supported."
      : zh ? "部分历史轮次尚未补载成功，已停止自动重试；可关闭再开启补载开关后重试。" : "Some history turns could not be loaded. Automatic retries stopped; turn recovery off and on to retry.";
    showToast(text, { type: "warn" });
  }

  function scheduleHistoryTurnRecovery() {
    const state = historyTurnRecoveryState;
    const ledger = historyTurnRecoveryLedger;
    window.clearTimeout(state.timer);
    state.timer = null;
    if (!state.disposed && historyTurnRecoveryEnabled() && (ledger.api || ledger.discoveryAttempts < 3)) {
      state.timer = window.setTimeout(historyTurnRecoveryTick, historyTurnRecoveryPollMs);
    }
  }

  async function historyTurnRecoveryTick() {
    const state = historyTurnRecoveryState;
    const ledger = historyTurnRecoveryLedger;
    window.clearTimeout(state.timer);
    state.timer = null;
    if (state.disposed || !historyTurnRecoveryEnabled()) return;
    if (ledger.flight) {
      scheduleHistoryTurnRecovery();
      return;
    }
    ledger.flight = state;
    try {
      const target = historyTurnRecoveryTarget();
      if (!target) return;
      if (!ledger.api) {
        if (ledger.discoveryAttempts >= 3) return;
        if (ledger.discoveryAttempts && Date.now() - ledger.discoveryAt < historyTurnRecoveryRetryMs) return;
        ledger.discoveryAttempts += 1;
        ledger.discoveryAt = Date.now();
        ledger.api = await discoverHistoryTurnRecoveryApi().catch(() => null);
        if (!ledger.api) {
          if (!state.disposed && ledger.discoveryAttempts >= 3) historyTurnRecoveryNotice("unsupported");
          return;
        }
      }
      if (state.disposed || !historyTurnRecoveryEnabled() || historyTurnRecoveryTarget()?.id !== target.id) return;
      const manager = historyTurnRecoveryManager(ledger.api, target.marker);
      if (!manager) return;
      const conversation = manager.getConversation(target.id);
      const islands = historyTurnRecoveryIslands(conversation);
      if (!islands || historyTurnRecoveryBusy(conversation, islands)) return;
      // 每次开启最多 32 轮，每轮最多 4 页/2 次失败；切换会话不能重置预算。
      for (const turn of historyTurnRecoveryCandidates(islands, target.id)) {
        const key = `${target.id}/${turn.turnId}`;
        const attempt = ledger.attempts.get(key);
        if (attempt?.done || attempt?.count >= 4 || attempt?.failures >= 2) continue;
        if (attempt?.failed && Date.now() - attempt.at < historyTurnRecoveryRetryMs) continue;
        if (!attempt && ledger.attempts.size >= 32) {
          historyTurnRecoveryNotice("failed");
          return;
        }
        const next = { count: (attempt?.count || 0) + 1, failures: attempt?.failures || 0, failed: false, at: Date.now(), done: false };
        ledger.attempts.set(key, next);
        const previousCount = turn.items.length;
        const previousCursor = turn.itemsPagination.olderCursor;
        let rejected = false;
        try {
          // all=false 保证只读一页；不能让原生 all=true 在一次调用里无限追页。
          await manager.pagination.itemPagination.loadTurnItems(target.id, turn.turnId, false, {
            priority: "interactive", source: "thread_hydration",
          });
        } catch { rejected = true; }
        if (state.disposed || !historyTurnRecoveryEnabled() || historyTurnRecoveryTarget()?.id !== target.id) return;
        const updated = historyTurnRecoveryIslands(manager.getConversation(target.id));
        const loaded = updated?.flat().find((candidate) => candidate.turnId === turn.turnId);
        next.done = !!loaded && (loaded.params.input.length > 0 || loaded.itemsPagination?.hasLoadedOldest === true);
        next.failed = rejected || !loaded || (!next.done && loaded.items.length <= previousCount
          && loaded.itemsPagination?.olderCursor === previousCursor);
        if (next.failed) next.failures += 1;
        if (!next.done && (next.count >= 4 || next.failures >= 2)) historyTurnRecoveryNotice("failed");
        break;
      }
    } catch {
      // 不记录聊天内容、会话 ID 或原生异常正文。
    } finally {
      if (ledger.flight === state) ledger.flight = null;
      scheduleHistoryTurnRecovery();
    }
  }

  function syncCodexPlusHistoryTurnRecovery() {
    const state = historyTurnRecoveryState;
    const ledger = historyTurnRecoveryLedger;
    if (state.disposed) return;
    if (!historyTurnRecoveryEnabled()) {
      ledger.enabled = false;
      window.clearTimeout(state.timer);
      state.timer = null;
      return;
    }
    if (!ledger.enabled) {
      ledger.enabled = true;
      ledger.attempts.clear();
      ledger.notices.clear();
      ledger.discoveryAttempts = 0;
    }
    if (!state.timer && ledger.flight !== state) scheduleHistoryTurnRecovery();
  }

  window.__codexPlusHistoryTurnRecoveryRuntime = {
    ledger: historyTurnRecoveryLedger,
    dispose() {
      historyTurnRecoveryState.disposed = true;
      window.clearTimeout(historyTurnRecoveryState.timer);
    },
  };
  syncCodexPlusHistoryTurnRecovery();
