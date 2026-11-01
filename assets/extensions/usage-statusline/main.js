  // 用量状态栏拓展的入口：拓展启用时开始观察输入框、轮询本会话的 token 事件，停用 / 热重载时摘掉状态栏。
  // 状态栏放进公开插槽 composer.below 给的容器；取数走清单里声明的 /thread-usage-live。
  // 算好的读数作为服务 codexpp.usage 提供：cached(threadId?) → { speed, cache, context, stale, modelContextWindow } | null。
  codexPlus.extension((ctx) => {
    codexPlusUsageHost.call = (route, payload) => ctx.call(route, payload);
    codexPlusUsageHost.diagnose = (event, detail) => ctx.call("/diagnostics/log", { event, detail: detail || {} }).catch(() => {});
    codexPlusUsageHost.legacySessionId = () => codexPlus.currentSession().id;

    ctx.registerSlot({
      slot: "composer.below",
      render: ({ container }) => {
        codexPlusUsageState.slotHosts.add(container);
        syncCodexPlusUsageStatusLine();
        return () => {
          codexPlusUsageState.slotHosts.delete(container);
          if (codexPlusUsageState.node?.parentElement === container) codexPlusUsageState.node.remove();
          syncCodexPlusUsageStatusLine();
        };
      },
    });
    ctx.effect(() => {
      // 旧版注入里的内置用量状态栏（拆包前）还在页面里时先停掉它（停了就不会再起），否则输入框下方会有两条。
      // 已安装的 Codex++ 与本地构建共用配置目录、或 Codex 还没按新版重新注入时会出现这种情况。
      window.__codexPlusUsageStatusLineRuntime?.dispose?.();
      startCodexPlusUsageStatusLine();
      return () => stopCodexPlusUsageStatusLine();
    });
    ctx.provide("codexpp.usage", Object.freeze({
      cached: (threadId) => codexPlusUsageSnapshot(codexPlusUsageState.cache.get(String(threadId || codexPlus.currentSessionId()))),
    }));
  });
