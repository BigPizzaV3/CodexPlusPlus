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
  let codexModelWhitelistRefreshTimer = 0;
  let codexModelWhitelistRefreshUntil = 0;
  const codexPlusModelListRequestIds = new Set();

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
      statusSummary: (state = {}) => {
        const summaryState = { ...codexServiceTierState, ...state };
        return serviceTierStatusMessage(
          summaryState.controlMode,
          summaryState.threadMode,
          summaryState.effectiveMode,
          summaryState.defaultMode,
          summaryState.effectiveServiceTier,
          summaryState.serviceTierSource
        );
      },
      resolveInheritedServiceTier: () => resolveInheritedServiceTier(),
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
        codexModelCatalogPromise = null;
      },
      setBackendSettings: (settings = {}) => {
        codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
        codexPlusBackendSettingsLoaded = true;
      },
      providerPatchEnabled: () => codexRemoteSessionProviderPatchEnabled(),
      providerNormalizationEnabled: () => codexRemoteSessionProviderNormalizationEnabled(),
      setServiceTierState: (state = {}) => {
        codexServiceTierState = { ...codexServiceTierState, ...state };
      },
      setThreadState: (state = {}) => {
        localStorage.setItem(codexThreadServiceTierKey, JSON.stringify({
          version: codexThreadServiceTierVersion,
          mode: "inherit",
          defaultMode: "inherit",
          entries: {},
          ...state,
        }));
      },
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
      codexModelCatalog.default_model,
      codexModelCatalog.model,
      ...(Array.isArray(codexModelCatalog.models) ? codexModelCatalog.models : []),
    ]);
  }

  async function loadCodexModelCatalog(force = false) {
    if (!force && codexModelCatalogPromise) return codexModelCatalogPromise;
    if (!force && codexModelCatalogLoadedAt && Date.now() - codexModelCatalogLoadedAt < 10000) return codexModelCatalog;
    codexModelCatalogPromise = postJson("/codex-model-catalog", {})
      .then(async (result) => {
        codexModelCatalog = result && typeof result === "object" ? result : { status: "failed", model: "", default_model: "", model_provider: "", codex_model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
        if ((!codexModelCatalog.models || codexModelCatalog.models.length === 0) && codexModelCatalog.status === "not_configured") {
          try {
            const settingsPromise = postJson("/settings/get", {});
            const timeoutPromise = new Promise((_, reject) => setTimeout(() => reject(new Error("fallback timeout")), 3000));
            const settingsResp = await Promise.race([settingsPromise, timeoutPromise]);
            if (settingsResp && settingsResp.relayProfiles && Array.isArray(settingsResp.relayProfiles)) {
              const activeId = settingsResp.activeRelayId || "";
              const profile = settingsResp.relayProfiles.find(p => p.id === activeId);
              if (profile && profile.modelList) {
                const extraModels = profile.modelList.split(/[\r\n,]+/).map(s => s.trim()).filter(Boolean);
                if (extraModels.length > 0) {
                  codexModelCatalog.models = extraModels;
                  codexModelCatalog.default_model = codexModelCatalog.default_model || extraModels[0];
                  sendCodexPlusDiagnostic("model_catalog_fallback_applied", { count: extraModels.length });
                }
              }
            }
          } catch (fallbackError) {
            sendCodexPlusDiagnostic("model_catalog_fallback_error", { error: String(fallbackError?.message || fallbackError) });
          }
        }
        codexModelCatalogLoadedAt = Date.now();
        renderCodexPlusMenu();
        scheduleCodexModelWhitelistRefresh();
        return codexModelCatalog;
      })
      .catch((error) => {
        codexModelCatalog = { status: "failed", message: String(error?.message || error), model: "", default_model: "", model_provider: "", codex_model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
        codexModelCatalogLoadedAt = Date.now();
        return codexModelCatalog;
      })
      .finally(() => {
        codexModelCatalogPromise = null;
      });
    return codexModelCatalogPromise;
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
        const shouldAddUltra = /sol|terra|gpt-5\.6|gpt-5\.5|gpt-5\.4|deepseek/i.test(String(modelName || ""));
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
    for (const key of ["displayName", "description", "defaultReasoningEffort"]) {
      if (typeof metadata[key] === "string" && metadata[key] && descriptor[key] !== metadata[key]) {
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

  function patchModelNameArray(models) {
    if (!stringArrayLooksPatchable(models)) return false;
    const customModels = codexPlusModelNames();
    if (!customModels.length) return false;
    let changed = false;
    customModels.forEach((modelName) => {
      if (!models.includes(modelName)) {
        models.push(modelName);
        changed = true;
      }
    });
    return changed;
  }

  function patchModelArray(models, allowEmpty = false) {
    if (!modelArrayLooksPatchable(models, allowEmpty)) return false;
    const customModels = codexPlusModelNames();
    if (!customModels.length) return false;
    let changed = false;
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
    return changed;
  }

  function patchModelContainer(value) {
    if (!value || typeof value !== "object") return false;
    let changed = false;
    if (patchModelArray(value.models, "defaultModel" in value || "availableModels" in value)) changed = true;
    if (patchModelNameArray(value.models)) changed = true;
    if (patchModelArray(value.data)) changed = true;
    if (patchModelArray(value.result)) changed = true;
    if (patchModelArray(value.pages?.[0]?.data)) changed = true;
    if (patchModelArray(value.result?.data)) changed = true;
    if (patchModelArray(value.result?.models)) changed = true;
    if (patchModelArray(value.message?.result?.data)) changed = true;
    if (patchModelArray(value.message?.result?.models)) changed = true;
    const names = codexPlusModelNames();
    if (value.availableModels instanceof Set) {
      names.forEach((name) => {
        if (!value.availableModels.has(name)) {
          value.availableModels.add(name);
          changed = true;
        }
      });
    }
    if (value.available_models instanceof Set) {
      names.forEach((name) => {
        if (!value.available_models.has(name)) {
          value.available_models.add(name);
          changed = true;
        }
      });
    }
    if (Array.isArray(value.availableModels)) {
      names.forEach((name) => {
        if (!value.availableModels.includes(name)) {
          value.availableModels.push(name);
          changed = true;
        }
      });
    }
    if (Array.isArray(value.available_models)) {
      names.forEach((name) => {
        if (!value.available_models.includes(name)) {
          value.available_models.push(name);
          changed = true;
        }
      });
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
      patchModelContainer(payload);
    } catch (error) {
      window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
      window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
    }
    return payload;
  }

  function installModelJsonResponsePatch() {
    if (window.__codexPlusModelJsonResponsePatchInstalled === "1") return;
    window.__codexPlusModelJsonResponsePatchInstalled = "1";
    window.__codexPlusModelJsonResponseOriginals = window.__codexPlusModelJsonResponseOriginals || {};
    const originals = window.__codexPlusModelJsonResponseOriginals;
    originals.responseJson = originals.responseJson || Response.prototype.json;
    if (typeof originals.responseJson !== "function") return;
    Response.prototype.json = async function codexPlusPatchedResponseJson(...args) {
      const payload = await originals.responseJson.apply(this, args);
      return await patchModelJsonResponse(payload);
    };
  }

  function patchStatsigModelDynamicConfig(config) {
    const names = codexPlusModelNames();
    const value = config?.value;
    if (!names.length || !value || typeof value !== "object") return config;
    const availableModels = Array.isArray(value.available_models) ? [...value.available_models] : [];
    let changed = false;
    names.forEach((name) => {
      if (!availableModels.includes(name)) {
        availableModels.push(name);
        changed = true;
      }
    });
    if (!changed) return config;
    const nextValue = { ...value, available_models: availableModels };
    try {
      config.value = nextValue;
    } catch {
      return { ...config, value: nextValue };
    }
    return config;
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
      if (!client.__codexPlusModelWhitelistPatched) {
        const originalGetDynamicConfig = client.getDynamicConfig.bind(client);
        client.getDynamicConfig = (name, options) => {
          const result = originalGetDynamicConfig(name, options);
          return String(name) === "107580212" ? patchStatsigModelDynamicConfig(result) : result;
        };
        client.__codexPlusModelWhitelistPatched = true;
      }
      try {
        patchStatsigModelDynamicConfig(client.getDynamicConfig("107580212", { disableExposureLog: true }));
      } catch {
      }
    });
  }

  function patchAppServerModelMessages() {
    if (window.__codexPlusModelMessagePatchInstalled) return;
    window.__codexPlusModelMessagePatchInstalled = true;
    window.addEventListener("codex-message-from-view", (event) => {
      try {
        const detail = event?.detail;
        const request = detail?.request;
        if (detail?.type === "mcp-request" && request?.method === "model/list") {
          request.params = { ...(request.params || {}), includeHidden: true };
          if (request.id != null) {
            const requestId = String(request.id);
            codexPlusModelListRequestIds.add(requestId);
            if (codexPlusModelListRequestIds.size > 64) {
              codexPlusModelListRequestIds.delete(codexPlusModelListRequestIds.values().next().value);
            }
            window.setTimeout(() => codexPlusModelListRequestIds.delete(requestId), 30_000);
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

  function patchMcpModelResponseData(data) {
    if (!codexPlusModelUnlockEnabled()) return false;
    if (data?.type !== "mcp-response") return false;
    const message = data.message || data.response;
    const requestId = message?.id != null ? String(message.id) : "";
    if (codexPlusModelListRequestIds.size === 0 || !codexPlusModelListRequestIds.has(requestId)) return false;
    codexPlusModelListRequestIds.delete(requestId);
    let changed = false;
    if (patchModelArray(message?.result?.data, true)) changed = true;
    if (patchModelArray(message?.result?.models, true)) changed = true;
    return changed;
  }

  function appServerModelRequestMethod(method, params) {
    if (method === "send-cli-request-for-host" && params?.method) return String(params.method);
    if (method === "vscode://codex/list-plugins") return "list-plugins";
    if (method === "vscode://codex/plugin/install") return "install-plugin";
    if (method === "vscode://codex/plugin/uninstall") return "uninstall-plugin";
    if (method === "plugin/list") return "list-plugins";
    if (method === "plugin/install") return "install-plugin";
    if (method === "plugin/uninstall") return "uninstall-plugin";
    return String(method || "");
  }

  function patchAppServerModelResult(method, result) {
    if (method !== "list-models-for-host") return result;
    try {
      if (Array.isArray(result)) patchModelArray(result, true);
      if (Array.isArray(result?.data)) patchModelArray(result.data, true);
      if (Array.isArray(result?.models)) patchModelArray(result.models, true);
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

  function codexPerModelContextEnabled() {
    const profile = codexRemoteSessionActiveProfile();
    if (!profile) return false;
    return [profile.modelWindows, profile.modelAutoCompact, profile.modelMetadata]
      .some((value) => typeof value === "string" && value.trim() && value.trim() !== "{}");
  }

  function codexThreadModelRequestState(method, params, result) {
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
    if (String(method || "") !== "turn/start" || !codexPerModelContextEnabled()) return null;
    const { threadId, model } = codexThreadModelRequestState(method, params);
    if (!threadId || !model) return null;
    const previousModel = client.__codexPlusThreadModels?.get(threadId) || "";
    if (!previousModel || previousModel === model) return null;
    let resumeParams = { threadId, model };
    resumeParams = applyCodexRemoteSessionProviderOverride("thread/resume", resumeParams);
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
    const originalSendRequest = client.__codexPlusModelOriginalSendRequest || client.sendRequest.bind(client);
    client.__codexPlusModelOriginalSendRequest = originalSendRequest;
    client.__codexPlusThreadModels = client.__codexPlusThreadModels || new Map();
    client.sendRequest = async function codexPlusModelPatchedSendRequest(method, params, options) {
      const requestMethod = appServerModelRequestMethod(String(method || ""), params);
      let providerRefreshFailed = false;
      if (codexRemoteSessionProviderRequestMethod(requestMethod)
          && codexRemoteSessionProviderPatchEnabled()
          && window.__codexSessionDeleteBridge) {
        const settingsLoaded = await loadBackendSettingsState();
        providerRefreshFailed = !settingsLoaded;
        if (providerRefreshFailed) {
          sendCodexPlusDiagnostic("remote_session_provider_refresh_failed", {});
        }
      } else if (codexRemoteSessionProviderRequestMethod(requestMethod)
          && codexRemoteSessionProviderOverrideEnabled()
          && !codexRemoteSessionTargetProvider()) {
        await loadCodexModelCatalog();
      }
      const providerParams = providerRefreshFailed
        ? params
        : applyCodexRemoteSessionProviderOverride(requestMethod, params);
      const nextParams = applyCodexServiceTierRequestOnly(requestMethod, providerParams);
      const modelContextRefresh = await refreshCodexThreadModelBeforeTurn(
        client,
        originalSendRequest,
        method,
        nextParams,
        options
      );
      const result = await originalSendRequest(method, nextParams, options);
      const threadState = codexThreadModelRequestState(requestMethod, nextParams, result);
      if (modelContextRefresh !== false && threadState.threadId && threadState.model
          && ["thread/start", "thread/resume", "turn/start"].includes(threadState.requestMethod)) {
        client.__codexPlusThreadModels.set(threadState.threadId, threadState.model);
      }
      if (!codexPlusModelUnlockEnabled()) return result;
      if (!codexPlusModelNames().length) await loadCodexModelCatalog();
      return patchAppServerModelResult(requestMethod, result);
    };
    if (typeof client.prewarmThreadStart === "function"
        && !client.__codexPlusServiceTierOriginalPrewarmThreadStart) {
      const originalPrewarmThreadStart = client.prewarmThreadStart.bind(client);
      client.__codexPlusServiceTierOriginalPrewarmThreadStart = originalPrewarmThreadStart;
      client.prewarmThreadStart = async function codexPlusServiceTierPrewarmThreadStart(params, options) {
        const nextParams = applyCodexServiceTierRequestOnly("thread/start", params);
        return originalPrewarmThreadStart(nextParams, options);
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

    let codexAppServerClientCaptureStarted = false;
  async function installCodexAppServerClientCapture() {
    if (codexAppServerClientCaptureStarted || window.__codexPlusAppServerClientCapture) return;
    codexAppServerClientCaptureStarted = true;
    try {
      if (typeof fetch !== "function") return;
      const url = codexAppAssetUrl("app-initial-") || await codexAppAssetUrlFromScriptText("app-initial-");
      if (!url) {
        sendCodexPlusDiagnostic("app_server_client_capture_locate_failed", { reason: "asset_url_missing" });
        return;
      }
      const response = await fetch(url);
      const text = response.ok ? await response.text() : "";
      const location = locateCodexAppServerClientBreakpoint(text);
      if (!location) {
        sendCodexPlusDiagnostic("app_server_client_capture_locate_failed", { reason: "anchor_missing" });
        return;
      }
      const urlRegex = url.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      window.__codexPlusAppServerClientCapture = { urlRegex, ...location };
      sendCodexPlusDiagnostic("app_server_client_capture_located", {
        lineNumber: location.lineNumber,
        columnNumber: location.columnNumber,
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
      || (codexPlusBackendSettingsLoaded && codexRemoteSessionProviderPatchEnabled())
      || codexPlusSettings().serviceTierControls;
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
    const originalSendRequest = proto.__codexPlusModelOriginalSendRequest || proto.sendRequest;
    proto.__codexPlusModelOriginalSendRequest = originalSendRequest;
    proto.__codexPlusThreadModels = proto.__codexPlusThreadModels || new Map();
    proto.sendRequest = async function codexPlusModelPatchedSendRequest(method, params, options) {
      const client = this;
      const requestMethod = appServerModelRequestMethod(String(method || ""), params);
      let providerRefreshFailed = false;
      if (codexRemoteSessionProviderRequestMethod(requestMethod)
          && codexRemoteSessionProviderPatchEnabled()
          && window.__codexSessionDeleteBridge) {
        const settingsLoaded = await loadBackendSettingsState();
        providerRefreshFailed = !settingsLoaded;
        if (providerRefreshFailed) {
          sendCodexPlusDiagnostic("remote_session_provider_refresh_failed", {});
        }
      } else if (codexRemoteSessionProviderRequestMethod(requestMethod)
          && codexRemoteSessionProviderOverrideEnabled()
          && !codexRemoteSessionTargetProvider()) {
        await loadCodexModelCatalog();
      }
      const providerParams = providerRefreshFailed
        ? params
        : applyCodexRemoteSessionProviderOverride(requestMethod, params);
      const nextParams = applyCodexServiceTierRequestOnly(requestMethod, providerParams);
      const modelContextRefresh = await refreshCodexThreadModelBeforeTurn(
        client,
        originalSendRequest.bind(client),
        method,
        nextParams,
        options
      );
      const result = await originalSendRequest.call(client, method, nextParams, options);
      const threadState = codexThreadModelRequestState(requestMethod, nextParams, result);
      if (modelContextRefresh !== false && threadState.threadId && threadState.model
          && ["thread/start", "thread/resume", "turn/start"].includes(threadState.requestMethod)) {
        client.__codexPlusThreadModels.set(threadState.threadId, threadState.model);
      }
      if (!codexPlusModelUnlockEnabled()) return result;
      if (!codexPlusModelNames().length) await loadCodexModelCatalog();
      return patchAppServerModelResult(requestMethod, result);
    };
    if (typeof proto.prewarmThreadStart === "function"
        && !proto.__codexPlusServiceTierOriginalPrewarmThreadStart) {
      const originalPrewarmThreadStart = proto.prewarmThreadStart;
      proto.__codexPlusServiceTierOriginalPrewarmThreadStart = originalPrewarmThreadStart;
      proto.prewarmThreadStart = async function codexPlusServiceTierPrewarmThreadStart(params, options) {
        const nextParams = applyCodexServiceTierRequestOnly("thread/start", params);
        return originalPrewarmThreadStart.call(this, nextParams, options);
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
        || (codexPlusBackendSettingsLoaded && codexRemoteSessionProviderPatchEnabled())
        || codexPlusSettings().serviceTierControls) {
      installAppServerModelRequestPatch();
      void installCodexAppServerClientCapture().catch(() => {});
    }
    void installDictationSupportPatch();
    if (!codexPlusModelUnlockEnabled()) return;
    installModelJsonResponsePatch();
    patchAppServerModelMessages();
  }

  function runCodexModelWhitelistRefreshPass() {
    if (!codexPlusModelUnlockEnabled() || !codexPlusModelNames().length) return false;
    try {
      patchStatsigModelWhitelist();
      installAppServerModelRequestPatch();
    } catch (error) {
      window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
      window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
    }
    return false;
  }

  function scheduleCodexModelWhitelistRefresh(durationMs = 2500) {
    if (!codexPlusModelUnlockEnabled()) return;
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

