  // 原生 Fork 的放置检查（#2826）：getForkCapabilities().readPlacement 要先读两个门控
  // sidebarCustomSectionsEnabled / useAppServerCustomSections，任一为 undefined 就返回
  // not-ready / execution-config-loading。后者来自 Statsig，Windows 上不登录账号（纯 API Key）
  // 时 Statsig 客户端一直不就绪，Fork 就永远失败：弹窗关闭、不产生新会话、没有任何提示。
  //
  // 这两个门控只决定「新会话放进哪个自定义侧栏分组」，而自定义分组需要账号；
  // 所以只在没有账号时把读不到的值按原生缺省 false 处理，登录用户保持原生行为。
  const codexForkPlacementGateKeys = new Set(["sidebarCustomSectionsEnabled", "useAppServerCustomSections"]);
  const codexForkReadinessState = window.__codexPlusForkReadinessState
    || (window.__codexPlusForkReadinessState = { installing: null, patched: false, reported: false });

  function codexForkPlacementSettings(settings) {
    return new Proxy(settings, {
      get(target, property) {
        if (property === "read") {
          return (key, ...rest) => {
            const value = target.read(key, ...rest);
            if (value !== undefined || !codexForkPlacementGateKeys.has(key)) return value;
            if (!codexForkReadinessState.reported) {
              codexForkReadinessState.reported = true;
              try { sendCodexPlusDiagnostic("fork_placement_gate_fallback", { key }); } catch {}
            }
            return false;
          };
        }
        // 方法绑回原对象：私有状态、getter 都按原对象读取。
        const value = Reflect.get(target, property, target);
        return typeof value === "function" ? value.bind(target) : value;
      },
    });
  }

  function codexForkReadinessFallbackActive(manager) {
    if (typeof codexPlusBackendSettings !== "undefined" && codexPlusBackendSettings?.enhancementsEnabled === false) return false;
    try {
      if (typeof manager?.settings?.read !== "function") return false;
      return manager.runtime?.accountId?.() == null;
    } catch {
      return false;
    }
  }

  function patchCodexForkCapabilities(prototype) {
    const original = prototype?.getForkCapabilities;
    if (typeof original !== "function") return false;
    if (original.__codexPlusForkReadiness === true) return true;
    const patched = function getForkCapabilities(...args) {
      if (!codexForkReadinessFallbackActive(this)) return original.apply(this, args);
      // 不改实例字段：以原实例为原型，只把 settings 换成兜底视图，其余字段照常读取。
      const receiver = Object.create(this, { settings: { value: codexForkPlacementSettings(this.settings) } });
      return original.apply(receiver, args);
    };
    Object.defineProperty(patched, "__codexPlusForkReadiness", { value: true });
    Object.defineProperty(patched, "__codexPlusOriginal", { value: original });
    prototype.getForkCapabilities = patched;
    return true;
  }

  function installCodexForkReadinessFallback(attempt = 0) {
    if (codexForkReadinessState.patched || codexForkReadinessState.installing) return codexForkReadinessState.installing;
    codexForkReadinessState.installing = (async () => {
      const module = await loadOptionalCodexAppModule("app-initial-");
      let patched = false;
      for (const value of Object.values(module || {})) {
        if (typeof value !== "function" || typeof value.prototype?.getForkCapabilities !== "function") continue;
        patched = patchCodexForkCapabilities(value.prototype) || patched;
      }
      return patched;
    })().catch(() => false).then((patched) => {
      codexForkReadinessState.installing = null;
      codexForkReadinessState.patched = patched;
      // 模块可能还没加载完；有限次退避重试，找不到就静默放弃（原生行为不变）。
      if (!patched && attempt < 5) window.setTimeout(() => void installCodexForkReadinessFallback(attempt + 1), 2000 * (attempt + 1));
      return patched;
    });
    return codexForkReadinessState.installing;
  }

  // 启动时没联网（#2898）：Statsig 客户端 Ready 但一份远程数据都没有（NoValues），而且整个会话不再重拉。
  // 原生只在「有数据」时才把执行配置（execution assignments）发布给宿主，于是
  // readPermissionRefreshEnabled() 一直是 null，分支对话在发 thread/fork 之前就判 not-ready /
  // execution-config-loading——上面的放置门控兜底管不到这一处，启动时连着网才正常。
  //
  // 原生的 readPermissionRefreshEnabled 闭包在设置对象里，拿不到；改从源头补：没有账号且 Statsig 完全没数据时，
  // 补一份空数据（见 33-offline-auto-review.js 的 codexPlusOfflineStatsigPatch）。所有闸门仍是「没下发 = 关」，
  // 和没数据时读到的值一样，只是原生能据此发布执行配置（permissionRefresh = false，即旧版权限选择方式），
  // 等同于启动时连着网的纯 API Key 会话。只写内存，拿到真实数据就不碰，登录用户保持原生行为。
  //
  // Codex 26.1111（#3162）：Statsig 评估挪到了主进程。渲染进程把自己的 Statsig 数据发布过去，主进程据此
  // 建一个只读 Statsig 客户端；分支对话的放置门控在主进程读 useAppServerCustomSections =
  // 「这个客户端的 codex-app-server-custom-sections 闸门」，没有客户端就是 undefined，于是 thread/fork
  // 之前就判 not-ready / execution-config-loading。渲染进程只在数据确实属于当前用户时才发布数据，
  // 否则发布「无数据」，主进程按缺省给执行配置（所以 permissionRefresh 能读到 false），却不建客户端。
  // 除了上面的完全没数据，还有一种常见情形：Statsig 拉不到，但本地缓存里有以前拉到的数据——渲染进程
  // 照常用这份缓存，原生却因为 user 哈希对不上（每次启动都变的会话字段）不认它，上面「只在没数据时补」
  // 的兜底也不触发。这时原样沿用缓存数据，只补上当前 user（闸门值一个不改），原生就认作当前用户的数据，
  // 主进程随之建出客户端，放置门控读到真实的闸门值。
  // 上面的放置门控兜底（#2826）在 26.1106 起的 Rspack 版本里找不到 app-initial 模块、不会生效，
  // 那里渲染进程的放置门控本来就恒有值；它只对更早的 Codex 版本有用。
  function codexForkStatsigFallbackActive() {
    if (typeof codexPlusBackendSettingsLoaded === "undefined" || codexPlusBackendSettingsLoaded !== true) return false;
    return !(typeof codexPlusBackendSettings !== "undefined" && codexPlusBackendSettings?.enhancementsEnabled === false);
  }

  function codexForkStatsigWithoutAccount(client) {
    try {
      const user = client.getContext()?.user;
      return user != null && (user.customIDs?.account_id ?? null) == null;
    } catch {
      return false;
    }
  }

  function codexForkStatsigFallbackTick() {
    if (!codexForkStatsigFallbackActive()) return;
    for (const client of codexPlusOfflineStatsigClients()) {
      try {
        if (!codexForkStatsigWithoutAccount(client)) continue;
        const hadValues = client.getContext()?.values != null;
        if (!codexPlusOfflineStatsigPatch(client, {}, { adoptUnverified: true })) continue;
        if (!codexForkReadinessState.statsigReported) {
          codexForkReadinessState.statsigReported = true;
          // cache：沿用了原生不认的缓存数据（#3162）；empty：完全没数据，补了空数据（#2898）。
          try { sendCodexPlusDiagnostic("fork_statsig_offline_fallback", { source: hadValues ? "cache" : "empty" }); } catch {}
        }
      } catch {}
    }
  }

  function installCodexForkStatsigFallback() {
    // 重新注入会得到新作用域：摘掉旧脚本留下的定时器，只留一个。
    window.clearInterval(codexForkReadinessState.statsigTimer);
    codexForkStatsigFallbackTick();
    // Statsig 可能晚于注入才 Ready、后端设置也是异步读到的：低频复查，补过之后每轮都是空操作。
    codexForkReadinessState.statsigTimer = window.setInterval(codexForkStatsigFallbackTick, 3000);
  }
