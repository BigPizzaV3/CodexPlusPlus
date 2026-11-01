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
