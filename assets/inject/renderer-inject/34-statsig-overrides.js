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
   *   gateCatalog:    [{ id, title, description, defaultOverride, risk, note? }]（Feature Lab 用）
   *
   * 逐项覆盖：后端设置 codexAppStatsigGateOverrides = { "<gate id>": "on" | "off" | "native" }，
   * 叠加在 gates 上（native = 不覆盖）。它从后端设置实时读，Feature Lab 改了不用重新注入。
   *
   * 通知：Codex 渲染进程只在 Statsig 派发 values_updated 时，把执行配置（权限刷新、时间线、
   * 排队追问、git 上下文、统一项目等）重新发布给主进程；主进程和读主进程副本的界面都以那份为准。
   * 接管往往晚于首次发布，所以接管后、以及生效覆盖变化时（含关掉总开关），要补派发一次
   * values_updated，否则这些开关只在渲染进程里变、主进程仍用远端值。Codex 只在值真的变了
   * 才重新发布，没变时派发是空操作。个别只在挂载时读一次的界面仍要重载页面。
   *
   * 记录：Feature Lab 打开「记录」后，即使总开关关着也会包一层（只透传），把页面查询过的
   * 开关 ID 与原值记下来，方便找新开关。
   */
  const codexPlusStatsigOverridesConfig = window.__CODEX_PLUS_STATSIG_OVERRIDES__ || null;
  const codexPlusStatsigGateIdPattern = /^\d{1,12}$/;
  const codexPlusStatsigGateChoices = new Set(["on", "off", "native"]);
  /** 页面查询过的开关：id → { id, native, count, lastAt }。挂在 window 上，重注入不丢。 */
  const codexPlusStatsigGateLog = window.__codexPlusStatsigGateLog instanceof Map
    ? window.__codexPlusStatsigGateLog
    : (window.__codexPlusStatsigGateLog = new Map());
  const codexPlusStatsigGateLogLimit = 500;

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

  /** 已知开关清单（Rust 侧从 assets/statsig-gates.json 下发）。 */
  function codexPlusStatsigGateCatalog() {
    const catalog = codexPlusStatsigOverridesConfig?.gateCatalog;
    return Array.isArray(catalog)
      ? catalog.filter((entry) => entry && typeof entry === "object" && codexPlusStatsigGateIdPattern.test(String(entry.id || "")))
      : [];
  }

  /** 后端设置里的逐项覆盖，只认「纯数字 ID → on/off/native」。 */
  function codexPlusStatsigUserGateOverrides() {
    const settings = typeof codexPlusBackendSettings === "object" && codexPlusBackendSettings ? codexPlusBackendSettings : null;
    const raw = settings?.codexAppStatsigGateOverrides;
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
    const result = {};
    for (const [id, choice] of Object.entries(raw)) {
      if (codexPlusStatsigGateIdPattern.test(id) && codexPlusStatsigGateChoices.has(choice)) result[id] = choice;
    }
    return result;
  }

  /** 生效的闸门覆盖：默认 gates 叠加逐项覆盖。on → true，off → false，native → 不覆盖。 */
  function codexPlusStatsigEffectiveGates(defaults, user) {
    const effective = { ...(defaults && typeof defaults === "object" ? defaults : {}) };
    for (const [id, choice] of Object.entries(user || {})) {
      if (choice === "on") effective[id] = true;
      else if (choice === "off") effective[id] = false;
      else if (choice === "native") delete effective[id];
    }
    return effective;
  }

  function codexPlusStatsigGateOverrides() {
    return codexPlusStatsigEffectiveGates(codexPlusStatsigOverrideSection("gates"), codexPlusStatsigUserGateOverrides());
  }

  function codexPlusStatsigRecording() {
    return window.__codexPlusStatsigRecording === true;
  }

  /** 记录一次开关查询（原值）。只在记录打开时写，超上限丢最早的。 */
  function codexPlusStatsigRecordGate(name, nativeValue) {
    if (!codexPlusStatsigRecording()) return;
    const id = String(name);
    if (!codexPlusStatsigGateIdPattern.test(id)) return;
    const entry = codexPlusStatsigGateLog.get(id) || { id, native: null, count: 0, lastAt: 0 };
    entry.native = typeof nativeValue === "boolean" ? nativeValue : entry.native;
    entry.count += 1;
    entry.lastAt = Date.now();
    codexPlusStatsigGateLog.delete(id);
    codexPlusStatsigGateLog.set(id, entry);
    while (codexPlusStatsigGateLog.size > codexPlusStatsigGateLogLimit) {
      codexPlusStatsigGateLog.delete(codexPlusStatsigGateLog.keys().next().value);
    }
  }

  /** 读开关的远端原值（绕过覆盖，不打曝光日志）。拿不到客户端时返回 null。 */
  function codexPlusStatsigNativeGateValue(name) {
    for (const client of codexPlusStatsigOverrideClients()) {
      const read = typeof client.__codexPlusStatsigOriginalGetFeatureGate === "function"
        ? client.__codexPlusStatsigOriginalGetFeatureGate
        : typeof client.getFeatureGate === "function" ? client.getFeatureGate.bind(client) : null;
      if (!read) continue;
      try {
        const result = read(String(name), { disableExposureLog: true });
        if (result && typeof result === "object" && typeof result.value === "boolean") return result.value;
        if (typeof result === "boolean") return result;
      } catch {}
    }
    return null;
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
      client.__codexPlusStatsigOriginalGetFeatureGate = originalGetFeatureGate;
      client.getFeatureGate = (gateName, options) => {
        const result = originalGetFeatureGate(gateName, options);
        codexPlusStatsigRecordGate(gateName, result && typeof result === "object" ? result.value : undefined);
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
      // 不在这里记录：Statsig 的 checkGate 内部就是调 getFeatureGate，记录已在那一层发生。
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

  /** 当前生效的覆盖签名；总开关关着时为空串（= 远端原值）。 */
  function codexPlusStatsigOverridesSignature() {
    if (!codexPlusStatsigOverridesEnabled()) return "";
    const sorted = (object) => Object.keys(object).sort().map((key) => [key, object[key]]);
    return JSON.stringify([
      sorted(codexPlusStatsigGateOverrides()),
      sorted(codexPlusStatsigDynamicConfigOverrides()),
      sorted(codexPlusStatsigExperimentOverrides()),
      sorted(codexPlusStatsigLayerOverrides()),
    ]);
  }

  /**
   * 覆盖变了就补派发 values_updated，让 Codex 重读开关并把执行配置重新发布给主进程。
   * 客户端还没 Ready 时不派发：它就绪时 SDK 自己会派发，那时包装已经在了。
   */
  function codexPlusStatsigNotifyIfChanged(client) {
    if (!client.__codexPlusStatsigFeatureGatePatched) return false;
    const signature = codexPlusStatsigOverridesSignature();
    const notified = typeof client.__codexPlusStatsigNotifiedSignature === "string"
      ? client.__codexPlusStatsigNotifiedSignature
      : "";
    if (signature === notified) return false;
    if (client.loadingStatus !== "Ready" || typeof client.$emt !== "function") return false;
    // 与 SDK 自己的 _setStatus 一致：先清 memo，再派发。
    try {
      if (client._memoCache && typeof client._memoCache === "object") client._memoCache = {};
    } catch {}
    let values;
    try {
      values = typeof client.getContext === "function" ? client.getContext()?.values : undefined;
    } catch {}
    client.__codexPlusStatsigNotifiedSignature = signature;
    client.$emt({ name: "values_updated", status: client.loadingStatus, values });
    return true;
  }

  function codexPlusStatsigOverridesTick() {
    const active = codexPlusStatsigOverridesEnabled() || codexPlusStatsigRecording();
    for (const client of codexPlusStatsigOverrideClients()) {
      try {
        if (active && codexPlusStatsigPatchClient(client)) {
          console.log("[StatsigOverrides]", "已接管 Statsig 开关覆盖");
        }
        // 关掉总开关后也走这里：签名回到空串，派发一次让 Codex 回到远端值。
        if (codexPlusStatsigNotifyIfChanged(client)) {
          console.log("[StatsigOverrides]", "已通知 Codex 重读开关");
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
