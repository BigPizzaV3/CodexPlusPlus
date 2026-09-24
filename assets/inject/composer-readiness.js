(function () {
  function isLocalHomeQuery(queryKey) {
    if (!Array.isArray(queryKey) || queryKey[0] !== "vscode" || queryKey[1] !== "codex-home") return false;
    if (queryKey.length === 2) return true;
    if (queryKey.length !== 3 || typeof queryKey[2] !== "string") return false;
    try {
      const params = JSON.parse(queryKey[2]);
      return !!params && !Array.isArray(params) && typeof params === "object"
        && (params.hostId == null || params.hostId === "local");
    } catch {
      return false;
    }
  }

  function createRecovery({ now = Date.now } = {}) {
    let states = new WeakMap();
    let generation = 0;
    const staleAfterMs = 10000;
    const maxAttempts = 2;

    return {
      tick(client, enabled) {
        if (!enabled) {
          states = new WeakMap();
          generation += 1;
          return 0;
        }
        if (typeof client?.cancelQueries !== "function" || typeof client?.refetchQueries !== "function") return 0;
        const queries = client.getQueryCache?.()?.getAll?.() || [];
        // 原生工作目录检查会等 codex-home；仅恢复这条已确认阻塞输入框的本地只读请求。
        const blocked = queries.some((query) => {
          const key = query.queryKey;
          return Array.isArray(key)
          && key[0] === "git" && key[1] === "local"
          && key[2] === "managed-worktree-state" && key[3] === "codex-home-loading"
          && query.getObserversCount?.() > 0;
        });
        if (!blocked) return 0;
        let attemptsStarted = 0;
        for (const query of queries) {
          if (!isLocalHomeQuery(query.queryKey)) continue;
          let state = states.get(query);
          if (state?.cancelling) continue;
          if (query.state?.data != null || query.state?.status !== "pending") {
            states.delete(query);
            continue;
          }
          if (query.state?.fetchStatus !== "fetching" || query.isActive?.() !== true) continue;
          if (!state) {
            state = { since: now(), attempts: 0, cancelling: false };
            states.set(query, state);
          }
          if (state.cancelling || state.attempts >= maxAttempts || now() - state.since < staleAfterMs) continue;
          state.attempts += 1;
          state.since = now();
          state.cancelling = true;
          attemptsStarted += 1;
          const currentGeneration = generation;
          const filter = { queryKey: query.queryKey, exact: true };
          // invalidate/refetch 会复用原来的 pending Promise；必须先取消，才能真正重新投递。
          // 不写入假目录或 available 状态，让原生检查根据真实响应自行放行。
          Promise.resolve().then(async () => {
            if (generation !== currentGeneration) return;
            await client.cancelQueries(filter);
            // 一旦取消就完成重读，即使期间切换模式，也不能留下被我们取消的请求。
            const request = client.refetchQueries(filter);
            // 新请求也可能丢回复；不等待它，后续心跳最多再恢复一次。
            Promise.resolve(request).catch(() => {});
          }).catch(() => {}).finally(() => { state.cancelling = false; });
        }
        return attemptsStarted;
      },
    };
  }

  if (typeof module === "object" && module.exports) module.exports = { createRecovery };
  if (typeof window !== "undefined") window.__codexPlusComposerReadiness ||= createRecovery();
})();
