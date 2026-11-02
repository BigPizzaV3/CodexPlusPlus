  // 请求状态条拓展的入口：拓展启用时每 1.2 秒问一次 Codex++ 进行中的代理请求，停用 / 热重载时摘掉状态条。
  // 状态条放进公开插槽 composer.above 给的容器；取数走清单里声明的 /proxy-status。
  // 轮询到的数据作为服务 codexpp.proxyStatus 提供：requests() 最近一次的结果，fetch() 立即问一次。
  codexPlus.extension((ctx) => {
    codexPlusProxyStatusHost.call = (route, payload, options) => ctx.call(route, payload, options);

    ctx.registerSlot({
      slot: "composer.above",
      render: ({ container }) => {
        codexPlusProxyStatusState.slotHosts.add(container);
        void codexPlusProxyStatusRefresh();
        return () => {
          codexPlusProxyStatusState.slotHosts.delete(container);
          if (codexPlusProxyStatusState.node?.parentElement === container) codexPlusProxyStatusState.node.remove();
        };
      },
    });
    ctx.effect(() => {
      // 旧版注入里的内置请求状态条（拆包前）还在页面里时先停掉它（停了就不会再起），否则会有两条、轮询两份。
      window.__codexPlusProxyRequestStatusRuntime?.dispose?.();
      startCodexPlusProxyStatus();
      return () => stopCodexPlusProxyStatus();
    });
    ctx.provide("codexpp.proxyStatus", Object.freeze({
      requests: () => [...codexPlusProxyStatusState.requests],
      fetch: () => codexPlusProxyStatusFetch().catch(() => []),
    }));
  });
