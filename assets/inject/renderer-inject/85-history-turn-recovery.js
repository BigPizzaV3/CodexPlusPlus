  // 补齐已经登记的终态轮次；非空或已有用户输入不代表消息分页完整。
  const previousHistoryTurnRecovery = window.__codexPlusHistoryTurnRecoveryRuntime;
  previousHistoryTurnRecovery?.dispose?.();
  // 仅由原有会话索引检查完成后调用；重注入保留去重记录与正在进行的原生读取。
  const historyTurnRecoveryLedger = previousHistoryTurnRecovery?.ledger || {
    flight: null, api: null,
  };
  historyTurnRecoveryLedger.requests ||= new Set();
  const historyTurnRecoveryState = { disposed: false };

  function historyTurnRecoveryEnabled() {
    return codexPlusBackendSettingsLoaded && codexPlusBackendSettings.enhancementsEnabled !== false
      && codexPlusBackendSettings.codexAppHistoryTurnRecovery === true;
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
        || !history.entitiesByKey || !Number.isSafeInteger(history.generation) || history.generation < 0
        || history.islands.length > 100) return null;
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

  function historyTurnRecoveryBusy(conversation, islands) {
    return conversation.resumeState !== "resumed"
      || conversation.threadRuntimeStatus?.type === "active"
      || (conversation.unconfirmedTurnSubmissions != null && (!Array.isArray(conversation.unconfirmedTurnSubmissions)
        || conversation.unconfirmedTurnSubmissions.length > 0))
      || !Array.isArray(conversation.requests) || conversation.requests.length > 0
      || conversation.turnsPagination?.isLoadingOlder !== false
      || islands.some((turns) => turns.some((turn) => turn.status === "inProgress"
        || turn.itemsPagination?.isLoadingOlder !== false));
  }

  function historyTurnRecoveryCandidates(islands) {
    // 不跨页发现新轮次，也不复活被原生内存策略驱逐的内容；连续非空缺页同样要补。
    return islands.flat().filter((turn) => ["completed", "interrupted", "failed"].includes(turn.status)
      && turn.itemsPagination.hasLoadedOldest === false && turn.itemsPagination.evicted !== true
      && turn.itemsPagination.reconnect == null);
  }

  function historyTurnRecoveryNeedsMetadata(turn) {
    return ["completed", "interrupted", "failed"].includes(turn.status)
      && (turn.durationMs == null || turn.turnStartedAtMs == null || turn.finalAssistantStartedAtMs == null);
  }

  async function recoverHistoryTurnMetadata(manager, target, ids, current, result, deadline) {
    const initial = current();
    const pending = new Set(initial?.islands.flat().filter((turn) => ids.has(turn.turnId)
      && historyTurnRecoveryNeedsMetadata(turn)).map((turn) => turn.turnId));
    if (!pending.size) return true;
    // 原生单页 item loader 不更新这些字段。只查 metadata，不整体 hydrate/rebuild islands。
    const writable = (snapshot) => snapshot && manager.getStreamRole?.(target.id)?.role === "owner"
      && (snapshot.conversation.unconfirmedTurnSubmissions == null
        || snapshot.conversation.unconfirmedTurnSubmissions.length === 0);
    if (typeof manager.sendRequest !== "function" || typeof manager.updateConversationState !== "function"
        || typeof manager.broadcastConversationSnapshot !== "function" || !writable(initial)) return false;
    let cursor = null;
    const seen = new Set();
    try {
      do {
        if (!writable(current()) || Date.now() >= deadline || result.metadataPages >= 64
            || result.metadataPages + result.loadedPages >= 128 || seen.has(cursor)) return false;
        seen.add(cursor);
        result.metadataPages += 1;
        const page = await manager.sendRequest("thread/turns/list", {
          threadId: target.id, cursor, limit: 32, sortDirection: "desc", itemsView: "notLoaded",
        }, { priority: "interactive", source: "thread_hydration" });
        if (!writable(current()) || !Array.isArray(page?.data) || page.data.length > 32
            || (page.nextCursor != null && typeof page.nextCursor !== "string")) return false;
        const rows = new Map();
        for (const row of page.data) {
          if (!normalizedCodexThreadUuid(row?.id) || rows.has(row.id)) return false;
          rows.set(row.id, row);
        }
        let changed = false;
        manager.updateConversationState(target.id, (draft) => {
          // setter 本身不检查 disposed；且原生会处理未确认提交，不能与它争用。
          if (!writable(current()) || draft.turnHistory?.history?.generation
              !== initial.conversation.turnHistory.history.generation
              || (draft.unconfirmedTurnSubmissions?.length || 0) > 0) return;
          const islands = historyTurnRecoveryIslands(draft);
          if (!islands || historyTurnRecoveryBusy(draft, islands)) return;
          for (const turn of islands.flat()) {
            const row = rows.get(turn.turnId);
            if (!pending.has(turn.turnId) || !row || row.status !== turn.status) continue;
            pending.delete(turn.turnId);
            // 与原生 L8t/B8t 一致；只填真实缺值，不覆盖现有 0、状态或消息。
            const values = {
              durationMs: row.durationMs,
              turnStartedAtMs: typeof row.startedAt === "number" ? row.startedAt * 1000 : null,
              finalAssistantStartedAtMs: typeof row.completedAt === "number" ? row.completedAt * 1000 : null,
            };
            for (const [key, value] of Object.entries(values)) {
              if (turn[key] == null && typeof value === "number" && Number.isFinite(value) && value >= 0) {
                turn[key] = value;
                changed = true;
              }
            }
          }
        }, false);
        if (changed && writable(current())) manager.broadcastConversationSnapshot(target.id);
        cursor = page.nextCursor ?? null;
      } while (pending.size && cursor != null);
      const snapshot = current();
      return !!snapshot && !pending.size && !snapshot.islands.flat().some((turn) => ids.has(turn.turnId)
        && historyTurnRecoveryNeedsMetadata(turn));
    } catch {
      // 不记录原生异常正文；元数据阶段失败不阻止仍可安全进行的消息分页。
      return false;
    }
  }

  async function runHistoryTurnRecovery(requestId) {
    const state = historyTurnRecoveryState;
    const ledger = historyTurnRecoveryLedger;
    const result = { status: "skipped", reason: "context_changed", loadedPages: 0, metadataPages: 0 };
    if (state.disposed || !historyTurnRecoveryEnabled()
        || !normalizedCodexThreadUuid(requestId) || ledger.requests.has(requestId)) {
      return { ...result, reason: state.disposed ? "disposed" : !historyTurnRecoveryEnabled() ? "disabled" : "duplicate_request" };
    }
    ledger.requests.add(requestId);
    if (ledger.requests.size > 64) ledger.requests.delete(ledger.requests.values().next().value);
    if (ledger.flight) return { ...result, status: "busy", reason: "recovery_busy" };
    ledger.flight = state;
    const deadline = Date.now() + 12000;
    try {
      const target = historyTurnRecoveryTarget();
      if (!target) return { ...result, reason: document.hidden ? "window_hidden"
        : /[?&]initialRoute=/.test(window.location.search) ? "unsupported_window" : "no_local_thread" };
      if (!ledger.api) {
        ledger.api = await discoverHistoryTurnRecoveryApi().catch(() => null);
        if (!ledger.api) return { ...result, status: "unavailable", reason: "native_api_unavailable" };
      }
      if (state.disposed || !historyTurnRecoveryEnabled()) return result;
      const owner = historyTurnRecoveryManager(ledger.api, target.marker);
      const initial = owner?.getConversation(target.id);
      const initialIslands = historyTurnRecoveryIslands(initial);
      if (!initialIslands) return { ...result, status: "unavailable", reason: "native_state_unavailable" };
      if (historyTurnRecoveryBusy(initial, initialIslands)) return { ...result, reason: "thread_busy" };
      const generation = initial.turnHistory.history.generation;
      const current = () => {
        if (state.disposed || !historyTurnRecoveryEnabled()) return null;
        const mounted = historyTurnRecoveryTarget();
        if (state.disposed || owner.disposed === true || mounted?.id !== target.id
            || historyTurnRecoveryManager(ledger.api, mounted.marker) !== owner
            || owner.getStreamRole?.(target.id)?.role === "follower") return null;
        const conversation = owner.getConversation(target.id);
        const islands = historyTurnRecoveryIslands(conversation);
        return islands && conversation.turnHistory.history.generation === generation
          && !historyTurnRecoveryBusy(conversation, islands) ? { conversation, islands } : null;
      };
      if (!current()) return result;
      const eligible = [...new Set([
        ...historyTurnRecoveryCandidates(initialIslands).map((turn) => turn.turnId),
        ...initialIslands.flat().filter(historyTurnRecoveryNeedsMetadata).map((turn) => turn.turnId),
      ])];
      const ids = new Set(eligible.slice(0, 32));
      // 没有常驻扫描或延迟重试；消息与元数据共享 128 页预算。
      while (!state.disposed) {
        const snapshot = current();
        if (!snapshot) return result;
        const turn = historyTurnRecoveryCandidates(snapshot.islands).find((turn) => ids.has(turn.turnId));
        if (!turn) {
          // 先补正文/文件；慢元数据查询不能耗尽预算后让消息补载一次都无法开始。
          const metadataComplete = await recoverHistoryTurnMetadata(owner, target, ids, current, result, deadline);
          if (result.metadataPages) result.status = "partial";
          const updated = current();
          if (!updated) return result;
          const itemsComplete = !historyTurnRecoveryCandidates(updated.islands).some((turn) => ids.has(turn.turnId));
          const complete = metadataComplete && itemsComplete && eligible.length <= 32;
          return { ...result, status: complete ? "completed" : "partial", reason: complete ? null : "incomplete" };
        }
        if (Date.now() >= deadline || result.loadedPages + result.metadataPages >= 128) return { ...result, status: "partial", reason: "budget_exhausted" };
        const previousCount = turn.items.length;
        const previousCursor = JSON.stringify(turn.itemsPagination.olderCursor);
        result.status = "partial";
        let rejected = false;
        try {
          // all=false 保证只读一页；不能让原生 all=true 在一次调用里无限追页。
          await owner.pagination.itemPagination.loadTurnItems(target.id, turn.turnId, false, {
            priority: "interactive", source: "thread_hydration",
          });
        } catch { rejected = true; }
        if (!rejected) result.loadedPages += 1;
        const updated = current();
        if (!updated) return result;
        const loaded = updated.islands.flat().find((candidate) => candidate.turnId === turn.turnId);
        const done = loaded?.itemsPagination?.hasLoadedOldest === true;
        const failed = rejected || !loaded || (!done && loaded.items.length <= previousCount
          && JSON.stringify(loaded.itemsPagination?.olderCursor) === previousCursor);
        if (failed) return { ...result, status: "partial", reason: "native_read_failed" };
      }
      return { ...result, status: "partial" };
    } catch {
      // 不记录聊天内容、会话 ID 或原生异常正文。
      return { ...result, status: "unavailable", reason: "native_api_unavailable" };
    } finally {
      if (ledger.flight === state) ledger.flight = null;
    }
  }

  window.__codexPlusHistoryTurnRecoveryRuntime = {
    ledger: historyTurnRecoveryLedger,
    run: runHistoryTurnRecovery,
    dispose() {
      historyTurnRecoveryState.disposed = true;
    },
  };
