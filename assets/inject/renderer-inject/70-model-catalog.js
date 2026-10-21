  async function setCodexGlobalState(key, value) {
    return await codexStateCall("set-global-state", { params: { key, value } });
  }

  function dispatchCodexPlusMessage(dispatcher, type, payload) {
    const message = codexServiceTierRequestOverride({ ...(payload || {}), type });
    const nextType = message?.type || type;
    const { type: _type, ...nextPayload } = message || {};
    if (nextType === "browser-use-session-route-capture") {
      observeCodexRemoteSessionNotification({ type: nextType, params: nextPayload });
    }
    return dispatcher.__codexServiceTierOriginalDispatchMessage(nextType, nextPayload);
  }

  function objectGlobalState(value) {
    return value && typeof value === "object" && !Array.isArray(value) ? { ...value } : {};
  }

  function uniqueValues(values) {
    return Array.from(new Set(values.filter((value) => typeof value === "string" && value.trim().length > 0)));
  }

  let codexModelCatalog = { status: "loading", model: "", default_model: "", model_provider: "", codex_model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
  let codexModelCatalogLoadedAt = 0;
  let codexModelCatalogPromise = null;
  let codexModelCatalogLoadSeq = 0;
  let codexModelCatalogRevision = "";
  let codexModelCatalogAppliedEpoch = 0;
  let codexModelWhitelistRefreshTimer = 0;
  let codexModelWhitelistRefreshUntil = 0;
  const codexPlusModelListRequestIds = new Set();
  const codexPlusModelListRequestHosts = new Map();
  // 跨重注入保留原生数据快照；WeakMap 仅持有模型集合，不缓存 DOM。
  const codexPlusModelCollectionSnapshots = window.__codexPlusModelCollectionSnapshots
    || (window.__codexPlusModelCollectionSnapshots = new WeakMap());
  const codexPlusModelDefaultSnapshots = window.__codexPlusModelDefaultSnapshots
    || (window.__codexPlusModelDefaultSnapshots = new WeakMap());
  const codexPlusModelQueryRefreshEpochs = new WeakMap();
  // 看门狗可能重注入；复用进行中的读取与退避链，回调始终使用最新目录闭包。
  const codexPlusNativeConfigRefreshRuntime = window.__codexPlusNativeConfigRefreshRuntime
    || (window.__codexPlusNativeConfigRefreshRuntime = { states: new WeakMap() });
  const codexPlusNativeConfigRefreshStates = codexPlusNativeConfigRefreshRuntime.states;
  codexPlusNativeConfigRefreshRuntime.signature = codexPlusNativeConfigCatalogSignature;
  codexPlusNativeConfigRefreshRuntime.refresh = refreshCodexNativeLocalUserConfig;

  if (window.__CODEX_PLUS_TEST_SERVICE_TIER__) {
    window.__codexPlusServiceTierTest = {
      applyServiceTierOverride: (method, params, threadIdHint = "") => applyCodexServiceTierRequestOverride(method, params, threadIdHint),
      applyProviderOverride: (method, params) => applyCodexRemoteSessionProviderOverride(method, params),
      remoteSessionStartedThreadId: (value) => codexRemoteSessionStartedThreadId(value),
      observeRemoteSessionNotification: (value) => observeCodexRemoteSessionNotification(value),
      installRemoteSessionRecoveryListener: () => installCodexRemoteSessionRecoveryListener(),
      installRemoteSessionDispatcherSubscription: (dispatcher, assetPrefix = "test") => installCodexRemoteSessionDispatcherSubscription(dispatcher, assetPrefix),
      dispatchMessage: (dispatcher, type, payload) => dispatchCodexPlusMessage(dispatcher, type, payload),
      requestOverride: (message) => codexServiceTierRequestOverride(message),
      diagnostics: () => [...(window.__codexPlusServiceTierTestDiagnostics || [])],
      currentModelName: () => codexServiceTierCurrentModelName(),
      fastAvailability: (modelName = codexServiceTierCurrentModelName()) => codexServiceTierFastAvailability(modelName),
      modelDescriptor: (modelName) => codexPlusModelDescriptor(modelName),
      setModelCatalog: (catalog = {}) => {
        codexModelCatalog = {
          status: "ok",
          model: "",
          default_model: "",
          model_provider: "",
          codex_model_provider: "",
          provider_name: "",
          models: [],
          sources: [],
          responses_api: { status: "unknown", message: "" },
          ...catalog,
        };
        codexModelCatalogLoadedAt = Date.now();
        codexModelCatalogAppliedEpoch += 1;
        codexModelCatalogPromise = null;
      },
      loadModelCatalog: (force = false) => loadCodexModelCatalog(force),
      observeModelCatalogRevision: (revision) => observeCodexModelCatalogRevision(revision),
      modelCatalogState: () => ({
        revision: codexModelCatalogRevision,
        loadSeq: codexModelCatalogLoadSeq,
        appliedEpoch: codexModelCatalogAppliedEpoch,
        models: [...(codexModelCatalog.models || [])],
      }),
      patchModelContainer: (value, hostId = "local") => patchModelContainer(value, hostId),
      patchModelArray: (models, allowEmpty = false, hostId = "local") => patchModelArray(models, allowEmpty, hostId),
      patchStatsigModelDynamicConfig: (config, hostId = "local") => patchStatsigModelDynamicConfig(config, hostId),
      refreshCachedLocalModelQueries: () => refreshCachedLocalModelQueries(),
      patchModelJsonResponse,
      patchMcpModelResponseData,
      recordModelListRequest,
      setBackendSettings: (settings = {}) => {
        codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
        codexPlusBackendSettingsLoaded = true;
      },
      providerPatchEnabled: () => codexRemoteSessionProviderPatchEnabled(),
      providerNormalizationEnabled: () => codexRemoteSessionProviderNormalizationEnabled(),
      settingStorageFromModule: codexSettingStorageFromModule,
      stateApiFromModule: codexStateApiFromModule,
      dispatcherFromModule: codexServiceTierDispatcherFromModule,
      patchAppServerClient: patchAppServerModelRequestClient,
      locateAppServerClientBreakpoint: locateCodexAppServerClientBreakpoint,
      installAppServerClientPrototypePatch: installCodexAppServerClientPrototypePatch,
      appServerClientPrototypeState: () => ({
        hasClass: typeof window.__codexPlusAppServerClientClass === "function",
        installed: window.__codexPlusAppServerClientPrototypePatchInstalled || null,
      }),
    };
    return;
  }

  function codexPlusModelUnlockEnabled() {
    return !!codexPlusSettings().modelWhitelistUnlock;
  }

  function codexPlusModelNames() {
    return uniqueValues([
      ...(Array.isArray(codexModelCatalog.models) ? codexModelCatalog.models : []),
      codexModelCatalog.default_model,
      codexModelCatalog.model,
    ]);
  }

  function observeCodexModelCatalogRevision(revision) {
    if (typeof revision !== "string" || !revision || revision === codexModelCatalogRevision) return false;
    codexModelCatalogRevision = revision;
    void loadCodexModelCatalog(true);
    return true;
  }

  async function loadCodexModelCatalog(force = false) {
    if (!force && codexModelCatalogPromise) return codexModelCatalogPromise;
    if (!force && codexModelCatalogLoadedAt && Date.now() - codexModelCatalogLoadedAt < 10000) return codexModelCatalog;
    const seq = ++codexModelCatalogLoadSeq;
    const revision = codexModelCatalogRevision;
    const request = readCodexAppServerPreparation("/codex-model-catalog", {})
      .then(async (result) => {
        const nextCatalog = result && typeof result === "object" ? result : { status: "failed", model: "", default_model: "", model_provider: "", codex_model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
        if ((!nextCatalog.models || nextCatalog.models.length === 0) && nextCatalog.status === "not_configured") {
          try {
            const settingsResp = await readCodexAppServerPreparation("/settings/get", {});
            if (settingsResp && settingsResp.relayProfiles && Array.isArray(settingsResp.relayProfiles)) {
              const activeId = settingsResp.activeRelayId || "";
              const profile = settingsResp.relayProfiles.find(p => p.id === activeId);
              if (profile && profile.modelList) {
                const extraModels = profile.modelList.split(/[\r\n,]+/).map(s => s.trim()).filter(Boolean);
                if (extraModels.length > 0) {
                  nextCatalog.models = extraModels;
                  nextCatalog.default_model = nextCatalog.default_model || extraModels[0];
                  sendCodexPlusDiagnostic("model_catalog_fallback_applied", { count: extraModels.length });
                }
              }
            }
          } catch (fallbackError) {
            sendCodexPlusDiagnostic("model_catalog_fallback_error", { error: String(fallbackError?.message || fallbackError) });
          }
        }
        if (seq !== codexModelCatalogLoadSeq || revision !== codexModelCatalogRevision) {
          return codexModelCatalogPromise && codexModelCatalogPromise !== request
            ? codexModelCatalogPromise
            : codexModelCatalog;
        }
        // 在目录替换前记录选中模型的供应商；裸 ID 在新目录中可能已经代表另一家。
        captureCodexAggregateSelectedModels();
        codexModelCatalog = nextCatalog;
        codexModelCatalogAppliedEpoch += 1;
        codexModelCatalogLoadedAt = Date.now();
        if (!window.__CODEX_PLUS_TEST_SERVICE_TIER__) refreshCachedLocalModelQueries();
        if (typeof codexPlusModelPickerCatalogChanged === "function") codexPlusModelPickerCatalogChanged();
        renderCodexPlusMenu();
        scheduleCodexModelWhitelistRefresh();
        return codexModelCatalog;
      })
      .catch((error) => {
        if (seq !== codexModelCatalogLoadSeq || revision !== codexModelCatalogRevision) {
          return codexModelCatalogPromise && codexModelCatalogPromise !== request
            ? codexModelCatalogPromise
            : codexModelCatalog;
        }
        captureCodexAggregateSelectedModels();
        codexModelCatalog = { status: "failed", message: String(error?.message || error), model: "", default_model: "", model_provider: "", codex_model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
        codexModelCatalogAppliedEpoch += 1;
        codexModelCatalogLoadedAt = Date.now();
        if (typeof codexPlusModelPickerCatalogChanged === "function") codexPlusModelPickerCatalogChanged();
        if (!window.__CODEX_PLUS_TEST_SERVICE_TIER__) refreshCachedLocalModelQueries();
        return codexModelCatalog;
      })
      .finally(() => {
        if (codexModelCatalogPromise === request) codexModelCatalogPromise = null;
      });
    codexModelCatalogPromise = request;
    return request;
  }

  function codexPlusModelMetadata(modelName) {
    const metadata = codexModelCatalog.modelMetadata || codexModelCatalog.model_metadata;
    const normalizedName = codexServiceTierModelFromValue(modelName);
    const exact = metadata && typeof metadata === "object" ? metadata[normalizedName] : null;
    const matchedKey = !exact && metadata && typeof metadata === "object"
      ? Object.keys(metadata).find((key) => key.toLowerCase() === normalizedName.toLowerCase())
      : null;
    const value = exact || (matchedKey ? metadata[matchedKey] : null);
    return value && typeof value === "object" ? value : null;
  }

  function modelReasoningEfforts(modelName) {
    const supported = codexPlusModelMetadata(modelName)?.supportedReasoningEfforts;
    if (Array.isArray(supported) && supported.length > 0) {
      const efforts = supported.map((entry) => ({ ...entry }));
      const hasMax = efforts.some((e) => e.reasoningEffort === "max");
      const hasUltra = efforts.some((e) => e.reasoningEffort === "ultra");
      if (!hasMax) efforts.push({ reasoningEffort: "max", description: "Maximum reasoning depth for the hardest problems" });
      if (!hasUltra) {
        // 名单里的模型确认支持 ultra。不要再把 deepseek 放进来：它的档位上限是 max，
        // 补出来的第四档会让「思考等级」列里只勾了 low/high/max 的模型在客户端多显示一档。
        const shouldAddUltra = /sol|terra|gpt-5\.6|gpt-5\.5|gpt-5\.4/i.test(String(modelName || ""));
        if (shouldAddUltra || efforts.length >= 4) efforts.push({ reasoningEffort: "ultra", description: "Maximum reasoning with automatic task delegation" });
      }
      return efforts;
    }
    return ["low", "medium", "high", "xhigh", "max", "ultra"].map((reasoningEffort) => ({ reasoningEffort, description: `${reasoningEffort} effort` }));
  }

  function applyCodexPlusModelMetadata(descriptor, modelName) {
    const metadata = codexPlusModelMetadata(modelName);
    if (!descriptor || !metadata) return false;
    let changed = false;
    for (const key of ["displayName", "description", "defaultReasoningEffort", "contextWindow", "maxContextWindow", "context_window", "max_context_window"]) {
      const valid = typeof metadata[key] === "string" ? !!metadata[key] : Number.isFinite(metadata[key]) && metadata[key] > 0;
      if (valid && descriptor[key] !== metadata[key]) {
        descriptor[key] = metadata[key];
        changed = true;
      }
    }
    if (Array.isArray(metadata.supportedReasoningEfforts) && metadata.supportedReasoningEfforts.length > 0) {
      const nextEfforts = modelReasoningEfforts(modelName);
      if (JSON.stringify(descriptor.supportedReasoningEfforts || []) !== JSON.stringify(nextEfforts)) {
        descriptor.supportedReasoningEfforts = nextEfforts;
        changed = true;
      }
    }
    return changed;
  }

  function codexPlusModelDescriptor(modelName) {
    const metadata = codexPlusModelMetadata(modelName);
    return {
      model: modelName,
      id: modelName,
      slug: modelName,
      name: modelName,
      displayName: metadata?.displayName || modelName,
      description: metadata?.description || codexModelCatalog.provider_name || codexModelCatalog.model_provider || "Custom model",
      hidden: false,
      isDefault: false,
      defaultReasoningEffort: metadata?.defaultReasoningEffort || "medium",
      supportedReasoningEfforts: modelReasoningEfforts(modelName),
    };
  }

  function modelArrayLooksPatchable(value, allowEmpty = false) {
    return Array.isArray(value)
      && (allowEmpty || value.length > 0)
      && value.every((item) => item && typeof item === "object" && typeof item.model === "string");
  }

  function stringArrayLooksPatchable(value) {
    return Array.isArray(value) && value.every((item) => typeof item === "string");
  }

  function patchModelNameArray(models, hostId) {
    if (!stringArrayLooksPatchable(models)) return false;
    const allowed = codexAggregateAllowedModelSet(hostId) || codexPureApiAllowedModelSet(hostId);
    let changed = prepareModelCollection(models, allowed);
    const customModels = codexPlusModelNames();
    if (!customModels.length) return changed;
    if (allowed) {
      const retained = models.filter((name) => allowed.has(name));
      if (retained.length !== models.length) {
        models.splice(0, models.length, ...retained);
        changed = true;
      }
    }
    customModels.forEach((modelName) => {
      if (!models.includes(modelName)) {
        models.push(modelName);
        changed = true;
      }
    });
    if (allowed) {
      const ordered = [...allowed];
      if (!sameModelCollection(models, ordered)) {
        models.splice(0, models.length, ...ordered);
        changed = true;
      }
    }
    finishModelCollection(models);
    return changed;
  }

  function modelFilterPolicy(allowed) {
    return allowed ? `${codexRemoteSessionActiveProfile()?.id || ""}:${codexModelCatalogAppliedEpoch}:${JSON.stringify([...allowed].sort())}` : "";
  }

  function sameModelCollection(left, right) {
    return !!right && left.length === right.length && left.every((item, index) => item === right[index]);
  }

  function prepareModelCollection(collection, allowed) {
    const policy = modelFilterPolicy(allowed);
    const previous = codexPlusModelCollectionSnapshots.get(collection);
    let current = [...collection];
    let changed = false;
    if (previous && previous.policy !== policy && sameModelCollection(current, previous.patched)) {
      if (!sameModelCollection(current, previous.original)) {
        // 恢复的描述符也要独立，后续重新投影不能反过来污染历史快照。
        const restored = previous.original.map((item) => item && typeof item === "object" ? { ...item } : item);
        if (Array.isArray(collection)) collection.splice(0, collection.length, ...restored);
        else { collection.clear(); restored.forEach((item) => collection.add(item)); }
        current = [...collection];
        changed = true;
      }
    }
    if (!allowed) codexPlusModelCollectionSnapshots.delete(collection);
    else if (!previous || previous.policy !== policy || !sameModelCollection(current, previous.patched)) {
      // 元数据补丁只替换顶层字段，独立描述符可保留字段原值及原有数组引用。
      const original = current.map((item) => item && typeof item === "object" ? { ...item } : item);
      codexPlusModelCollectionSnapshots.set(collection, { policy, original, patched: null });
    }
    return changed;
  }

  function finishModelCollection(collection) {
    const snapshot = codexPlusModelCollectionSnapshots.get(collection);
    if (snapshot) snapshot.patched = [...collection];
  }

  function patchModelDefaults(value, allowed) {
    const policy = modelFilterPolicy(allowed);
    const previous = codexPlusModelDefaultSnapshots.get(value);
    let changed = false;
    if (previous && previous.policy !== policy) {
      for (const [key, state] of Object.entries(previous.fields)) {
        if (value[key] === state.patched && value[key] !== state.original) {
          value[key] = state.original;
          changed = true;
        }
      }
      codexPlusModelDefaultSnapshots.delete(value);
    }
    if (!allowed) return changed;
    const snapshot = codexPlusModelDefaultSnapshots.get(value) || { policy, fields: {} };
    const fallback = [codexModelCatalog.model, codexModelCatalog.default_model, ...allowed]
      .find((name) => typeof name === "string" && allowed.has(name));
    for (const key of ["defaultModel", "default_model"]) {
      const original = value[key];
      const name = typeof original === "string" ? original : original?.model || original?.id || original?.slug;
      if (!name || !fallback || name === fallback) continue;
      const patched = typeof original === "string" ? fallback : codexPlusModelDescriptor(fallback);
      if (!snapshot.fields[key] || original !== snapshot.fields[key].patched) {
        snapshot.fields[key] = { original, patched };
      }
      value[key] = patched;
      changed = true;
    }
    codexPlusModelDefaultSnapshots.set(value, snapshot);
    return changed;
  }

  // 只收窄当前纯 API 供应商的有效目录；名单里的 GPT 也是可用模型，不能按前缀删。
  // 目录失败/未配置/仍属于上一个供应商时继续保留原生列表。
  function modelResponseHostId(...values) {
    let local = false;
    for (const value of values) {
      let candidates;
      try { candidates = typeof value === "string" ? [value] : [value?.hostId, value?.host_id]; }
      catch { return ""; }
      for (const candidate of candidates) {
        if (typeof candidate !== "string" || !candidate.trim()) continue;
        const host = candidate.trim();
        if (host !== "local") return host;
        local = true;
      }
    }
    return local ? "local" : "";
  }

  function codexPureApiAllowedModelSet(hostId) {
    if (hostId !== "local") return null;
    const profile = codexRemoteSessionActiveProfile();
    if (!codexPlusModelUnlockEnabled() || !codexPlusBackendSettingsLoaded
        || profile?.relayMode !== "pureApi" || codexModelCatalog.status !== "ok") return null;
    const profileId = String(profile.id || "").trim();
    const belongsToProfile = profileId && (codexModelCatalog.model_provider === profileId
      || (Array.isArray(codexModelCatalog.sources)
        && codexModelCatalog.sources.some((source) => source?.id === `relay-profile:${profileId}`)));
    const names = codexPlusModelNames();
    return belongsToProfile && names.length ? new Set(names) : null;
  }

  function codexAggregateAllowedModelSet(hostId, allowUnpublished = false) {
    if (hostId !== "local" || !codexPlusBackendSettingsLoaded
        || (codexModelCatalog.status !== "ok"
          && !(allowUnpublished && codexModelCatalog.status === "not_applied"
            && Array.isArray(codexModelCatalog.models)))) return null;
    const profile = codexRemoteSessionActiveProfile();
    if (profile?.relayMode !== "aggregate") return null;
    const belongsToProfile = Array.isArray(codexModelCatalog.sources)
      && codexModelCatalog.sources.some((source) =>
        source?.id === `aggregate:${profile.id}`
        && ["aggregate_model_catalog", "aggregate_pool_catalog"].includes(source?.type));
    return belongsToProfile ? new Set(codexModelCatalog.status === "not_applied"
      ? uniqueValues(codexModelCatalog.models) : codexPlusModelNames()) : null;
  }

  function codexAggregateSelectedModelTarget(model, hostId, groupId = "") {
    const allowed = codexAggregateAllowedModelSet(hostId);
    if (!allowed || typeof model !== "string") return "";
    const pool = codexModelCatalog.sources?.some((source) => source?.type === "aggregate_pool_catalog");
    if (pool) {
      const bare = model.startsWith("cpx-") ? model.slice(model.indexOf("/") + 1) : model;
      return bare !== model && allowed.has(bare) ? bare : "";
    }
    const defaultGroup = codexModelCatalog.codex_plus_default_group_id;
    if (typeof defaultGroup !== "string" || !defaultGroup) return "";
    if (groupId) {
      const base = model.startsWith("cpx-") ? model.slice(model.indexOf("/") + 1) : model;
      const target = groupId === defaultGroup ? base : `cpx-${groupId}/${base}`;
      return target !== model && allowed.has(target)
        && codexPlusModelMetadata(target)?.codex_plus_group_id === groupId ? target : "";
    }
    if (allowed.has(model)) return "";
    const prefix = `cpx-${defaultGroup}/`;
    if (!model.startsWith(prefix)) return "";
    const target = model.slice(prefix.length);
    // 只迁移晋升为默认成员的旧前缀，不能凭同名模型切到另一家。
    return allowed.has(target) && codexPlusModelMetadata(target)?.codex_plus_group_id === defaultGroup
      ? target : "";
  }

  function codexAggregateModelPickers() {
    const pickers = [];
    for (const node of document.querySelectorAll("[data-codex-intelligence-trigger]")) {
      const key = Object.keys(node).find((name) => name.startsWith("__reactFiber$"));
      let fiber = key ? node[key] : null;
      let picker = null;
      let hostId = "";
      let conversationId = "";
      let threadId = "";
      for (let depth = 0; fiber && depth < 128; depth += 1, fiber = fiber.return) {
        const props = fiber.memoizedProps || {};
        if (!picker && typeof props.model === "string" && typeof props.onSelectModel === "function"
            && Array.isArray(props.models)) picker = props;
        if (!hostId && typeof props.permissionsHostId === "string" && props.permissionsHostId.trim()) {
          hostId = props.permissionsHostId.trim();
        }
        if (!conversationId && typeof props.conversationId === "string") conversationId = props.conversationId.trim();
        if (!threadId && typeof props.threadId === "string") threadId = props.threadId.trim();
      }
      // 宿主以 conversationId 标识当前会话；不能被更近层的旧 threadId 遮蔽。
      const owner = conversationId || threadId;
      if (picker && hostId === "local" && !picker.lockedModelSlug && !picker.modelOptionsDisabled
          && !picker.modelSelectionDisabled) pickers.push({ node, picker, owner, hostId });
      else {
        window.__codexPlusAggregateModelSelectionIdentities?.delete(node);
        window.__codexPlusAggregateModelSelectionAttempts?.delete(node);
      }
    }
    return pickers;
  }

  function captureCodexAggregateSelectedModels() {
    if (!codexAggregateAllowedModelSet("local")) return;
    const identities = window.__codexPlusAggregateModelSelectionIdentities
      || (window.__codexPlusAggregateModelSelectionIdentities = new WeakMap());
    const profileId = codexRemoteSessionActiveProfile()?.id;
    for (const { node, picker, owner, hostId } of codexAggregateModelPickers()) {
      if (!owner) { identities.delete(node); continue; }
      const previous = identities.get(node);
      // 连续保存时 React 可能尚未消费上一次迁移，保留最初的供应商身份。
      if (previous && previous.model === picker.model && previous.owner === owner
          && previous.hostId === hostId && previous.profileId === profileId) continue;
      const groupId = codexPlusModelMetadata(picker.model)?.codex_plus_group_id;
      if (typeof groupId === "string" && groupId) {
        identities.set(node, { model: picker.model, groupId, owner, hostId, profileId });
      }
    }
  }

  function syncCodexAggregateSelectedModel() {
    if (!codexAggregateAllowedModelSet("local")) return false;
    const attempts = window.__codexPlusAggregateModelSelectionAttempts
      || (window.__codexPlusAggregateModelSelectionAttempts = new WeakMap());
    let changed = false;
    for (const { node, picker, owner, hostId } of codexAggregateModelPickers()) {
      const identities = window.__codexPlusAggregateModelSelectionIdentities;
      const identity = identities?.get(node);
      const retained = identity?.model === picker.model && identity?.owner === owner && identity?.hostId === hostId
        && identity?.profileId === codexRemoteSessionActiveProfile()?.id;
      if (identity && !retained) identities.delete(node);
      const target = codexAggregateSelectedModelTarget(picker.model, "local", retained ? identity.groupId : "");
      // 等 React 消费新列表后再调用原生选择入口，保留当前思考等级。
      if (!target || !picker.models.some((entry) => entry?.model === target)) continue;
      const signature = JSON.stringify([codexModelCatalogAppliedEpoch,
        codexRemoteSessionActiveProfile()?.id || "", hostId, owner,
        picker.model, target, picker.reasoningEffort || ""]);
      const previous = attempts.get(node);
      if (previous?.callback === picker.onSelectModel && previous.signature === signature) continue;
      attempts.set(node, { callback: picker.onSelectModel, signature });
      try {
        picker.onSelectModel(target, picker.reasoningEffort);
        changed = true;
      } catch (error) {
        attempts.delete(node);
        sendCodexPlusDiagnostic("aggregate_model_selection_refresh_failed", { error: String(error?.message || error) });
      }
    }
    return changed;
  }

  function codexPlusConfiguredDefaultModel() {
    if (codexModelCatalog.status !== "ok") return "";
    const allowed = codexAggregateAllowedModelSet("local") || codexPureApiAllowedModelSet("local");
    if (!allowed) return "";
    return [codexModelCatalog.model, codexModelCatalog.default_model, ...allowed]
      .find((model) => typeof model === "string" && allowed.has(model)) || "";
  }

  function syncCodexConfiguredDefaultModel() {
    const target = codexPlusConfiguredDefaultModel();
    if (!target) return;
    const attempts = window.__codexPlusDefaultModelAttempts ||= new WeakMap();
    for (const { node, picker, owner } of codexAggregateModelPickers()) {
      // 已有会话交给其保存的模型身份恢复；新会话不能沿用原生目录外的默认值。
      if (owner || picker.disabled || codexPlusModelNames().includes(picker.model)
          || codexAggregateSelectedModelTarget(picker.model, "local")
          || !picker.models.some((entry) => entry?.model === target)) continue;
      const signature = `${codexModelCatalogAppliedEpoch}:${picker.model}:${target}`;
      if (attempts.get(node) === signature) continue;
      attempts.set(node, signature);
      const efforts = modelReasoningEfforts(target);
      const effort = efforts.some((entry) => entry.reasoningEffort === picker.reasoningEffort)
        ? picker.reasoningEffort : codexPlusModelMetadata(target)?.defaultReasoningEffort || "medium";
      try { Promise.resolve(picker.onSelectModel(target, effort)).catch(() => {}); } catch {}
    }
  }

  function normalizeCodexConfiguredModelRequest(method, params, hostId) {
    if (hostId !== "local" || !params || typeof params !== "object") return params;
    if (method === "send-cli-request-for-host" && params.method) {
      if (modelResponseHostId(params) !== "local") return params;
      const next = normalizeCodexConfiguredModelRequest(String(params.method), params.params, hostId);
      return next === params.params ? params : { ...params, params: next };
    }
    if (!["thread/start", "thread/resume", "turn/start"].includes(method)) return params;
    const allowed = codexAggregateAllowedModelSet(hostId) || codexPureApiAllowedModelSet(hostId);
    if (!allowed) return params;
    const model = params.model || params.collaborationMode?.settings?.model;
    if (model && allowed.has(model)) return params;
    const mapped = model && codexAggregateSelectedModelTarget(model, hostId);
    const target = mapped || (method === "thread/start" ? codexPlusConfiguredDefaultModel() : "");
    if (!target || target === model) return params;
    const next = { ...params, model: target };
    if (params.collaborationMode?.settings?.model) {
      next.collaborationMode = { ...params.collaborationMode,
        settings: { ...params.collaborationMode.settings, model: target } };
    }
    return next;
  }

  function patchModelArray(models, allowEmpty = false, hostId) {
    if (!modelArrayLooksPatchable(models, allowEmpty)) return false;
    const aggregateModels = codexAggregateAllowedModelSet(hostId);
    // 未发布目录只撤下原生列表里的旧模型，不新增型号、修改默认值或触发自动迁移。
    const unpublishedModels = codexModelCatalog.status === "not_applied"
      ? codexAggregateAllowedModelSet(hostId, true) : null;
    const allowed = aggregateModels || codexPureApiAllowedModelSet(hostId) || unpublishedModels;
    let changed = prepareModelCollection(models, allowed);
    const customModels = unpublishedModels ? [] : codexPlusModelNames();
    if (!customModels.length && !allowed) return changed;
    if (allowed) {
      const retained = models.filter((item) => allowed.has(item.model));
      if (retained.length !== models.length) {
        models.splice(0, models.length, ...retained);
        changed = true;
      }
    }
    const existing = new Map(models.map((item) => [item.model, item]));
    models.forEach((item) => {
      if (customModels.includes(item.model)) {
        if (item.hidden !== false) {
          item.hidden = false;
          changed = true;
        }
        if (applyCodexPlusModelMetadata(item, item.model)) changed = true;
      }
    });
    customModels.forEach((modelName) => {
      if (!existing.has(modelName)) {
        models.push(codexPlusModelDescriptor(modelName));
        changed = true;
      }
    });
    if (allowed) {
      // app-server 可能仍缓存旧 catalog。默认成员变化会重命名 slug，必须撤掉旧前缀，
      // 并按新目录排列；只做追加会留下旧标签、重复组及找不到新选中模型的问题。
      const byModel = new Map(models.map((item) => [item.model, item]));
      const ordered = [...allowed].map((name) => byModel.get(name)).filter(Boolean);
      if (!sameModelCollection(models, ordered)) {
        models.splice(0, models.length, ...ordered);
        changed = true;
      }
      if (!unpublishedModels) {
        const defaultModel = codexModelCatalog.model || codexModelCatalog.default_model;
        models.forEach((item) => {
          const isDefault = item.model === defaultModel;
          if (item.isDefault !== isDefault) { item.isDefault = isDefault; changed = true; }
        });
      }
    }
    finishModelCollection(models);
    return changed;
  }

  function patchModelContainer(value, hostId) {
    if (!value || typeof value !== "object") return false;
    hostId = modelResponseHostId(hostId, value, value.message, value.result);
    let changed = false;
    if (patchModelArray(value.models, "defaultModel" in value || "availableModels" in value, hostId)) changed = true;
    if (patchModelNameArray(value.models, hostId)) changed = true;
    if (patchModelArray(value.data, false, hostId)) changed = true;
    if (patchModelArray(value.result, false, hostId)) changed = true;
    if (patchModelArray(value.pages?.[0]?.data, false, hostId)) changed = true;
    if (patchModelArray(value.result?.data, false, hostId)) changed = true;
    if (patchModelArray(value.result?.models, false, hostId)) changed = true;
    if (patchModelArray(value.message?.result?.data, false, hostId)) changed = true;
    if (patchModelArray(value.message?.result?.models, false, hostId)) changed = true;
    const names = codexPlusModelNames();
    const allowed = codexAggregateAllowedModelSet(hostId) || codexPureApiAllowedModelSet(hostId);
    if (value.availableModels instanceof Set) {
      if (prepareModelCollection(value.availableModels, allowed)) changed = true;
      if (allowed) for (const name of value.availableModels) {
        if (!allowed.has(name)) { value.availableModels.delete(name); changed = true; }
      }
      names.forEach((name) => {
        if (!value.availableModels.has(name)) {
          value.availableModels.add(name);
          changed = true;
        }
      });
      finishModelCollection(value.availableModels);
    }
    if (value.available_models instanceof Set) {
      if (prepareModelCollection(value.available_models, allowed)) changed = true;
      if (allowed) for (const name of value.available_models) {
        if (!allowed.has(name)) { value.available_models.delete(name); changed = true; }
      }
      names.forEach((name) => {
        if (!value.available_models.has(name)) {
          value.available_models.add(name);
          changed = true;
        }
      });
      finishModelCollection(value.available_models);
    }
    if (Array.isArray(value.availableModels)) {
      if (patchModelNameArray(value.availableModels, hostId)) changed = true;
    }
    if (Array.isArray(value.available_models)) {
      if (patchModelNameArray(value.available_models, hostId)) changed = true;
    }
    if (Array.isArray(value.hiddenModels)) {
      const before = value.hiddenModels.length;
      value.hiddenModels = value.hiddenModels.filter((name) => !names.includes(name));
      if (value.hiddenModels.length !== before) changed = true;
    }
    if (Array.isArray(value.hidden_models)) {
      const before = value.hidden_models.length;
      value.hidden_models = value.hidden_models.filter((name) => !names.includes(name));
      if (value.hidden_models.length !== before) changed = true;
    }
    if (patchModelDefaults(value, allowed)) changed = true;
    return changed;
  }

  function modelJsonResponseLooksPatchable(payload) {
    if (!payload || typeof payload !== "object") return false;
    const descriptorArrays = [
      payload.models,
      payload.data,
      payload.result,
      payload.pages?.[0]?.data,
      payload.result?.data,
      payload.result?.models,
      payload.message?.result?.data,
      payload.message?.result?.models,
    ];
    if (descriptorArrays.some((value) => modelArrayLooksPatchable(value))) return true;
    const hasModelContainerSignal = "defaultModel" in payload
      || "default_model" in payload
      || "availableModels" in payload
      || "available_models" in payload
      || "hiddenModels" in payload
      || "hidden_models" in payload
      || "modelMetadata" in payload
      || "model_metadata" in payload;
    return hasModelContainerSignal && Array.isArray(payload.models)
      && payload.models.every((value) => typeof value === "string");
  }

  async function patchModelJsonResponse(payload) {
    if (!codexPlusModelUnlockEnabled()) return payload;
    if (!codexPlusModelNames().length) await loadCodexModelCatalog();
    if (!modelJsonResponseLooksPatchable(payload)) return payload;
    try {
      patchModelContainer(payload, modelResponseHostId(payload, payload.result, payload.message));
    } catch (error) {
      window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
      window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
    }
    return payload;
  }

  function installModelJsonResponsePatch() {
    if (window.__codexPlusModelJsonResponsePatchInstalled === "3") return;
    window.__codexPlusModelJsonResponsePatchInstalled = "3";
    window.__codexPlusModelJsonResponseOriginals = window.__codexPlusModelJsonResponseOriginals || {};
    const originals = window.__codexPlusModelJsonResponseOriginals;
    originals.responseJson = originals.responseJson || Response.prototype.json;
    if (typeof originals.responseJson !== "function") return;
    Response.prototype.json = async function codexPlusPatchedResponseJson(...args) {
      const payload = await originals.responseJson.apply(this, args);
      return await patchModelJsonResponse(payload);
    };
  }

  function patchStatsigModelDynamicConfig(config, hostId) {
    const value = config?.value;
    if (!value || typeof value !== "object") return config;
    const availableModels = Array.isArray(value.available_models) ? [...value.available_models] : [];
    const collectionSnapshot = codexPlusModelCollectionSnapshots.get(value.available_models);
    if (collectionSnapshot) codexPlusModelCollectionSnapshots.set(availableModels, { ...collectionSnapshot });
    const nextValue = { ...value, available_models: availableModels };
    const defaultSnapshot = codexPlusModelDefaultSnapshots.get(value);
    if (defaultSnapshot) codexPlusModelDefaultSnapshots.set(nextValue, { ...defaultSnapshot, fields: { ...defaultSnapshot.fields } });
    hostId = modelResponseHostId(hostId, value);
    const listChanged = patchModelNameArray(availableModels, hostId);
    const defaultsChanged = patchModelDefaults(nextValue, codexAggregateAllowedModelSet(hostId) || codexPureApiAllowedModelSet(hostId));
    if (!listChanged && !defaultsChanged) return config;
    try {
      config.value = nextValue;
    } catch {
      return { ...config, value: nextValue };
    }
    return config;
  }

  function codexPlusNativeConfigCatalogSignature() {
    if (codexModelCatalog.status !== "ok") return "";
    // epoch 会随轮询增长；只对目录/配置版本变化重新读原生配置。
    return JSON.stringify([
      codexModelCatalogRevision, codexModelCatalog.model_provider,
      codexModelCatalog.codex_model_provider, codexModelCatalog.model,
      codexModelCatalog.default_model, codexModelCatalog.models,
      codexModelCatalog.modelMetadata || codexModelCatalog.model_metadata,
      (codexModelCatalog.sources || []).map((source) => [source.id, source.type]),
    ]);
  }

  function refreshCodexNativeLocalUserConfig(client) {
    if (!codexPlusModelUnlockEnabled() && !codexAggregateAllowedModelSet("local")) return false;
    const signature = codexPlusNativeConfigCatalogSignature();
    if (!signature) return false;
    // 原生模型 observer 依赖这个禁用自动读取的查询来决定目录和白名单。
    // invalidateQueries 不会重新读取 disabled observer，必须复用其原生 queryFn。
    const queries = client.getQueryCache?.()?.getAll?.() || [];
    let requested = false;
    for (const query of queries) {
      const key = query?.queryKey;
      if (!Array.isArray(key) || key.length !== 3 || key[0] !== "config"
        || key[1] !== "user" || key[2] !== "local"
        || typeof query.fetch !== "function" || typeof query.options?.queryFn !== "function") continue;
      let state = codexPlusNativeConfigRefreshStates.get(query);
      if (!state) {
        state = { completed: "", pending: null, changedWhilePending: false, failed: "", failures: 0, retryAfter: 0, retryTimer: 0 };
        codexPlusNativeConfigRefreshStates.set(query, state);
      }
      if (state.pending) {
        if (state.pending !== signature) state.changedWhilePending = true;
        continue;
      }
      if (state.completed === signature || (state.failed === signature && Date.now() < state.retryAfter)) continue;
      requested = true;
      if (state.retryTimer) window.clearTimeout(state.retryTimer);
      state.retryTimer = 0;
      // 先登记 pending，防止 fetch 同步通知 React 后再次进入扫描。
      state.completed = "";
      state.changedWhilePending = false;
      state.pending = signature;
      Promise.resolve().then(() => query.fetch(undefined, { cancelRefetch: true }))
        .then((data) => {
          if (data?.readSucceeded !== true || !data.response?.config) throw new Error("native config read failed");
          state.completed = signature;
          state.failures = 0;
          state.retryAfter = 0;
          sendCodexPlusDiagnostic("model_native_config_refreshed", {});
        })
        .catch(() => {
          state.failures = state.failed === signature ? state.failures + 1 : 1;
          state.failed = signature;
          const delay = Math.min(30000, 1000 * 2 ** Math.min(state.failures - 1, 5));
          state.retryAfter = Date.now() + delay;
          state.retryTimer = window.setTimeout(() => {
            state.retryTimer = 0;
            codexPlusNativeConfigRefreshRuntime.refresh(client);
          }, delay);
        })
        .finally(() => {
          state.pending = null;
          // 连续切换期间的旧请求结束后，再读一次最新配置，不能把旧结果视作已更新。
          if (state.changedWhilePending || signature !== codexPlusNativeConfigRefreshRuntime.signature()) {
            state.completed = "";
            state.retryAfter = 0;
            codexPlusNativeConfigRefreshRuntime.refresh(client);
          }
        });
    }
    return requested;
  }

  function codexPlusLocalModelQuery(query) {
    const key = query?.queryKey;
    return Array.isArray(key)
      && key[0] === "models"
      && key[1] === "list"
      && key[2] === "local";
  }

  function cloneCodexPlusModelCollection(models) {
    const next = models.map((model) => model && typeof model === "object" ? { ...model } : model);
    transferCodexPlusModelCollectionSnapshot(models, next);
    return next;
  }

  function transferCodexPlusModelCollectionSnapshot(models, next) {
    const snapshot = codexPlusModelCollectionSnapshots.get(models);
    if (snapshot && sameModelCollection(models, snapshot.patched)) {
      // 克隆与 TanStack 结构共享都可能替换元素引用；恢复时必须匹配最终存储的元素。
      codexPlusModelCollectionSnapshots.set(next, {
        ...snapshot,
        original: [...snapshot.original],
        patched: [...next],
      });
    }
  }

  function patchCachedLocalModelQueryData(current) {
    if (Array.isArray(current)) {
      const next = cloneCodexPlusModelCollection(current);
      return patchModelArray(next, true, "local") ? next : current;
    }
    if (!current || typeof current !== "object" || !Array.isArray(current.data)) return current;
    const data = cloneCodexPlusModelCollection(current.data);
    return patchModelArray(data, true, "local") ? { ...current, data } : current;
  }

  function codexPlusModelQueryCollection(value) {
    if (Array.isArray(value)) return value;
    return value && typeof value === "object" && Array.isArray(value.data) ? value.data : null;
  }

  function refreshCachedLocalModelQueries() {
    const client = typeof findCodexQueryClient === "function"
      ? findCodexQueryClient()
      : (window.__REACT_QUERY_CLIENT__ || window.__codexQueryClient);
    if (!client || typeof client.setQueryData !== "function") return false;
    installCodexPlusModelQueryPublication(client);
    refreshCodexNativeLocalUserConfig(client);
    if (codexPlusModelQueryRefreshEpochs.get(client) === codexModelCatalogAppliedEpoch) return false;
    const cache = client.getQueryCache?.();
    const queries = (typeof cache?.findAll === "function"
      ? cache.findAll({ queryKey: ["models", "list"] })
      : (typeof cache?.getAll === "function" ? cache.getAll() : []))
      .filter(codexPlusLocalModelQuery);
    if (!queries.length) return false;
    let published = false;
    for (const query of queries) {
      const currentCollection = codexPlusModelQueryCollection(query.state?.data || query.data);
      if (!codexPlusModelUnlockEnabled() && !codexAggregateAllowedModelSet("local", true)
          && (!currentCollection || !codexPlusModelCollectionSnapshots.has(currentCollection))) continue;
      try {
        let nextCollection = null;
        const stored = client.setQueryData(query.queryKey, (current) => {
          const next = patchCachedLocalModelQueryData(current);
          nextCollection = next !== current ? codexPlusModelQueryCollection(next) : null;
          return next;
        });
        const storedCollection = codexPlusModelQueryCollection(stored);
        if (nextCollection && storedCollection && storedCollection !== nextCollection)
          transferCodexPlusModelCollectionSnapshot(nextCollection, storedCollection);
        published = true;
      } catch {
      }
      if (typeof client.invalidateQueries === "function") {
        try {
          Promise.resolve(client.invalidateQueries({ queryKey: query.queryKey, exact: true })).catch(() => {});
          published = true;
        } catch {
        }
      }
    }
    if (!published) return false;
    codexPlusModelQueryRefreshEpochs.set(client, codexModelCatalogAppliedEpoch);
    sendCodexPlusDiagnostic("model_query_cache_refreshed", { queryCount: queries.length });
    return true;
  }

  function installCodexPlusModelQueryPublication(client) {
    // 保存后 exact invalidate 仍可能从 app-server 拿回旧目录。在通知 React 之前投影，
    // 保证晚到响应不能把新列表覆盖回去。跨重注入复用钩子，不层层包裹 Query 原型。
    const runtime = window.__codexPlusModelQueryPublication
      || (window.__codexPlusModelQueryPublication = { prototypes: new WeakSet() });
    runtime.rewrite = (query, data) => codexPlusLocalModelQuery(query)
      && (codexPlusModelUnlockEnabled() || codexAggregateAllowedModelSet("local", true))
      ? patchCachedLocalModelQueryData(data) : data;
    runtime.transfer = (next, stored) => {
      const source = codexPlusModelQueryCollection(next);
      const target = codexPlusModelQueryCollection(stored);
      if (source && target && source !== target) transferCodexPlusModelCollectionSnapshot(source, target);
    };
    for (const query of client.getQueryCache?.()?.getAll?.() || []) {
      const proto = Object.getPrototypeOf(query);
      if (!proto || typeof proto.setData !== "function" || runtime.prototypes.has(proto)) continue;
      const original = proto.setData;
      proto.setData = function codexPlusPublishModelQuery(data, ...rest) {
        const next = runtime.rewrite(this, data);
        const stored = original.call(this, next, ...rest);
        runtime.transfer(next, stored);
        return stored;
      };
      runtime.prototypes.add(proto);
    }
  }

  function statsigClients() {
    const root = window.__STATSIG__ || globalThis.__STATSIG__;
    if (!root || typeof root !== "object") return [];
    const clients = [root.firstInstance, typeof root.instance === "function" ? root.instance() : null];
    if (root.instances && typeof root.instances === "object") clients.push(...Object.values(root.instances));
    return clients.filter((client, index, array) => client && typeof client === "object" && array.indexOf(client) === index);
  }

  function patchStatsigModelWhitelist() {
    statsigClients().forEach((client) => {
      if (typeof client.getDynamicConfig !== "function") return;
      if (client.__codexPlusModelWhitelistPatched !== "3") {
        const originalGetDynamicConfig = client.getDynamicConfig.bind(client);
        client.getDynamicConfig = (name, options) => {
          const result = originalGetDynamicConfig(name, options);
          return String(name) === "107580212" ? patchStatsigModelDynamicConfig(result) : result;
        };
        client.__codexPlusModelWhitelistPatched = "3";
      }
      try {
        patchStatsigModelDynamicConfig(client.getDynamicConfig("107580212", { disableExposureLog: true }));
      } catch {
      }
    });
  }

  function patchAppServerModelMessages() {
    if (window.__codexPlusModelMessagePatchInstalled === "3") return;
    window.__codexPlusModelMessagePatchInstalled = "3";
    window.addEventListener("codex-message-from-view", (event) => {
      try {
        const detail = event?.detail;
        const request = detail?.request;
        if (detail?.type === "mcp-request" && request?.method === "model/list") {
          request.params = { ...(request.params || {}), includeHidden: true };
          if (request.id != null) {
            const requestId = String(request.id);
            recordModelListRequest(requestId, modelResponseHostId(detail, request, request.params));
          }
        }
      } catch (error) {
        window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
        window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
      }
    }, true);

    window.addEventListener("message", (event) => {
      try {
        patchMcpModelResponseData(event?.data);
      } catch (error) {
        window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
        window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
      }
    }, true);
  }

  function recordModelListRequest(requestId, hostId) {
    if (codexPlusModelListRequestIds.has(requestId)
        && codexPlusModelListRequestHosts.get(requestId) !== hostId) hostId = "";
    codexPlusModelListRequestIds.add(requestId);
    codexPlusModelListRequestHosts.set(requestId, hostId);
    if (codexPlusModelListRequestIds.size > 64) {
      const oldest = codexPlusModelListRequestIds.values().next().value;
      codexPlusModelListRequestIds.delete(oldest);
      codexPlusModelListRequestHosts.delete(oldest);
    }
    window.setTimeout(() => {
      codexPlusModelListRequestIds.delete(requestId);
      codexPlusModelListRequestHosts.delete(requestId);
    }, 30_000);
  }

  function patchMcpModelResponseData(data) {
    if (!codexPlusModelUnlockEnabled()) return false;
    if (data?.type !== "mcp-response") return false;
    const message = data.message || data.response;
    const requestId = message?.id != null ? String(message.id) : "";
    if (codexPlusModelListRequestIds.size === 0 || !codexPlusModelListRequestIds.has(requestId)) return false;
    const hostId = modelResponseHostId(data, message, codexPlusModelListRequestHosts.get(requestId));
    codexPlusModelListRequestIds.delete(requestId);
    codexPlusModelListRequestHosts.delete(requestId);
    let changed = false;
    if (patchModelArray(message?.result?.data, true, hostId)) changed = true;
    if (patchModelArray(message?.result?.models, true, hostId)) changed = true;
    return changed;
  }

  function appServerModelRequestMethod(method, params) {
    if (method === "send-cli-request-for-host" && params?.method) return appServerModelRequestMethod(String(params.method), params.params);
    if (method === "vscode://codex/list-plugins") return "list-plugins";
    if (method === "vscode://codex/plugin/install") return "install-plugin";
    if (method === "vscode://codex/plugin/uninstall") return "uninstall-plugin";
    if (method === "plugin/list") return "list-plugins";
    if (method === "plugin/install") return "install-plugin";
    if (method === "plugin/read") return "read-plugin";
    if (method === "plugin/installed") return "installed-plugins";
    if (method === "plugin/uninstall") return "uninstall-plugin";
    return String(method || "");
  }

  function patchAppServerModelResult(method, result, hostId) {
    if (!["list-models-for-host", "model/list"].includes(method)) return result;
    hostId = modelResponseHostId(hostId, result);
    try {
      if (Array.isArray(result)) patchModelArray(result, true, hostId);
      if (Array.isArray(result?.data)) patchModelArray(result.data, true, hostId);
      if (Array.isArray(result?.models)) patchModelArray(result.models, true, hostId);
      sendCodexPlusDiagnostic("model_app_server_result_patched", {
        method,
        modelCount: Array.isArray(result?.data) ? result.data.length : Array.isArray(result?.models) ? result.models.length : Array.isArray(result) ? result.length : null,
      });
    } catch (error) {
      window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
      window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
    }
    return result;
  }

  // 原生 next-turn 设置可能仅存活到线程卸载；只记录已成功提交的模型设置。
  function codexRememberedThreadModels() {
    try {
      const value = JSON.parse(localStorage.getItem("codexPlus.threadModels.v1") || "[]");
      return Array.isArray(value) ? value.filter((entry) => entry && typeof entry.key === "string"
        && typeof entry.model === "string" && Number.isFinite(entry.at)).slice(0, 2000) : [];
    } catch { return []; }
  }

  function codexThreadModelStorageKey(hostId, threadId) {
    const profile = codexRemoteSessionActiveProfile();
    return hostId === "local" && codexPlusBackendSettingsLoaded && profile?.id && threadId
      ? JSON.stringify([hostId, profile.id, threadId]) : "";
  }

  function codexRetainedThreadModel(hostId, threadId) {
    const key = codexThreadModelStorageKey(hostId, threadId);
    if (!key || codexModelCatalog.status !== "ok") return null;
    const profile = codexRemoteSessionActiveProfile();
    if (Array.isArray(codexModelCatalog.sources) && codexModelCatalog.sources.length
        && !codexModelCatalog.sources.some((source) =>
          source?.id === `${profile?.relayMode === "aggregate" ? "aggregate" : "relay-profile"}:${profile?.id}`)) return null;
    const saved = codexLatestThreadModel(hostId, threadId);
    if (!saved) return null;
    const model = resolveCodexThreadModelSelection(saved);
    if (!model) return null;
    const metadata = codexPlusModelMetadata(model);
    const efforts = modelReasoningEfforts(model).map((entry) => entry.reasoningEffort);
    const effort = saved.effort == null || efforts.includes(saved.effort) ? saved.effort
      : efforts.includes(metadata?.defaultReasoningEffort) ? metadata.defaultReasoningEffort : null;
    return { ...saved, model, effort };
  }

  function codexLatestThreadModel(hostId, threadId) {
    // 兼容旧的按配置保存记录；跨直连/聚合读取同一对话最近一次选择。
    return codexRememberedThreadModels().filter((entry) => {
      try {
        const scope = JSON.parse(entry.key);
        return Array.isArray(scope) && scope[0] === hostId && scope[2] === threadId;
      } catch { return false; }
    }).sort((a, b) => b.at - a.at)[0] || null;
  }

  function codexThreadModelSourceGroup(model) {
    const profile = codexRemoteSessionActiveProfile();
    return codexPlusModelMetadata(model)?.codex_plus_group_id
      || (profile?.relayMode === "pureApi" ? profile.id : "");
  }

  function resolveCodexThreadModelSelection(saved) {
    if (codexModelCatalog.status !== "ok") return "";
    const names = codexPlusModelNames();
    const baseName = (model) => model.startsWith("cpx-") ? model.slice(model.indexOf("/") + 1) : model;
    const matches = names.filter((model) => baseName(model) === baseName(saved.model));
    let groupId = saved.groupId;
    if (!groupId) {
      try {
        const profileId = JSON.parse(saved.key)[1];
        const profile = codexPlusBackendSettings.relayProfiles?.find((entry) => entry.id === profileId);
        if (profile?.relayMode === "pureApi") groupId = profile.id;
      } catch { /* 无来源的旧记录仍可按模型名匹配。 */ }
    }
    const sameSource = groupId && matches.find((model) => codexThreadModelSourceGroup(model) === groupId);
    if (sameSource) return sameSource;
    const compatible = saved.effort == null ? matches : matches.filter((model) =>
      modelReasoningEfforts(model).some((entry) => entry.reasoningEffort === saved.effort));
    const candidates = compatible.length ? compatible : matches;
    return candidates.find((model) => model === baseName(saved.model)) || candidates[0] || "";
  }

  function captureCodexThreadModelSelection(params) {
    const model = params?.model || params?.collaborationMode?.settings?.model;
    if (typeof model !== "string" || !model || codexModelCatalog.status !== "ok"
        || !codexPlusModelNames().includes(model)) return null;
    return { model, effort: params?.effort ?? params?.reasoningEffort
      ?? params?.collaborationMode?.settings?.reasoning_effort ?? null,
      groupId: codexThreadModelSourceGroup(model) };
  }

  function rememberCodexThreadModel(method, params, result, hostId, selection, explicit = false) {
    if (!codexThreadModelObserverActive()) return;
    if (!["thread/settings/update", "turn/start", "thread/start"].includes(String(method))) return;
    const threadId = params?.threadId || result?.thread?.id;
    const key = codexThreadModelStorageKey(hostId, threadId);
    if (selection === undefined) selection = captureCodexThreadModelSelection(params);
    if (!key || !selection || result?.applied === false || result?.error
        || !resolveCodexThreadModelSelection(selection)) return;
    const previous = codexRememberedThreadModels();
    // 发消息可能携带原生回退值；已有明确选择时不能用它覆盖。
    const latest = codexLatestThreadModel(hostId, threadId);
    if (String(method) === "turn/start" && !explicit && latest) return;
    // 自动恢复到替代供应商不能覆盖原始来源，否则切回时将无法找回原选择。
    const retained = latest && codexRetainedThreadModel(hostId, threadId);
    if (!explicit && latest && latest.key !== key && !retained) return;
    if (!explicit && retained && retained.model === selection.model && retained.effort === selection.effort) return;
    const remembered = previous.find((item) => item.key === key);
    // 自动恢复的重复确认不改变记录版本，否则会把三次重试额度不断重置。
    if (remembered && remembered.groupId === selection.groupId && remembered.effort === selection.effort
        && resolveCodexThreadModelSelection(remembered) === resolveCodexThreadModelSelection(selection)) return;
    const entry = { key, ...selection, at: Date.now() };
    try {
      localStorage.setItem("codexPlus.threadModels.v1", JSON.stringify([
        entry, ...previous.filter((item) => item.key !== key),
      ].slice(0, 2000)));
    } catch { /* 存储不可用时继续原生流程。 */ }
  }

  function retainCodexThreadResumeModel(method, params, hostId) {
    if (String(method) !== "thread/resume") return params;
    const saved = codexRetainedThreadModel(hostId, params?.threadId);
    if (!saved) return params;
    const crossProfile = saved.key !== codexThreadModelStorageKey(hostId, params?.threadId);
    if (!crossProfile && params?.model && params.model !== saved.model) return params;
    return { ...params, model: saved.model, reasoningEffort: crossProfile ? saved.effort : params?.reasoningEffort ?? saved.effort };
  }

  async function sendCodexRetainedModelRequest(client, send, method, params, options) {
    if (String(method) === "send-cli-request-for-host" && params?.method) {
      return sendCodexRetainedModelRequest({ hostId: params.hostId || client.hostId },
        (nestedMethod, nestedParams, nestedOptions) => send(method,
          { ...params, method: nestedMethod, params: nestedParams }, nestedOptions),
        String(params.method), params.params, options);
    }
    const profileId = codexRemoteSessionActiveProfile()?.id;
    const key = codexThreadModelStorageKey(client.hostId, params?.threadId);
    const selection = captureCodexThreadModelSelection(params);
    const intent = window.__codexPlusThreadModelSelections?.get(key);
    const explicit = intent && Date.now() - intent.at < 30000
      && resolveCodexThreadModelSelection(intent.selection) === params?.model;
    const pending = window.__codexPlusThreadModelPending || (window.__codexPlusThreadModelPending = new Map());
    const tracks = key && String(method) === "thread/settings/update";
    if (tracks) pending.set(key, (pending.get(key) || 0) + 1);
    try {
      const next = retainCodexThreadResumeModel(method, params, client.hostId);
      const result = await send(method, next, options);
      if (result?.applied === false || result?.error) {
        if (explicit && window.__codexPlusThreadModelSelections?.get(key) === intent) window.__codexPlusThreadModelSelections.delete(key);
      }
      if (profileId === codexRemoteSessionActiveProfile()?.id) {
        const confirmedSelection = explicit && !intent.powerSelection ? intent.selection : selection;
        rememberCodexThreadModel(method, next, result, client.hostId, confirmedSelection, !!explicit);
      }
      return result;
    } catch (error) {
      // 不支持的 RPC 会由原生回退到本地状态；其它失败不能确认用户选择。
      const unsupported = error?.code === -32601 || /method (?:not found|unsupported|not supported)/i.test(String(error?.message || error));
      if (explicit && !unsupported && window.__codexPlusThreadModelSelections?.get(key) === intent) window.__codexPlusThreadModelSelections.delete(key);
      throw error;
    } finally {
      if (tracks) {
        const count = (pending.get(key) || 1) - 1;
        if (count) pending.set(key, count); else pending.delete(key);
      }
    }
  }

  function codexThreadModelPickerContext(node) {
    const fiberKey = Object.keys(node).find((key) => key.startsWith("__reactFiber$"));
    let fiber = fiberKey ? node[fiberKey] : null;
    let picker, conversationId = "", threadId = "", hostId = "", selectedModel = "", selectedEffort = "";
    for (let depth = 0; fiber && depth < 128; depth++, fiber = fiber.return) {
      const props = fiber.memoizedProps || {};
      if (!selectedModel && typeof fiber.key === "string" && codexPlusModelNames().includes(fiber.key)) selectedModel = fiber.key;
      if (!picker && !selectedEffort) selectedEffort = [props.value, fiber.key]
        .find((value) => ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"].includes(value)) || "";
      if (!picker && typeof props.model === "string" && typeof props.onSelectModel === "function"
          && Array.isArray(props.models)) picker = props;
      if (!conversationId && typeof props.conversationId === "string") conversationId = props.conversationId;
      if (!threadId && typeof props.threadId === "string") threadId = props.threadId;
      if (!hostId && typeof props.permissionsHostId === "string") hostId = props.permissionsHostId;
    }
    return { picker, owner: conversationId || threadId, hostId, selectedModel, selectedEffort };
  }

  function observeCodexThreadModelSelection(event) {
    if (!codexThreadModelObserverActive()) return;
    if (!["click", "keydown", "pointerup"].includes(event.type)) return;
    const slider = event.target?.closest?.("[data-reasoning-slider]");
    if (slider && event.type === "click") return;
    if (event.type === "pointerup" && !slider) return;
    if (event.type === "keydown" && !(slider
      ? ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)
      : ["Enter", " "].includes(event.key))) return;
    const row = slider || event.target?.closest?.('[role="menuitemradio"]');
    if (!row || row.getAttribute("aria-disabled") === "true"
        || row.closest('[inert], [hidden], [aria-hidden="true"]')) return;
    const { picker, owner, hostId, selectedModel, selectedEffort } = codexThreadModelPickerContext(row);
    const key = codexThreadModelStorageKey(hostId, owner);
    if (!key || !picker || picker.lockedModelSlug || picker.modelOptionsDisabled || picker.modelSelectionDisabled
        || (slider && (picker.disabled || picker.reasoningEffortDisabled))) return;
    if (!slider && !selectedEffort && !row.closest("[data-model-picker-view]")) return;
    const selection = captureCodexThreadModelSelection({
      model: slider ? picker.model : selectedModel || (selectedEffort ? picker.model : ""),
      effort: slider ? picker.reasoningEffort : selectedEffort || null,
    });
    if (!selection) return;
    const selections = window.__codexPlusThreadModelSelections || (window.__codexPlusThreadModelSelections = new Map());
    // 只记用户激活原生模型行的意图；原生实际选中后才持久化，取消和自动回退不算选择。
    selections.set(key, { selection, powerSelection: !!slider, awaitPowerChange: !!slider && event.type === "keydown",
      effortOnly: !selectedModel, at: Date.now(), confirmedSince: 0 });
    for (const [entryKey, value] of selections) if (Date.now() - value.at >= 30000) selections.delete(entryKey);
    window.setTimeout?.(syncCodexRetainedThreadModels, 300);
  }

  function codexThreadModelObserverActive() {
    return !observeCodexThreadModelSelection.__codexPlusInstalled
      || window.__codexPlusThreadModelSelectionObserver === observeCodexThreadModelSelection;
  }

  function installCodexThreadModelSelectionObserver() {
    if (!codexThreadModelObserverActive()) return;
    if (window.__codexPlusThreadModelSelectionObserver === observeCodexThreadModelSelection) return;
    const previous = window.__codexPlusThreadModelSelectionObserver;
    for (const type of ["click", "keydown", "pointerup"]) {
      if (previous) document.removeEventListener?.(type, previous, true);
      document.addEventListener?.(type, observeCodexThreadModelSelection, true);
    }
    observeCodexThreadModelSelection.__codexPlusInstalled = true;
    window.__codexPlusThreadModelSelectionObserver = observeCodexThreadModelSelection;
    window.__codexPlusThreadModelRestoreStates = new WeakMap();
  }

  function syncCodexRetainedThreadModels() {
    if (!codexThreadModelObserverActive()) return;
    installCodexThreadModelSelectionObserver();
    const states = window.__codexPlusThreadModelRestoreStates || (window.__codexPlusThreadModelRestoreStates = new WeakMap());
    for (const node of document.querySelectorAll("[data-codex-intelligence-trigger]")) {
      const { picker, owner, hostId } = codexThreadModelPickerContext(node);
      const key = codexThreadModelStorageKey(hostId, owner);
      if (window.__codexPlusThreadModelPending?.has(key)) continue;
      const selections = window.__codexPlusThreadModelSelections;
      const intent = selections?.get(key);
      if (intent && Date.now() - intent.at < 30000) {
        // 滑杆组合选择由原生事件提交后读取；不能把拖动预览当成最终选择。
        if (intent.powerSelection && picker) {
          const selection = captureCodexThreadModelSelection({ model: picker.model, effort: picker.reasoningEffort });
          if (selection && (selection.model !== intent.selection.model || selection.effort !== intent.selection.effort)) {
            intent.selection = selection;
            intent.awaitPowerChange = false;
            intent.confirmedSince = 0;
          }
          if (intent.awaitPowerChange) {
            selections.delete(key);
            states.delete(node);
            window.setTimeout?.(syncCodexRetainedThreadModels, 300);
            continue;
          }
        }
        const target = resolveCodexThreadModelSelection(intent.selection);
        if (picker?.model === target && (!intent.effortOnly || picker.reasoningEffort === intent.selection.effort)) {
          if (!intent.confirmedSince) {
            intent.confirmedSince = Date.now();
            window.setTimeout?.(syncCodexRetainedThreadModels, 300);
          } else if (Date.now() - intent.confirmedSince >= 250) {
            rememberCodexThreadModel("thread/settings/update", { threadId: owner }, {}, hostId,
              { ...intent.selection, effort: picker.reasoningEffort ?? null }, true);
            selections.delete(key);
          }
        } else intent.confirmedSince = 0;
        states.delete(node);
        continue;
      }
      if (intent) selections.delete(key);
      const saved = codexRetainedThreadModel(hostId, owner);
      if (!picker || !saved || picker.lockedModelSlug || picker.modelOptionsDisabled || picker.modelSelectionDisabled
          || !picker.models.some((entry) => entry?.model === saved.model)) {
        states.delete(node);
        continue;
      }
      const signature = JSON.stringify([key, picker.model, picker.reasoningEffort, saved.model, saved.effort, saved.at]);
      if (picker.model === saved.model && (saved.effort == null || picker.reasoningEffort === saved.effort)) {
        states.delete(node);
        continue;
      }
      const previous = states.get(node);
      if (!previous || previous.signature !== signature) {
        states.set(node, { signature, since: Date.now(), attempts: 0, inFlight: false });
        window.setTimeout?.(syncCodexRetainedThreadModels, 1600);
        continue;
      }
      // 避免在原生乐观更新、会话加载或用户切换模型过程中抢写。
      if (previous.inFlight || previous.attempts >= 3 || Date.now() - previous.since < 1500) continue;
      previous.attempts += 1;
      previous.inFlight = true;
      const settled = () => {
        previous.inFlight = false;
        previous.since = Date.now();
        // 原生回调可能不返回 Promise，下一轮按实际 picker 状态确认；最多尝试三次。
        if (codexThreadModelObserverActive() && states.get(node) === previous && previous.attempts < 3) window.setTimeout?.(syncCodexRetainedThreadModels, 1600);
      };
      try {
        Promise.resolve(picker.onSelectModel(saved.model, saved.effort)).then(settled, settled);
      } catch { settled(); }
    }
  }

  function codexPerModelContextEnabled() {
    const profile = codexRemoteSessionActiveProfile();
    if (!profile) return false;
    return [profile.modelWindows, profile.modelAutoCompact, profile.modelMetadata]
      .some((value) => typeof value === "string" && value.trim() && value.trim() !== "{}");
  }

  function codexThreadModelRequestState(method, params, result) {
    if (method === "send-cli-request-for-host" && params?.method) {
      return codexThreadModelRequestState(String(params.method), params.params, result);
    }
    const requestMethod = String(method || "");
    const threadId = String(
      params?.threadId
      || params?.conversationId
      || result?.thread?.id
      || result?.threadId
      || ""
    ).trim();
    const model = String(params?.model || result?.thread?.model || "").trim();
    return { requestMethod, threadId, model };
  }

  async function refreshCodexThreadModelBeforeTurn(client, originalSendRequest, method, params, options) {
    if (method === "send-cli-request-for-host" && params?.method) {
      if ([params.hostId, params.host_id].some((host) => typeof host === "string" && host !== "local")) return null;
      return refreshCodexThreadModelBeforeTurn(client,
        (nestedMethod, nestedParams, nestedOptions) => originalSendRequest(method,
          { ...params, method: nestedMethod, params: nestedParams }, nestedOptions),
        String(params.method), params.params, options);
    }
    if (String(method || "") !== "turn/start" || !codexPerModelContextEnabled()) return null;
    const { threadId, model } = codexThreadModelRequestState(method, params);
    if (!threadId || !model) return null;
    const previousModel = client.__codexPlusThreadModels?.get(threadId) || "";
    if (!previousModel || previousModel === model) return null;
    if (client.hostId === "local" && codexManagedSessionConnection()
        && window.__codexPlusConnectionRecoveryPending?.has(threadId)) {
      throw new Error("此会话正在检查连接，请稍后重新发送");
    }
    let resumeParams = { threadId, model };
    resumeParams = applyCodexRemoteSessionProviderOverride("thread/resume", resumeParams);
    resumeParams = applyCodexManagedSessionConnection("thread/resume", resumeParams, client.hostId);
    try {
      await originalSendRequest("thread/resume", resumeParams, options);
      client.__codexPlusThreadModels.set(threadId, model);
      sendCodexPlusDiagnostic("thread_model_context_refreshed", {
        threadId,
        from: previousModel,
        to: model,
      });
      return true;
    } catch (error) {
      sendCodexPlusDiagnostic("thread_model_context_refresh_failed", {
        threadId,
        from: previousModel,
        to: model,
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
      return false;
    }
  }

  function patchAppServerModelRequestClient(client) {
    if (!client || typeof client.sendRequest !== "function") return false;
    registerNativeHostClient(client);
    try {
      if (!Object.isExtensible(client)) return false;
      for (const key of [
        "__codexPlusModelRequestPatch",
        "__codexPlusModelOriginalSendRequest",
        "__codexPlusThreadModels",
        "__codexPlusServiceTierOriginalPrewarmThreadStart",
        "sendRequest",
        "prewarmThreadStart",
      ]) {
        const descriptor = Object.getOwnPropertyDescriptor(client, key);
        if (descriptor && descriptor.writable === false && typeof descriptor.set !== "function") return false;
      }
    } catch {
      return false;
    }
    if (client.__codexPlusModelRequestPatch === codexAppServerModelRequestPatchVersion) return true;
    const nativeSendRequest = (client.__codexPlusModelOriginalSendRequest || client.sendRequest).bind(client);
    client.__codexPlusModelOriginalSendRequest = nativeSendRequest;
    const originalSendRequest = (method, params, options) =>
      sendCodexRetainedModelRequest(client, nativeSendRequest, method, params, options);
    client.__codexPlusThreadModels = client.__codexPlusThreadModels || new Map();
    client.sendRequest = async function codexPlusModelPatchedSendRequest(method, params, options) {
      const managed = codexPlusPluginNativeInterceptClient(method, params, options,
        (nextOptions) => originalSendRequest(method, params, nextOptions), client.hostId);
      if (managed) return await managed;
      const requestMethod = appServerModelRequestMethod(String(method || ""), params);
      let providerRefreshFailed = false;
      const managedAggregateTurn = requestMethod === "turn/start"
        && client.hostId === "local" && !!codexManagedSessionConnection();
      if (codexRemoteSessionProviderRequestMethod(requestMethod)
          && codexRemoteSessionProviderPatchEnabled()
          && window.__codexSessionDeleteBridge
          && !managedAggregateTurn) {
        const settingsLoaded = await loadBackendSettingsState(codexAppServerPreparationTimeoutMs);
        providerRefreshFailed = !settingsLoaded;
        if (providerRefreshFailed) {
          sendCodexPlusDiagnostic("remote_session_provider_refresh_failed", {});
        }
      } else if (codexRemoteSessionProviderRequestMethod(requestMethod)
          && codexRemoteSessionProviderOverrideEnabled()
          && !codexRemoteSessionTargetProvider()) {
        providerRefreshFailed = (await loadCodexModelCatalog())?.status === "failed";
      }
      const configuredParams = providerRefreshFailed ? params
        : normalizeCodexConfiguredModelRequest(String(method || ""), params, client.hostId);
      const providerParams = providerRefreshFailed
        ? params
        : applyCodexRemoteSessionProviderOverride(requestMethod, configuredParams);
      const nextParams = await prepareCodexManagedSessionConnection(String(method || ""),
        providerParams, client.hostId, originalSendRequest);
      const modelContextRefresh = await refreshCodexThreadModelBeforeTurn(
        client,
        originalSendRequest,
        method,
        nextParams,
        options
      );
      const managedTurn = await sendCodexManagedSessionTurn(client, originalSendRequest,
        String(method || ""), nextParams, options, modelContextRefresh);
      if (managedTurn) return managedTurn.result;
      const result = await originalSendRequest(method, nextParams, options);
      noteCodexManagedSessionStart(String(method || ""), nextParams, result, client.hostId);
      const threadState = codexThreadModelRequestState(String(method || ""), nextParams, result);
      if (modelContextRefresh !== false && threadState.threadId && threadState.model
          && ["thread/start", "thread/resume", "turn/start"].includes(threadState.requestMethod)) {
        client.__codexPlusThreadModels.set(threadState.threadId, threadState.model);
      }
      if (!codexPlusModelUnlockEnabled() || !["list-models-for-host", "model/list"].includes(requestMethod)) return result;
      if (!codexPlusModelNames().length) await loadCodexModelCatalog();
      return patchAppServerModelResult(requestMethod, result, modelResponseHostId(params, client));
    };
    if (typeof client.prewarmThreadStart === "function") {
      const originalPrewarmThreadStart = (client.__codexPlusServiceTierOriginalPrewarmThreadStart
        || client.prewarmThreadStart).bind(client);
      client.__codexPlusServiceTierOriginalPrewarmThreadStart = originalPrewarmThreadStart;
      client.prewarmThreadStart = async function codexPlusServiceTierPrewarmThreadStart(params, options) {
        return await prewarmCodexManagedSession(client, originalPrewarmThreadStart, params, options);
      };
    }
    client.__codexPlusModelRequestPatch = codexAppServerModelRequestPatchVersion;
    return true;
  }

  // issue #2177：Codex 26.908 把 AppServerRequestClient 类藏进模块闭包且不再导出，
  // 渲染层扫描在新版上永远 not_found，直接改写 dispatcher 又会撞上不可写的 RPC stub。
  // 改为两段式接管：这里先用纯文本定位算出 sendRequest 的断点坐标（按 UTF-16 计数，
  // 与 V8 断点坐标语义一致），launcher 侧 bridge.rs 再用 CDP Debugger 按坐标下条件断点，
  // 命中时把类构造器挂到 window.__codexPlusAppServerClientClass，随后对原型套用与
  // 实例版完全一致的请求补丁。断点条件 `!window.__codexPlusAppServerClientClass`
  // 保证页面重载后自动重新捕获，且每次页面生命周期内只暂停一次。
    function locateCodexAppServerClientBreakpoint(text) {
    if (typeof text !== "string" || !text) return null;
    const markerIdx = text.indexOf(codexAppServerClientCaptureMarker);
    if (markerIdx < 0) return null;
    const anchorIdx = text.lastIndexOf(codexAppServerClientCaptureAnchor, markerIdx);
    if (anchorIdx < 0 || markerIdx - anchorIdx > 220) return null;
    const braceIdx = text.indexOf("{", anchorIdx);
    if (braceIdx < 0) return null;
    let lineNumber = 0;
    let lastNewline = -1;
    for (let i = 0; i < braceIdx; i++) {
      if (text.charCodeAt(i) === 10) {
        lineNumber += 1;
        lastNewline = i;
      }
    }
    return { lineNumber, columnNumber: braceIdx - lastNewline - 1 };
  }

  // issue #2399：抓取目标不能再按 asset 文件名前缀写死。Codex 26.930 把
  // AppServerRequestClient 从 `app-initial-*.js` 搬到了 `app-shared-*.js`，
  // 按前缀找资产必然 anchor_missing，模型白名单解锁在整条 app-server 路径上失效。
  // 加一项前缀只治当前这一版，下次改名又会失灵，所以改成**按内容找类定义**：
  // 遍历已加载的 app asset，谁包含 marker 文本谁就是目标，两代产物都能覆盖。
  // 排序只做「可能的更靠前」的启发式，不影响正确性；命中全靠文本匹配。
  const codexAppServerClientBundleHints = ["app-shared-", "app-initial-", "app-main-", "chatg"];
  const codexAppServerClientAssetFetchLimit = 24;

  function codexAppServerClientAssetCandidateUrls() {
    const urls = codexAppAssetCandidateUrls();
    const rank = (url) => {
      const name = (url.split("/").pop() || "").toLowerCase();
      const hint = codexAppServerClientBundleHints.findIndex((part) => name.includes(part));
      return hint < 0 ? codexAppServerClientBundleHints.length : hint;
    };
    // 按「像主 bundle」在前、体积（URL 长度做代理）大的在前排序，再截断。
    // 全量 fetch 所有 app asset 会重蹈 #1960 的覆辙，所以限制尝试数量。
    return urls
      .slice()
      .sort((left, right) => rank(left) - rank(right) || right.length - left.length)
      .slice(0, codexAppServerClientAssetFetchLimit);
  }

  // 返回 { url, location }；找不到返回 null。diagnostics 由调用方按命中/未命中归类。
  async function locateCodexAppServerClientInAssets() {
    const urls = codexAppServerClientAssetCandidateUrls();
    if (urls.length === 0) return { urls, hit: null };
    // 先走廉价的 asset loader（有 30s 失败冷却与重试上限），脚本里已经内联了
    // asset 名时能直接命中，省掉整轮 fetch；不可用就退回到全量文本匹配。
    const hinted = resolveCodexAppServerClientHintedUrl();
    const ordered = hinted
      ? [hinted, ...urls.filter((url) => url !== hinted)]
      : urls;
    for (const url of ordered) {
      try {
        const text = await fetch(url).then((response) => response.ok ? response.text() : "");
        if (!text) continue;
        const location = locateCodexAppServerClientBreakpoint(text);
        if (location) return { urls, hit: { url, location } };
      } catch {
        // 单个 asset 拉取失败不影响其余候选，继续下一个。
      }
    }
    return { urls, hit: null };
  }

  // 兼容旧路径：asset 名确实内联在入口脚本里时，用 loader 的缓存直接拿到 URL，
  // 不必为了它把所有 asset 拉一遍。找不到就返回空串，交给内容匹配兜底。
  function resolveCodexAppServerClientHintedUrl() {
    for (const prefix of codexAppServerClientBundleHints) {
      const url = codexAppAssetUrl(prefix);
      if (url) return url;
    }
    return "";
  }

    let codexAppServerClientCaptureStarted = false;
  async function installCodexAppServerClientCapture() {
    if (codexAppServerClientCaptureStarted || window.__codexPlusAppServerClientCapture) return;
    codexAppServerClientCaptureStarted = true;
    try {
      if (typeof fetch !== "function") return;
      const { urls, hit } = await locateCodexAppServerClientInAssets();
      if (!hit) {
        // 区分「一个候选资产都没有」和「有资产但没有类定义」：
        // 前者是页面还没加载完，后者才是 Codex 真的改了产物形状。
        sendCodexPlusDiagnostic("app_server_client_capture_locate_failed", {
          reason: urls.length === 0 ? "asset_url_missing" : "anchor_missing",
          candidateCount: urls.length,
        });
        return;
      }
      const urlRegex = hit.url.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      window.__codexPlusAppServerClientCapture = { urlRegex, ...hit.location };
      sendCodexPlusDiagnostic("app_server_client_capture_located", {
        lineNumber: hit.location.lineNumber,
        columnNumber: hit.location.columnNumber,
        asset: (hit.url.split("/").pop() || "").split("?")[0],
      });
    } catch (error) {
      codexAppServerClientCaptureStarted = false;
      sendCodexPlusDiagnostic("app_server_client_capture_locate_failed", {
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
    }
  }

    function installCodexAppServerClientPrototypePatch() {
    if (window.__codexPlusAppServerClientPrototypePatchInstalled === codexAppServerModelRequestPatchVersion) return true;
    const wanted = codexPlusModelUnlockEnabled()
      || codexPluginMarketplacePatchEnabled()
      || (codexPlusBackendSettingsLoaded && codexRemoteSessionProviderPatchEnabled())
      || codexPlusSettings().sessionDelete;
    if (!wanted) return false;
    const klass = window.__codexPlusAppServerClientClass;
    if (!klass || typeof klass !== "function" || !klass.prototype) return false;
    const proto = klass.prototype;
    if (proto.__codexPlusModelRequestPatch === codexAppServerModelRequestPatchVersion) {
      window.__codexPlusAppServerClientPrototypePatchInstalled = codexAppServerModelRequestPatchVersion;
      return true;
    }
    try {
      const descriptor = Object.getOwnPropertyDescriptor(proto, "sendRequest");
      if (!descriptor || descriptor.writable === false) {
        sendCodexPlusDiagnostic("app_server_client_prototype_patch_skipped", {});
        window.__codexPlusAppServerClientPrototypePatchInstalled = codexAppServerModelRequestPatchVersion;
        return false;
      }
    } catch {
      window.__codexPlusAppServerClientPrototypePatchInstalled = codexAppServerModelRequestPatchVersion;
      return false;
    }
    const nativeSendRequest = proto.__codexPlusModelOriginalSendRequest || proto.sendRequest;
    proto.__codexPlusModelOriginalSendRequest = nativeSendRequest;
    const originalSendRequest = function (method, params, options) {
      return sendCodexRetainedModelRequest(this, nativeSendRequest.bind(this), method, params, options);
    };
    proto.__codexPlusThreadModels = proto.__codexPlusThreadModels || new Map();
    proto.sendRequest = async function codexPlusModelPatchedSendRequest(method, params, options) {
      const client = this;
      registerNativeHostClient(client);
      const managed = codexPlusPluginNativeInterceptClient(method, params, options,
        (nextOptions) => originalSendRequest.call(client, method, params, nextOptions), client.hostId);
      if (managed) return await managed;
      const requestMethod = appServerModelRequestMethod(String(method || ""), params);
      let providerRefreshFailed = false;
      const managedAggregateTurn = requestMethod === "turn/start"
        && client.hostId === "local" && !!codexManagedSessionConnection();
      if (codexRemoteSessionProviderRequestMethod(requestMethod)
          && codexRemoteSessionProviderPatchEnabled()
          && window.__codexSessionDeleteBridge
          && !managedAggregateTurn) {
        const settingsLoaded = await loadBackendSettingsState(codexAppServerPreparationTimeoutMs);
        providerRefreshFailed = !settingsLoaded;
        if (providerRefreshFailed) {
          sendCodexPlusDiagnostic("remote_session_provider_refresh_failed", {});
        }
      } else if (codexRemoteSessionProviderRequestMethod(requestMethod)
          && codexRemoteSessionProviderOverrideEnabled()
          && !codexRemoteSessionTargetProvider()) {
        providerRefreshFailed = (await loadCodexModelCatalog())?.status === "failed";
      }
      const configuredParams = providerRefreshFailed ? params
        : normalizeCodexConfiguredModelRequest(String(method || ""), params, client.hostId);
      const providerParams = providerRefreshFailed
        ? params
        : applyCodexRemoteSessionProviderOverride(requestMethod, configuredParams);
      const nextParams = await prepareCodexManagedSessionConnection(String(method || ""),
        providerParams, client.hostId, originalSendRequest.bind(client));
      const modelContextRefresh = await refreshCodexThreadModelBeforeTurn(
        client,
        originalSendRequest.bind(client),
        method,
        nextParams,
        options
      );
      const managedTurn = await sendCodexManagedSessionTurn(client, originalSendRequest.bind(client),
        String(method || ""), nextParams, options, modelContextRefresh);
      if (managedTurn) return managedTurn.result;
      const result = await originalSendRequest.call(client, method, nextParams, options);
      noteCodexManagedSessionStart(String(method || ""), nextParams, result, client.hostId);
      const threadState = codexThreadModelRequestState(String(method || ""), nextParams, result);
      if (modelContextRefresh !== false && threadState.threadId && threadState.model
          && ["thread/start", "thread/resume", "turn/start"].includes(threadState.requestMethod)) {
        client.__codexPlusThreadModels.set(threadState.threadId, threadState.model);
      }
      if (!codexPlusModelUnlockEnabled() || !["list-models-for-host", "model/list"].includes(requestMethod)) return result;
      if (!codexPlusModelNames().length) await loadCodexModelCatalog();
      return patchAppServerModelResult(requestMethod, result, modelResponseHostId(params, client));
    };
    if (typeof proto.prewarmThreadStart === "function") {
      const originalPrewarmThreadStart = proto.__codexPlusServiceTierOriginalPrewarmThreadStart
        || proto.prewarmThreadStart;
      proto.__codexPlusServiceTierOriginalPrewarmThreadStart = originalPrewarmThreadStart;
      proto.prewarmThreadStart = async function codexPlusServiceTierPrewarmThreadStart(params, options) {
        return await prewarmCodexManagedSession(this, originalPrewarmThreadStart.bind(this), params, options);
      };
    }
    proto.__codexPlusModelRequestPatch = codexAppServerModelRequestPatchVersion;
    window.__codexPlusAppServerClientPrototypePatchInstalled = codexAppServerModelRequestPatchVersion;
    sendCodexPlusDiagnostic("app_server_client_prototype_patch_installed", {});
    return true;
  }

  const appServerModelRequestPatchMaxMisses = 8;
  const appServerModelRequestPatchMaxRetryDelayMs = 30000;
  let appServerModelRequestPatchMissCount = 0;
  let appServerModelRequestPatchDisabled = false;
  let appServerModelRequestPatchPromise = null;
  let appServerModelRequestPatchRetryTimer = 0;
  let appServerModelRequestPatchRetryDelayMs = 250;

  function scheduleAppServerModelRequestPatchRetry() {
    if (!codexRemoteSessionProviderPatchEnabled()) return;
    if (appServerModelRequestPatchRetryTimer) return;
    // issue #2256/#2255：固定 250ms 重试在 Codex 改 asset 命名后变成每秒 4 轮的全量
    // rescan（每轮 fetch 全部 app asset）。改为指数退避， miss 计满后由熔断停掉。
    appServerModelRequestPatchRetryTimer = window.setTimeout(() => {
      appServerModelRequestPatchRetryTimer = 0;
      installAppServerModelRequestPatch();
    }, appServerModelRequestPatchRetryDelayMs);
    appServerModelRequestPatchRetryDelayMs = Math.min(appServerModelRequestPatchRetryDelayMs * 4, appServerModelRequestPatchMaxRetryDelayMs);
  }

  function noteAppServerModelRequestPatchMiss(event, detail) {
    appServerModelRequestPatchMissCount += 1;
    // installAppServerModelRequestPatch() runs on every model-whitelist
    // refresh tick (~120ms). On Codex builds where the app-server module was
    // renamed/removed (e.g. 26.623+, issue #1324) this layer never succeeds
    // and would otherwise emit the same diagnostic on every tick forever.
    // Report the first miss so telemetry still captures the cause, then stay
    // quiet, and finally disable this layer once it is clearly unavailable.
    // This is a graceful fallback: the remaining whitelist layers (Statsig
    // config / React state / response JSON patch) keep injecting the custom
    // models on their own.
    if (appServerModelRequestPatchMissCount === 1) {
      sendCodexPlusDiagnostic(event, detail);
    }
    // issue #2256：provider 重试路径以前在这里提前 return，绕过下面的 maxMisses
    // 熔断，失败变成 250ms 无限重试（每轮全量 rescan 全部 app assets）。
    // 现在两个路径统一计数：先按 maxMisses 熔断，未熔断时再走指数退避重试。
    if (appServerModelRequestPatchMissCount >= appServerModelRequestPatchMaxMisses && !appServerModelRequestPatchDisabled) {
      appServerModelRequestPatchDisabled = true;
      clearTimeout(appServerModelRequestPatchRetryTimer);
      appServerModelRequestPatchRetryTimer = 0;
      sendCodexPlusDiagnostic("model_app_server_request_patch_skipped", {
        misses: appServerModelRequestPatchMissCount,
        lastEvent: event,
      });
      return;
    }
    if (!appServerModelRequestPatchDisabled) {
      scheduleAppServerModelRequestPatchRetry();
    }
  }

  function installAppServerModelRequestPatch() {
    if (window.__codexPlusAppServerModelRequestPatchInstalled === codexAppServerModelRequestPatchVersion) return;
    if (appServerModelRequestPatchDisabled) return;
    if (appServerModelRequestPatchPromise) return;
    const patch = async () => {
      try {
        const { modules, candidates, sources, discovery } = await loadAppServerRequestCandidates();
        if (modules.length === 0) {
          noteAppServerModelRequestPatchMiss("model_app_server_request_patch_skipped", {
            reason: "app_server_request_assets_missing",
          });
          return;
        }
        let patchedCount = 0;
        for (const candidate of candidates) {
          if (patchAppServerModelRequestClient(candidate)) patchedCount += 1;
        }
        if (patchedCount > 0) {
          clearTimeout(appServerModelRequestPatchRetryTimer);
          appServerModelRequestPatchRetryTimer = 0;
          appServerModelRequestPatchMissCount = 0;
          appServerModelRequestPatchRetryDelayMs = 250;
          window.__codexPlusAppServerModelRequestPatchInstalled = codexAppServerModelRequestPatchVersion;
          sendCodexPlusDiagnostic("model_app_server_request_patch_installed", {
            moduleCount: modules.length,
            candidateCount: candidates.length,
            patchedCount,
            sources,
            discovery,
          });
        } else {
          noteAppServerModelRequestPatchMiss("model_app_server_request_patch_not_found", {
            moduleCount: modules.length,
            candidateCount: candidates.length,
            sources,
            discovery,
          });
        }
      } catch (error) {
        noteAppServerModelRequestPatchMiss("model_app_server_request_patch_failed", {
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
      }
    };
    appServerModelRequestPatchPromise = patch().finally(() => {
      appServerModelRequestPatchPromise = null;
    });
    void appServerModelRequestPatchPromise;
  }

  function ensureCodexModelWhitelistInstalls() {
    if (codexPlusModelUnlockEnabled()
        || codexPluginMarketplacePatchEnabled()
        || (codexPlusBackendSettingsLoaded && codexRemoteSessionProviderPatchEnabled())
          || codexPlusSettings().sessionDelete) {
      installAppServerModelRequestPatch();
      void installCodexAppServerClientCapture().catch(() => {});
    }
    void installDictationSupportPatch();
    if (!codexPlusModelUnlockEnabled()) return;
    installModelJsonResponsePatch();
    patchAppServerModelMessages();
  }

  function runCodexModelWhitelistRefreshPass() {
    syncCodexConfiguredDefaultModel();
    syncCodexRetainedThreadModels();
    if (!codexPlusModelUnlockEnabled()) {
      if (codexAggregateAllowedModelSet("local", true)) refreshCachedLocalModelQueries();
      syncCodexAggregateSelectedModel();
      return false;
    }
    try {
      patchStatsigModelWhitelist();
      refreshCachedLocalModelQueries();
      syncCodexAggregateSelectedModel();
      installAppServerModelRequestPatch();
    } catch (error) {
      window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
      window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
    }
    return false;
  }

  function scheduleCodexModelWhitelistRefresh(durationMs = 2500) {
    if (!codexPlusModelUnlockEnabled() && !codexAggregateAllowedModelSet("local", true)) return;
    codexModelWhitelistRefreshUntil = Math.max(codexModelWhitelistRefreshUntil, Date.now() + durationMs);
    if (codexModelWhitelistRefreshTimer) return;
    sendCodexPlusDiagnostic("model_whitelist_refresh_scheduled", { durationMs });
    const tick = () => {
      codexModelWhitelistRefreshTimer = 0;
      runCodexModelWhitelistRefreshPass();
      if (Date.now() < codexModelWhitelistRefreshUntil) {
        codexModelWhitelistRefreshTimer = window.setTimeout(tick, 120);
      }
    };
    tick();
  }

  function refreshCodexModelWhitelistFromScan(mutations) {
    ensureCodexModelWhitelistInstalls();
    if (!codexPlusModelNames().length) {
      loadCodexModelCatalog();
      runCodexModelWhitelistRefreshPass();
      return;
    }
    runCodexModelWhitelistRefreshPass();
  }

  function threadIdVariants(sessionId) {
    if (typeof sessionId !== "string" || !sessionId.trim()) return [];
    const id = sessionId.trim();
    const bareId = id.startsWith("local:") ? id.slice("local:".length) : id;
    return uniqueValues([id, bareId, `local:${bareId}`]);
  }

  function sessionKey(sessionId) {
    const variants = threadIdVariants(sessionId);
    const bareId = variants.find((id) => !id.startsWith("local:"));
    return bareId || variants[0] || "";
  }

  function uuidV7TimestampMs(sessionId) {
    const id = sessionKey(sessionId).replaceAll("-", "");
    if (!/^[0-9a-fA-F]{12}/.test(id)) return 0;
    const timestamp = Number.parseInt(id.slice(0, 12), 16);
    return Number.isFinite(timestamp) ? timestamp : 0;
  }
