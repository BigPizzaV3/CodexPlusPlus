  /**
   * 内置拓展「Statsig 界面增强」。
   *
   * Codex 界面里的不少行为由 Statsig 远程开关决定（升级推销、网络/流式超时、记忆开关、
   * 消息队列、时间线、侧聊 attach 等）。这一层只补「纯客户端」的几项；服务端校验的能力
   * （云端任务、真实配额）不受影响，也改不动。
   *
   * 做法：包一层 getFeatureGate / checkGate / getExperiment / getLayer / getDynamicConfig，
   * 只在开关打开时按配置覆盖返回值。注意 Statsig 的真实实现是
   * `checkGate(name) { return this.getFeatureGate(name).value }`，所以只包 checkGate 会漏掉
   * 页面里直接读 `getFeatureGate(...).value` 的执行配置；四个读口都要包。
   *
   * 配置由 Rust 侧经 window.__CODEX_PLUS_STATSIG_OVERRIDES__ 下发，关闭开关即恢复。
   *
   *   gates:          { "<gate id>": true }
   *   experiments:    { "<experiment id>": { "<key>": value } }
   *   layers:         { "<layer id>": { "<key>": value } }
   *   dynamicConfigs: { "<config id>": { "<key>": value } }
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

  function codexPlusStatsigOverrideSection(name) {
    const section = codexPlusStatsigOverridesConfig?.[name];
    return section && typeof section === "object" ? section : {};
  }

  function codexPlusStatsigGateOverrides() {
    return codexPlusStatsigOverrideSection("gates");
  }

  function codexPlusStatsigExperimentOverrides() {
    return codexPlusStatsigOverrideSection("experiments");
  }

  function codexPlusStatsigLayerOverrides() {
    return codexPlusStatsigOverrideSection("layers");
  }

  function codexPlusStatsigDynamicConfigOverrides() {
    return codexPlusStatsigOverrideSection("dynamicConfigs");
  }

  /** 把覆盖值合进 dynamic config / experiment / layer 的 value 与 get()，返回同一个对象。 */
  function codexPlusStatsigMergeObjectValue(target, overrides) {
    if (!target || typeof target !== "object" || !overrides || typeof overrides !== "object") return target;
    const value = target.value && typeof target.value === "object" && !Array.isArray(target.value)
      ? target.value
      : target.__value && typeof target.__value === "object" && !Array.isArray(target.__value)
        ? target.__value
        : null;
    if (!value) return target;
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
      target.value = next;
    } catch {
      return target;
    }
    // Layer 用 __value 存原始参数；experiment 的 __evaluation.value 也可能被调用方直接读。
    try {
      if ("__value" in target) target.__value = next;
      if (target.__evaluation && typeof target.__evaluation === "object") {
        target.__evaluation = {
          ...target.__evaluation,
          value: next,
          is_user_in_experiment: true,
          is_experiment_active: true,
        };
      }
    } catch {}
    if (typeof target.get === "function" && !target.__codexPlusStatsigMerged) {
      const originalGet = target.get.bind(target);
      target.get = (key, fallback) => (key in next ? next[key] : originalGet(key, fallback));
      target.__codexPlusStatsigMerged = true;
    }
    return target;
  }

  function codexPlusStatsigPatchClient(client) {
    if (!client || typeof client !== "object") return false;
    let changed = false;

    if (typeof client.getFeatureGate === "function" && !client.__codexPlusStatsigFeatureGatePatched) {
      const originalGetFeatureGate = client.getFeatureGate.bind(client);
      client.getFeatureGate = (gateName, options) => {
        const result = originalGetFeatureGate(gateName, options);
        if (!codexPlusStatsigOverridesEnabled()) return result;
        const overrides = codexPlusStatsigGateOverrides();
        const key = String(gateName);
        if (!Object.prototype.hasOwnProperty.call(overrides, key)) return result;
        // getFeatureGate 是 memoized，同一个对象会被界面和 SDK 共享，不能就地改 value。
        return { ...result, value: overrides[key] === true };
      };
      client.__codexPlusStatsigFeatureGatePatched = true;
      changed = true;
    }

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
          ? codexPlusStatsigMergeObjectValue(result, overrides[key])
          : result;
      };
      client.__codexPlusStatsigConfigPatched = true;
      changed = true;
    }

    if (typeof client.getExperiment === "function" && !client.__codexPlusStatsigExperimentPatched) {
      const originalGetExperiment = client.getExperiment.bind(client);
      client.getExperiment = (name, options) => {
        const result = originalGetExperiment(name, options);
        if (!codexPlusStatsigOverridesEnabled()) return result;
        const overrides = codexPlusStatsigExperimentOverrides();
        const key = String(name);
        return Object.prototype.hasOwnProperty.call(overrides, key)
          ? codexPlusStatsigMergeObjectValue(result, overrides[key])
          : result;
      };
      client.__codexPlusStatsigExperimentPatched = true;
      changed = true;
    }

    if (typeof client.getLayer === "function" && !client.__codexPlusStatsigLayerPatched) {
      const originalGetLayer = client.getLayer.bind(client);
      client.getLayer = (name, options) => {
        const result = originalGetLayer(name, options);
        if (!codexPlusStatsigOverridesEnabled()) return result;
        const overrides = codexPlusStatsigLayerOverrides();
        const key = String(name);
        return Object.prototype.hasOwnProperty.call(overrides, key)
          ? codexPlusStatsigMergeObjectValue(result, overrides[key])
          : result;
      };
      client.__codexPlusStatsigLayerPatched = true;
      changed = true;
    }

    // 已经取过的对象可能被界面持有引用：装上开关后先就地合并一次。
    if (changed && codexPlusStatsigOverridesEnabled()) {
      const mergeAlreadyFetched = (method, overrides) => {
        if (typeof client[method] !== "function") return;
        Object.entries(overrides).forEach(([name, entries]) => {
          try {
            codexPlusStatsigMergeObjectValue(client[method](name, { disableExposureLog: true }), entries);
          } catch {
            // 远端还没下发这条配置时拿不到对象，等下一次调用再合并。
          }
        });
      };
      mergeAlreadyFetched("getDynamicConfig", codexPlusStatsigDynamicConfigOverrides());
      mergeAlreadyFetched("getExperiment", codexPlusStatsigExperimentOverrides());
      mergeAlreadyFetched("getLayer", codexPlusStatsigLayerOverrides());
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
