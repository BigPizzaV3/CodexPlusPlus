  /**
   * 内置拓展「Statsig 界面增强」。
   *
   * Codex 界面里的不少行为由 Statsig 远程开关决定（升级推销、网络/流式超时、记忆开关）。
   * 这一层只补「纯客户端」的几项：强制隐藏升级与额度推销、放宽弱网超时、补开记忆的
   * Statsig 侧开关。服务端校验的能力（云端任务、真实配额）不受影响，也改不动。
   *
   * 做法：包一层 checkGate / getDynamicConfig，只在开关打开时按配置覆盖返回值；
   * 配置由 Rust 侧经 window.__CODEX_PLUS_STATSIG_OVERRIDES__ 下发，关闭开关即恢复。
   */
  const codexPlusStatsigOverridesConfig = window.__CODEX_PLUS_STATSIG_OVERRIDES__ || null;

  function codexPlusStatsigOverridesEnabled() {
    return (
      codexPlusBackendSettingsLoaded === true
      && codexPlusSettings().statsigOverrides === true
      && codexPlusStatsigOverridesConfig?.enabled === true
    );
  }

  function codexPlusStatsigOverrideClients() {
    const root = window.__STATSIG__ || globalThis.__STATSIG__;
    if (!root || typeof root !== "object") return [];
    const clients = [root.firstInstance, typeof root.instance === "function" ? root.instance() : null];
    if (root.instances && typeof root.instances === "object") clients.push(...Object.values(root.instances));
    return clients.filter((client, index, array) => client && typeof client === "object" && array.indexOf(client) === index);
  }

  function codexPlusStatsigGateOverrides() {
    const gates = codexPlusStatsigOverridesConfig?.gates;
    return gates && typeof gates === "object" ? gates : {};
  }

  function codexPlusStatsigDynamicConfigOverrides() {
    const configs = codexPlusStatsigOverridesConfig?.dynamicConfigs;
    return configs && typeof configs === "object" ? configs : {};
  }

  /** 把覆盖值合进 dynamic config，返回同一个对象（调用方通常持有引用）。 */
  function codexPlusStatsigApplyDynamicConfig(config, overrides) {
    if (!config || typeof config !== "object" || !overrides || typeof overrides !== "object") return config;
    const value = config.value && typeof config.value === "object" ? config.value : null;
    if (!value) return config;
    const next = { ...value };
    Object.entries(overrides).forEach(([key, override]) => {
      const current = next[key];
      next[key] =
        override && typeof override === "object" && !Array.isArray(override)
          && current && typeof current === "object" && !Array.isArray(current)
          ? { ...current, ...override }
          : override;
    });
    try {
      config.value = next;
    } catch {
      return config;
    }
    if (typeof config.get === "function" && !config.__codexPlusStatsigMerged) {
      const originalGet = config.get.bind(config);
      config.get = (key, fallback) => (key in next ? next[key] : originalGet(key, fallback));
      config.__codexPlusStatsigMerged = true;
    }
    return config;
  }

  function codexPlusStatsigPatchClient(client) {
    if (!client || typeof client !== "object") return false;
    let changed = false;

    if (typeof client.checkGate === "function" && !client.__codexPlusStatsigGatePatched) {
      const originalCheckGate = client.checkGate.bind(client);
      client.checkGate = (gateName, options) => {
        if (codexPlusStatsigOverridesEnabled()) {
          const overrides = codexPlusStatsigGateOverrides();
          const key = String(gateName);
          if (Object.prototype.hasOwnProperty.call(overrides, key)) return overrides[key] === true;
        }
        return originalCheckGate(gateName, options);
      };
      client.__codexPlusStatsigGatePatched = true;
      changed = true;
    }

    if (typeof client.getDynamicConfig === "function" && !client.__codexPlusStatsigConfigPatched) {
      const originalGetDynamicConfig = client.getDynamicConfig.bind(client);
      client.getDynamicConfig = (name, options) => {
        const result = originalGetDynamicConfig(name, options);
        if (!codexPlusStatsigOverridesEnabled()) return result;
        const overrides = codexPlusStatsigDynamicConfigOverrides();
        const key = String(name);
        return Object.prototype.hasOwnProperty.call(overrides, key)
          ? codexPlusStatsigApplyDynamicConfig(result, overrides[key])
          : result;
      };
      client.__codexPlusStatsigConfigPatched = true;
      changed = true;
    }

    // 已经取过的 config 对象可能被界面持有引用：装上开关后先就地合并一次。
    if (changed && codexPlusStatsigOverridesEnabled() && typeof client.getDynamicConfig === "function") {
      Object.entries(codexPlusStatsigDynamicConfigOverrides()).forEach(([name, overrides]) => {
        try {
          codexPlusStatsigApplyDynamicConfig(
            client.getDynamicConfig(name, { disableExposureLog: true }),
            overrides,
          );
        } catch {
          // 远端还没下发这条配置时拿不到对象，等下一次调用再合并。
        }
      });
    }

    return changed;
  }

  function codexPlusStatsigOverridesTick() {
    if (!codexPlusStatsigOverridesEnabled()) return;
    for (const client of codexPlusStatsigOverrideClients()) {
      try {
        if (codexPlusStatsigPatchClient(client)) {
          console.log("[StatsigOverrides]", "已接管 Statsig 开关覆盖");
        }
      } catch (error) {
        console.warn("[StatsigOverrides]", "覆盖失败：", error && error.message);
      }
    }
  }

  function codexPlusStatsigOverridesExtension(ctx) {
    codexPlusStatsigOverridesTick();
    // 后端设置与 Statsig 客户端都可能晚于注入就绪：低频复查，已经接管过时是空操作。
    ctx.setInterval(codexPlusStatsigOverridesTick, 3000);
  }
