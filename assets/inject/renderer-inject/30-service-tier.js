  // 服务等级由 Codex 原生控件决定；此兼容入口仅负责供应商适配。
  function applyCodexServiceTierRequestOverride(method, params, threadIdHint = "") {
    return applyCodexRemoteSessionProviderOverride(method, params);
  }

  function codexRemoteSessionActiveProfile() {
    if (!codexPlusBackendSettings.relayProfilesEnabled) return null;
    const profiles = Array.isArray(codexPlusBackendSettings.relayProfiles)
      ? codexPlusBackendSettings.relayProfiles
      : [];
    const activeId = String(codexPlusBackendSettings.activeRelayId || "");
    return profiles.find((item) => String(item?.id || "") === activeId) || null;
  }

  function codexRemoteSessionProviderPatchEnabled() {
    const profile = codexRemoteSessionActiveProfile();
    if (!profile) return false;
    const relayMode = String(profile.relayMode || "");
    return relayMode === "pureApi" || relayMode === "aggregate"
      || (relayMode === "official" && !!profile.officialMixApiKey);
  }

  function codexRemoteSessionProviderNormalizationEnabled() {
    if (!codexRemoteSessionProviderPatchEnabled()) return false;
    const profile = codexRemoteSessionActiveProfile();
    if (String(profile?.relayMode || "") !== "official") return false;
    const sessionProvider = String(
      codexPlusBackendSettings.activeRelaySessionProvider || "custom"
    ).trim().toLowerCase();
    return sessionProvider !== "openai";
  }

  function codexRemoteSessionProviderOverrideEnabled() {
    const profile = codexRemoteSessionActiveProfile();
    if (!profile) return false;
    const relayMode = String(profile.relayMode || "");
    if (relayMode === "pureApi") return true;
    return codexRemoteSessionProviderNormalizationEnabled();
  }

  function codexRelayConfigModelProvider(configContents) {
    const text = String(configContents || "");
    const match = /(?:^|\n)\s*model_provider\s*=\s*["']([^"'\n]+)["']/m.exec(text);
    return match ? String(match[1]).trim() : "";
  }

  function codexRemoteSessionTargetProvider() {
    const profile = codexRemoteSessionActiveProfile();
    const relayMode = String(profile?.relayMode || "");
    // 解析中继实际写进 config.toml 的 model_provider（比如
    // `model_provider = "deepseek"` 配 [model_providers.deepseek]），而不是
    // 假定每个 pureApi 中继都叫 "custom"——那会让恢复会话报
    // "Model provider `custom` not found"。
    //
    // 顺序上先看 profile.configContents 再看 activeRelayCodexProvider：后者是
    // 全局缓存，切换供应商后可能还是上一个的值；profile 是当前这次调用现取的，
    // 更可信。反过来会让 pureApi 恢复会话拿到陈旧 provider。
    const fromConfig = codexRelayConfigModelProvider(profile?.configContents || "");
    if (fromConfig) return fromConfig;
    // pureApi 且 profile 自己没声明供应方时回到 "custom"，不去读可能陈旧的全局缓存。
    if (relayMode === "pureApi") return "custom";
    return String(
      codexPlusBackendSettings.activeRelayCodexProvider
      || codexModelCatalog?.codex_model_provider
      || codexModelCatalog?.codexModelProvider
      || codexModelCatalog?.model_provider
      || codexModelCatalog?.modelProvider
      || (String(profile?.relayMode || "") === "pureApi" ? "custom" : "")
      || ""
    ).trim();
  }

  function codexRemoteSessionProviderRequestMethod(method) {
    return [
      "thread/start",
      "thread/resume",
      "thread/fork",
      "start-conversation",
      "start-thread-for-host",
      "thread-prewarm-start",
      "prewarm-thread-start-for-host",
      "turn/start",
    ].includes(String(method || ""));
  }

  function applyCodexRemoteSessionProviderOverride(method, params) {
    const requestMethod = String(method || "");
    if (!codexRemoteSessionProviderRequestMethod(requestMethod)) return params;
    if (!codexRemoteSessionProviderOverrideEnabled()) return params;
    if (!params || typeof params !== "object" || Array.isArray(params)) return params;
    const profile = codexRemoteSessionActiveProfile();
    const pureApi = String(profile?.relayMode || "") === "pureApi";
    // Pure API 恢复旧会话时必须覆盖持久化的 openai；官登会话保留原生恢复语义。
    if (requestMethod === "thread/resume" && !pureApi) return params;
    if (requestMethod === "turn/start" && !pureApi) return params;
    const hasModelProvider = Object.prototype.hasOwnProperty.call(params, "modelProvider")
      || Object.prototype.hasOwnProperty.call(params, "model_provider");
    if (requestMethod === "turn/start" && !hasModelProvider) return params;
    const targetProvider = codexRemoteSessionTargetProvider();
    if (!targetProvider || targetProvider === "openai") return params;
    const requestedProvider = String(params.modelProvider || params.model_provider || "").trim();
    if (requestedProvider && requestedProvider !== "openai" && requestedProvider !== targetProvider) {
      return params;
    }
    if (requestedProvider === targetProvider && !Object.prototype.hasOwnProperty.call(params, "model_provider")) {
      return params;
    }
    const nextParams = { ...params, modelProvider: targetProvider };
    delete nextParams.model_provider;
    sendCodexPlusDiagnostic("remote_session_provider_override_applied", {
      method: requestMethod,
      from: requestedProvider || "(missing)",
      to: targetProvider,
    });
    return nextParams;
  }

  function codexManagedSessionConnection() {
    const profile = codexRemoteSessionActiveProfile();
    const connection = codexPlusBackendSettings.managedSessionConnection;
    if (!profile || profile.relayMode !== "aggregate" || !connection
        || connection.profileId !== profile.id || connection.provider !== "codex_plus_managed"
        || !/^http:\/\/127\.0\.0\.1:[1-9]\d{0,4}\/v1$/.test(connection.baseUrl || "")) return null;
    return connection;
  }

  function applyCodexManagedSessionConnection(method, params, hostId) {
    if (hostId !== "local" || !["thread/start", "thread/resume", "thread/fork"].includes(method)
        || !params || typeof params !== "object" || Array.isArray(params)) return params;
    // 原生继承分支禁止同时传 config；其父会话已经持有连接快照。
    if (method === "thread/fork" && params.experimentalPredictionMode === true) return params;
    const connection = codexManagedSessionConnection();
    if (!connection) return params;
    const config = { ...(params.config || {}) };
    if (config.model_providers && typeof config.model_providers === "object") {
      config.model_providers = { ...config.model_providers };
      delete config.model_providers.codex_plus_managed;
    }
    // 清掉请求携带的同名传输覆盖，避免扁平字段再次覆盖完整 provider 表。
    for (const key of Object.keys(config)) {
      if (key === "model_provider" || key === "model_providers.codex_plus_managed"
          || key.startsWith("model_providers.codex_plus_managed.")) {
        delete config[key];
      }
    }
    config.model_provider = connection.provider;
    config["model_providers.codex_plus_managed"] = {
      name: "custom",
      base_url: connection.baseUrl,
      wire_api: "responses",
      experimental_bearer_token: "codex-plus-aggregate",
      requires_openai_auth: false,
      supports_websockets: false,
    };
    const next = { ...params, modelProvider: connection.provider, config };
    delete next.model_provider;
    return next;
  }

  async function prepareCodexManagedSessionConnection(method, params, hostId, sendRequest) {
    if (method === "send-cli-request-for-host" && hostId === "local" && params?.method) {
      if ([params.hostId, params.host_id].some((host) => typeof host === "string" && host !== "local")) {
        return params;
      }
      const nested = await prepareCodexManagedSessionConnection(
        String(params.method), params.params, hostId,
        (nestedMethod, nestedParams, options) => sendRequest(method,
          { ...params, method: nestedMethod, params: nestedParams }, options));
      return nested === params.params ? params : { ...params, params: nested };
    }
    const patched = applyCodexManagedSessionConnection(method, params, hostId);
    if (patched !== params || hostId !== "local" || method !== "thread/resume" || !params?.threadId) {
      return patched;
    }
    // 退出聚合保护后，旧会话不能继续依赖只存在于会话快照里的内部 provider。
    let provider = params.modelProvider || params.model_provider || "";
    if (!provider) {
      try {
        const result = await readCodexManagedSessionForCheck(params.threadId, sendRequest);
        provider = result?.thread?.modelProvider || "";
      } catch {
        // 按路径恢复等原生情形允许 read 尚不可用，保留原始 resume 语义。
        return params;
      }
    }
    if (provider !== "codex_plus_managed") return params;
    const target = codexRemoteSessionTargetProvider();
    if (!target || target === "codex_plus_managed") throw new Error("请先选择可用的供应商配置");
    const config = { ...(params.config || {}) };
    delete config.model_provider;
    delete config["model_providers.codex_plus_managed"];
    const next = { ...params, config, modelProvider: target };
    delete next.model_provider;
    return next;
  }

  function noteCodexManagedSessionStart(method, params, result, hostId) {
    if (hostId !== "local") return;
    if (method === "send-cli-request-for-host" && params?.method) {
      if ([params.hostId, params.host_id].some((host) => typeof host === "string" && host !== "local")) return;
      return noteCodexManagedSessionStart(String(params.method), params.params, result, hostId);
    }
    if (!["thread/start", "thread/fork"].includes(method)) return;
    const connection = codexManagedSessionConnection();
    const provider = params?.config?.["model_providers.codex_plus_managed"];
    const inherited = method === "thread/fork" && params.experimentalPredictionMode === true
      && window.__codexPlusManagedSessionStarts?.get(params.threadId);
    const resultProvider = result?.modelProvider || result?.thread?.modelProvider || "";
    if (!connection || (provider?.base_url !== connection.baseUrl
        && inherited?.signature !== `${connection.profileId}|${connection.baseUrl}`)
        || (resultProvider && resultProvider !== connection.provider) || !result?.thread?.id) return;
    const starts = window.__codexPlusManagedSessionStarts ||= new Map();
    starts.set(result.thread.id, {
      signature: `${connection.profileId}|${connection.baseUrl}`,
      model: params.model || result.model || "",
      needsRecovery: false,
    });
    if (starts.size > 256) starts.delete(starts.keys().next().value);
  }

  async function prewarmCodexManagedSession(client, prewarm, params, options) {
    // 预创建直接走原生队列，不经过 sendRequest；必须在这个入口同样固定连接。
    if (client.hostId === "local" && window.__codexSessionDeleteBridge
        && codexRemoteSessionActiveProfile()?.relayMode === "aggregate") {
      await loadBackendSettingsState(codexAppServerPreparationTimeoutMs);
    }
    const nextParams = applyCodexManagedSessionConnection("thread/start",
      params, client.hostId);
    const result = await prewarm(nextParams, options);
    noteCodexManagedSessionStart("thread/start", nextParams, result, client.hostId);
    return result;
  }

  function observeCodexManagedSessionConnectionError(value) {
    if (!codexManagedSessionConnection()) return false;
    function observe(candidate, depth) {
      if (!candidate || typeof candidate !== "object" || depth > 4) return false;
      if ([candidate.hostId, candidate.host_id].some((host) => typeof host === "string" && host !== "local")) return false;
      const method = candidate.method || candidate.type;
      const params = candidate.params || candidate;
      const error = method === "error" ? params.error
        : method === "turn/completed" ? params.turn?.error : null;
      const threadId = params.threadId;
      if (error && typeof threadId === "string" && threadId.length > 0 && threadId.length <= 128
          && codexManagedSessionConnectionError(error)) {
        const starts = window.__codexPlusManagedSessionStarts ||= new Map();
        const known = starts.get(threadId) || {};
        starts.set(threadId, { ...known, needsRecovery: true });
        if (starts.size > 256) starts.delete(starts.keys().next().value);
        return true;
      }
      if (method === "turn/completed" && params.turn?.status === "completed") {
        const starts = window.__codexPlusManagedSessionStarts;
        const known = starts?.get(threadId);
        if (known?.needsRecovery) starts.set(threadId, { ...known, needsRecovery: false });
      }
      // 原生逐 token 通知直接退出，不递归扫描输出内容。
      if (method === "error" || method === "turn/completed" || candidate.type === "mcp-notification") return false;
      for (const key of ["message", "response", "detail", "data", "payload", "notification"]) {
        if (observe(candidate[key], depth + 1)) {
          return true;
        }
      }
      return false;
    }
    return observe(value, 0);
  }

  const codexManagedSessionReadTimeoutMs = 800;
  const codexManagedSessionRecoveryTimeoutMs = 5000;

  function codexManagedSessionRecoveryOptions(options, timeoutMs = codexManagedSessionRecoveryTimeoutMs) {
    // 不继承消息提交的 onOutcomeUnknown、消息 ID 和 trace；它们属于实际 turn/start。
    return { priority: options?.priority, timeoutMs };
  }

  async function readCodexManagedSessionForCheck(threadId, sendRequest, options) {
    let timer;
    try {
      return await Promise.race([
        sendRequest("thread/read", { threadId, includeTurns: false },
          codexManagedSessionRecoveryOptions(options, codexManagedSessionReadTimeoutMs)),
        new Promise((resolve) => {
          timer = setTimeout(() => resolve(null), codexManagedSessionReadTimeoutMs);
        }),
      ]);
    } catch {
      return null;
    } finally {
      if (timer) clearTimeout(timer);
    }
  }

  function codexManagedSessionConnectionError(error) {
    const info = error?.codexErrorInfo;
    if (info && typeof info === "object"
        && ["responseStreamDisconnected", "responseStreamConnectionFailed", "httpConnectionFailed"]
          .some((key) => Object.prototype.hasOwnProperty.call(info, key))) return true;
    const message = [error?.message || (typeof error === "string" ? error : ""),
      error?.additionalDetails || ""].join(" ").toLowerCase();
    return /reconnecting|connection (?:failed|closed|reset|refused)|network|proxy|stream disconnected|stream closed|econn|broken pipe|failed to send|error sending request|transport/.test(message);
  }

  function assertCodexManagedSessionConnection(signature) {
    const connection = codexManagedSessionConnection();
    if (!connection || `${connection.profileId}|${connection.baseUrl}` !== signature) {
      throw new Error("连接配置已变化，请重新发送消息");
    }
  }

  async function reloadCodexManagedSessionConnection(threadId, read, turnParams, sendRequest, options, signature) {
    const recoveryOptions = codexManagedSessionRecoveryOptions(options);
    // 已加载 resume 不改配置，可用于读取运行时设置；不拉取整段聊天历史。
    const current = read.thread.status.type === "idle"
      ? await sendRequest("thread/resume", { threadId, excludeTurns: true }, recoveryOptions) : null;
    if (current?.thread?.status && current.thread.status.type !== "idle") {
      throw new Error("请先停止当前回复，再恢复连接");
    }
    assertCodexManagedSessionConnection(signature);
    const params = { threadId, excludeTurns: true, config: {} };
    for (const key of ["cwd", "approvalPolicy", "approvalsReviewer", "runtimeWorkspaceRoots", "serviceTier"]) {
      if (current?.[key] !== undefined) params[key] = current[key];
    }
    const model = turnParams?.model || current?.model;
    if (model) params.model = model;
    const effort = turnParams?.effort ?? current?.reasoningEffort;
    if (effort) params.config.model_reasoning_effort = effort;
    if (Object.prototype.hasOwnProperty.call(turnParams || {}, "serviceTier")) {
      params.serviceTier = turnParams.serviceTier;
    }
    if (typeof current?.activePermissionProfile?.id === "string") {
      params.permissions = current.activePermissionProfile.id;
    } else if (current?.sandbox) {
      const modes = { readOnly: "read-only", workspaceWrite: "workspace-write", dangerFullAccess: "danger-full-access" };
      const mode = modes[current.sandbox.type];
      if (!mode) throw new Error("当前权限配置不支持安全恢复连接");
      params.sandbox = mode;
      if (mode === "workspace-write") {
        const sandbox = current.sandbox;
        params.config.sandbox_workspace_write = {
          writable_roots: sandbox.writableRoots || [],
          network_access: sandbox.networkAccess === true,
          exclude_tmpdir_env_var: sandbox.excludeTmpdirEnvVar === true,
          exclude_slash_tmp: sandbox.excludeSlashTmp === true,
        };
      }
    }
    const protectedParams = applyCodexManagedSessionConnection("thread/resume", params, "local");
    // 已加载 resume 忽略连接覆盖，必须先卸载空闲会话再恢复。
    await sendRequest("thread/unsubscribe", { threadId }, recoveryOptions);
    assertCodexManagedSessionConnection(signature);
    const result = await sendRequest("thread/resume", protectedParams, recoveryOptions);
    if (result?.modelProvider !== "codex_plus_managed") throw new Error("会话未采用受管连接配置");
    return result;
  }

  async function sendCodexManagedSessionTurn(client, sendRequest, method, params, options, modelContextRefresh) {
    if (client.hostId !== "local") return null;
    if (method === "send-cli-request-for-host" && params?.method) {
      if ([params.hostId, params.host_id].some((host) => typeof host === "string" && host !== "local")) {
        return null;
      }
      return await sendCodexManagedSessionTurn(client,
        (nestedMethod, nestedParams, nestedOptions) => sendRequest(method,
          { ...params, method: nestedMethod, params: nestedParams }, nestedOptions),
        String(params.method), params.params, options, modelContextRefresh);
    }
    if (method !== "turn/start" || !codexManagedSessionConnection() || !params?.threadId) return null;
    const connection = codexManagedSessionConnection();
    const starts = window.__codexPlusManagedSessionStarts ||= new Map();
    const signature = `${connection?.profileId}|${connection?.baseUrl}`;
    const known = starts.get(params.threadId);
    const pending = window.__codexPlusConnectionRecoveryPending;
    if (pending?.has(params.threadId)) throw new Error("此会话正在检查连接，请稍后重新发送");
    const rememberSuccessfulTurn = () => {
      const current = starts.get(params.threadId) || {};
      starts.set(params.threadId, {
        ...current,
        signature,
        model: params.model || current.model || "",
        needsRecovery: current !== known && current.needsRecovery === true,
      });
      if (starts.size > 256) starts.delete(starts.keys().next().value);
    };
    // 已由 Codex++ 创建/恢复的会话直接发送。正常消息不再额外读取配置或线程状态。
    if (known?.signature === signature && !known.needsRecovery) {
      try {
        const response = await sendRequest(method, params, options);
        rememberSuccessfulTurn();
        if (modelContextRefresh !== false && params.model) client.__codexPlusThreadModels?.set(params.threadId, params.model);
        return { result: response };
      } catch (error) {
        // 只把明确的连接类错误标成待检查；当前消息不自动重发。
        if (codexManagedSessionConnectionError(error)) {
          starts.set(params.threadId, { ...known, needsRecovery: true });
        }
        throw error;
      }
    }

    // 首条消息或尚未见过的旧会话直接交给原生发送，避免在没有 rollout
    // 的新线程上调用 thread/resume。连接错误只标记下一次发送恢复。
    if (!known) {
      try {
        const response = await sendRequest(method, params, options);
        // turn/start 的确认只代表任务入队，不能证明后续流连接使用了受管供应商。
        if (modelContextRefresh !== false && params.model) client.__codexPlusThreadModels?.set(params.threadId, params.model);
        return { result: response };
      } catch (error) {
        if (codexManagedSessionConnectionError(error)) {
          starts.set(params.threadId, {
            signature,
            model: params.model || "",
            needsRecovery: true,
          });
        }
        throw error;
      }
    }

    const recoveries = window.__codexPlusConnectionRecoveryPending ||= new Set();
    recoveries.add(params.threadId);
    try {
      // 仅连接签名变化或上一次明确连接失败时检查，且读请求超时后交回原生发送。
      const read = await readCodexManagedSessionForCheck(params.threadId, sendRequest, options);
      assertCodexManagedSessionConnection(signature);
      if (!read?.thread) return null;
      const status = read.thread.status?.type;
      if (!["idle", "notLoaded"].includes(status)) return null;
      // 新建会话在首条 turn 前没有 rollout，不能调用 thread/resume。
      if (read.thread.path && !(status === "idle" && known.signature === signature
          && read.thread.modelProvider === connection.provider)) {
        await reloadCodexManagedSessionConnection(params.threadId, read, params, sendRequest, options, signature);
      }
      assertCodexManagedSessionConnection(signature);
      // 已完成连接准备后就释放锁，消息确认变慢不能占住恢复入口。
    } finally {
      recoveries.delete(params.threadId);
    }
    try {
      const response = await sendRequest(method, params, options);
      rememberSuccessfulTurn();
      if (modelContextRefresh !== false && params.model) client.__codexPlusThreadModels?.set(params.threadId, params.model);
      return { result: response };
    } catch (error) {
      if (codexManagedSessionConnectionError(error)) {
        starts.set(params.threadId, { ...(starts.get(params.threadId) || known), needsRecovery: true });
      }
      throw error;
    }
  }

  function codexRemoteSessionStartedThreadId(value) {
    const queue = [{ value, depth: 0 }];
    const seen = new WeakSet();
    while (queue.length > 0) {
      const current = queue.shift();
      const candidate = current?.value;
      if (!candidate || typeof candidate !== "object") continue;
      if (seen.has(candidate)) continue;
      seen.add(candidate);
      const method = String(candidate.method || candidate.type || "");
      if (method === "thread/started") {
        const thread = candidate.params?.thread || candidate.thread || candidate.payload?.thread;
        const threadId = String(thread?.id || candidate.params?.threadId || candidate.threadId || "").trim();
        if (threadId) return threadId;
      }
      if (method === "browser-use-session-route-capture") {
        const threadId = String(
          candidate.params?.conversationId
          || candidate.params?.conversation_id
          || candidate.conversationId
          || candidate.conversation_id
          || ""
        ).trim();
        if (threadId) return threadId;
      }
      if (method === "browser-sidebar-browser-use-state") {
        const isActive = candidate.params?.isActive ?? candidate.params?.is_active
          ?? candidate.isActive ?? candidate.is_active;
        if (isActive !== true) continue;
        const threadId = String(
          candidate.params?.conversationId
          || candidate.params?.conversation_id
          || candidate.conversationId
          || candidate.conversation_id
          || ""
        ).trim();
        if (threadId) return threadId;
      }
      if (current.depth >= 4) continue;
      for (const key of ["message", "response", "detail", "data", "payload", "params", "request"]) {
        const nested = candidate[key];
        if (nested && typeof nested === "object") {
          queue.push({ value: nested, depth: current.depth + 1 });
        }
      }
    }
    return "";
  }

  function requestCodexRemoteSessionRecovery(threadId, attempt) {
    const payload = { thread_id: threadId };
    const testHook = window.__CODEX_PLUS_TEST_REMOTE_RECOVERY__;
    const request = typeof testHook === "function"
      ? Promise.resolve(testHook(payload, attempt))
      : postJson("/remote-control-session/recover", payload);
    return request.then((result) => {
      if (attempt === 0
        || result?.message === "Remote Control session recovery complete"
        || result?.message === "Remote Control session catalog recovery complete") {
        sendCodexPlusDiagnostic("remote_session_recovery_requested", {
          threadId,
          attempt,
          status: result?.status || "",
          message: result?.message || "",
          changedSessionFiles: result?.changed_session_files || 0,
          catalogRowsInserted: result?.sqlite_catalog_rows_inserted || 0,
        });
      }
      return result;
    }).catch((error) => {
      if (attempt === 0) {
        sendCodexPlusDiagnostic("remote_session_recovery_failed", {
          threadId,
          attempt,
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
      }
      return null;
    });
  }

  function scheduleCodexRemoteSessionRecovery(threadId) {
    if (!codexRemoteSessionProviderNormalizationEnabled()) return false;
    const normalizedThreadId = String(threadId || "").trim();
    if (!normalizedThreadId || normalizedThreadId.length > 128) return false;
    window.__codexPlusRemoteSessionRecoveryPending = window.__codexPlusRemoteSessionRecoveryPending || new Map();
    const pending = window.__codexPlusRemoteSessionRecoveryPending;
    if (pending.has(normalizedThreadId)) return false;
    const retryOffsets = [100, 350, 800, 1600, 3000];
    const state = { timer: 0 };
    const finish = () => {
      if (state.timer) window.clearTimeout(state.timer);
      state.timer = 0;
      if (pending.get(normalizedThreadId) === state) pending.delete(normalizedThreadId);
    };
    const runAttempt = async (attempt) => {
      state.timer = 0;
      if (!codexRemoteSessionProviderNormalizationEnabled()) {
        finish();
        return;
      }
      const result = await requestCodexRemoteSessionRecovery(normalizedThreadId, attempt);
      const message = String(result?.message || "");
      if (message === "Remote Control session recovery complete"
        || message === "Remote Control session catalog recovery complete"
        || message === "Remote Control session recovery is disabled for the active profile") {
        finish();
        return;
      }
      const nextAttempt = attempt + 1;
      if (nextAttempt >= retryOffsets.length) {
        finish();
        return;
      }
      const nextDelay = retryOffsets[nextAttempt] - retryOffsets[attempt];
      state.timer = window.setTimeout(() => void runAttempt(nextAttempt), nextDelay);
    };
    state.timer = window.setTimeout(() => void runAttempt(0), retryOffsets[0]);
    pending.set(normalizedThreadId, state);
    return true;
  }

  function observeCodexRemoteSessionNotification(value) {
    observeCodexManagedSessionConnectionError(value);
    const threadId = codexRemoteSessionStartedThreadId(value);
    return threadId ? scheduleCodexRemoteSessionRecovery(threadId) : false;
  }

  function installCodexRemoteSessionRecoveryListener() {
    if (window.__codexPlusRemoteSessionRecoveryInstalled === codexRemoteSessionRecoveryVersion) return true;
    if (window.__codexPlusRemoteSessionRecoveryMessageHandler) {
      window.removeEventListener("message", window.__codexPlusRemoteSessionRecoveryMessageHandler, true);
    }
    if (window.__codexPlusRemoteSessionRecoveryViewHandler) {
      window.removeEventListener("codex-message-from-view", window.__codexPlusRemoteSessionRecoveryViewHandler, true);
    }
    const messageHandler = (event) => {
      const origin = String(event?.origin || "");
      // 桌面原生桥通过 MessageEvent 投递，source=null、origin=""。
      const nativeLocalNotification = event?.source == null && !origin
        && event?.data?.type === "mcp-notification" && event.data.hostId === "local";
      if (!nativeLocalNotification) {
        if (event?.source !== window) return false;
        if (origin && origin !== "null" && origin !== window.location.origin) return false;
      }
      return observeCodexRemoteSessionNotification(event?.data);
    };
    const viewHandler = (event) => observeCodexRemoteSessionNotification(event?.detail);
    window.__codexPlusRemoteSessionRecoveryMessageHandler = messageHandler;
    window.__codexPlusRemoteSessionRecoveryViewHandler = viewHandler;
    window.addEventListener("message", messageHandler, true);
    window.addEventListener("codex-message-from-view", viewHandler, true);
    window.__codexPlusRemoteSessionRecoveryInstalled = codexRemoteSessionRecoveryVersion;
    sendCodexPlusDiagnostic("remote_session_recovery_listener_installed", {
      version: codexRemoteSessionRecoveryVersion,
    });
    return true;
  }

  function installCodexRemoteSessionDispatcherSubscription(dispatcher, assetPrefix = "") {
    if (!dispatcher || typeof dispatcher.subscribe !== "function") return false;
    if (window.__codexPlusRemoteSessionRecoveryDispatcher === dispatcher
        && window.__codexPlusRemoteSessionRecoveryDispatcherVersion === codexRemoteSessionRecoveryVersion) {
      return true;
    }
    if (typeof window.__codexPlusRemoteSessionRecoveryDispatcherUnsubscribe === "function") {
      try {
        window.__codexPlusRemoteSessionRecoveryDispatcherUnsubscribe();
      } catch {
      }
    }
    const handler = (payload) => {
      if (observeCodexRemoteSessionNotification(payload)) return true;
      const params = payload && typeof payload === "object" ? payload : {};
      if (observeCodexRemoteSessionNotification({
        method: "thread/started",
        params,
      })) return true;
      return observeCodexRemoteSessionNotification({
        method: "thread/started",
        params: { thread: params },
      });
    };
    const browserUseHandler = (payload) => observeCodexRemoteSessionNotification({
      type: "browser-sidebar-browser-use-state",
      params: payload && typeof payload === "object" ? payload : {},
    });
    const unsubscribers = [
      dispatcher.subscribe("thread/started", handler),
      dispatcher.subscribe("error", (params) => observeCodexManagedSessionConnectionError({ method: "error", params })),
      dispatcher.subscribe("turn/completed", (params) => observeCodexManagedSessionConnectionError({ method: "turn/completed", params })),
      dispatcher.subscribe("browser-sidebar-browser-use-state", browserUseHandler),
    ];
    window.__codexPlusRemoteSessionRecoveryDispatcher = dispatcher;
    window.__codexPlusRemoteSessionRecoveryDispatcherHandler = handler;
    window.__codexPlusRemoteSessionRecoveryDispatcherUnsubscribe = () => {
      for (const unsubscribe of unsubscribers) {
        if (typeof unsubscribe !== "function") continue;
        try {
          unsubscribe();
        } catch {
        }
      }
    };
    window.__codexPlusRemoteSessionRecoveryDispatcherVersion = codexRemoteSessionRecoveryVersion;
    sendCodexPlusDiagnostic("remote_session_dispatcher_subscription_installed", { assetPrefix });
    return true;
  }

  function codexServiceTierRequestOverride(message, skipFetchEnvelope = false) {
    if (!message || typeof message !== "object") return message;
    if (!skipFetchEnvelope && message.type === "fetch" && typeof message.url === "string") {
      const urlPrefix = "vscode://codex/";
      if (!message.url.startsWith(urlPrefix)) return message;
      const requestType = message.url.slice(urlPrefix.length).split(/[?#]/, 1)[0];
      let params = null;
      let bodyWasString = false;
      if (typeof message.body === "string") {
        try {
          params = JSON.parse(message.body);
          bodyWasString = true;
        } catch (_) {
          return message;
        }
      } else if (message.body && typeof message.body === "object") {
        params = message.body;
      } else {
        return message;
      }
      if (!params || typeof params !== "object" || Array.isArray(params)) return message;
      const bodyHadType = Object.prototype.hasOwnProperty.call(params, "type");
      const originalBodyType = params.type;
      const logicalMessage = { ...params, type: requestType };
      const patchedMessage = codexServiceTierRequestOverride(logicalMessage, true);
      if (patchedMessage === logicalMessage) return message;
      const nextParams = { ...patchedMessage };
      delete nextParams.type;
      if (bodyHadType) nextParams.type = originalBodyType;
      return {
        ...message,
        body: bodyWasString ? JSON.stringify(nextParams) : nextParams,
      };
    }
    if (message.type === "send-cli-request-for-host") {
      const method = String(message.method || "");
      const params = applyCodexServiceTierRequestOverride(method, message.params);
      return params === message.params ? message : { ...message, params };
    }
    if (message.type === "mcp-request" && message.request && typeof message.request === "object") {
      const method = String(message.request.method || "");
      const params = applyCodexServiceTierRequestOverride(method, message.request.params);
      if (params === message.request.params) return message;
      return { ...message, request: { ...message.request, params } };
    }
    if (message.type === "worker-request" && message.request && typeof message.request === "object") {
      const method = String(message.request.method || "");
      const params = applyCodexServiceTierRequestOverride(method, message.request.params);
      if (params === message.request.params) return message;
      return { ...message, request: { ...message.request, params } };
    }
    if (message.type === "thread-prewarm-start" && message.request && typeof message.request === "object") {
      const params = applyCodexServiceTierRequestOverride("thread/start", message.request.params);
      if (params === message.request.params) return message;
      return { ...message, request: { ...message.request, params } };
    }
    if (message.type === "start-conversation") {
      const nextMessage = applyCodexServiceTierRequestOverride("thread/start", message);
      return nextMessage === message ? message : nextMessage;
    }
    if (message.type === "prewarm-thread-start-for-host" && message.params && typeof message.params === "object") {
      const params = applyCodexServiceTierRequestOverride("thread/start", message.params);
      return params === message.params ? message : { ...message, params };
    }
    if (message.type === "start-thread-for-host") {
      const params = applyCodexServiceTierRequestOverride("thread/start", message);
      return params === message ? message : params;
    }
    if (message.type === "start-turn-for-host" && message.params && typeof message.params === "object") {
      const params = applyCodexServiceTierRequestOverride("turn/start", message.params, message.conversationId);
      return params === message.params ? message : { ...message, params };
    }
    return message;
  }

  function codexServiceTierDispatcherFromModule(module) {
    const directSingleton = module?.idt;
    if (directSingleton
        && typeof directSingleton === "object"
        && typeof directSingleton.dispatchMessage === "function"
        && typeof directSingleton.subscribe === "function") {
      return directSingleton;
    }
    const values = module && typeof module === "object" ? Object.values(module) : [];
    const singleton = values.find((candidate) => candidate
      && typeof candidate === "object"
      && typeof candidate.dispatchMessage === "function"
      && typeof candidate.subscribe === "function");
    if (singleton) return singleton;
    const dispatcherClass = values.find((candidate) => typeof candidate === "function"
      && typeof candidate.getInstance === "function"
      && typeof candidate.prototype?.dispatchMessage === "function");
    return dispatcherClass?.getInstance?.() || null;
  }

  const serviceTierDispatcherPatchMaxMisses = 8;
  let serviceTierDispatcherPatchMissCount = 0;
  let serviceTierDispatcherPatchDisabled = false;
  let serviceTierDispatcherPatchPromise = null;

  function codexServiceTierDispatcherPatchable(dispatcher) {
    if (!dispatcher || typeof dispatcher !== "object") return false;
    try {
      if (!Object.isExtensible(dispatcher)) return false;
      for (const key of ["__codexServiceTierOriginalDispatchMessage", "dispatchMessage"]) {
        const descriptor = Object.getOwnPropertyDescriptor(dispatcher, key);
        if (descriptor && descriptor.writable === false && typeof descriptor.set !== "function") return false;
      }
      return true;
    } catch {
      // Codex 26.908 RPC stubs can throw even while being inspected.
      return false;
    }
  }

  // issue #1960：这是 installAppServerModelRequestPatch（#1324）和插件市场那两层的同一个缺陷。
  // 补丁挂在 scanLightweight() 里每轮都跑，而早退守卫只在装上之后才写入，
  // Codex 侧 asset 改名后就永远装不上，于是每轮 scan 重新拉一遍全部 app asset，
  // 而且每轮都发一条相同的诊断。首次失败仍上报以便定位，之后噤声，连续失败够多次就停掉这一层。
  function installCodexServiceTierDispatcherPatch() {
    if (window.__codexServiceTierRequestOverrideInstalled === codexServiceTierRequestOverrideVersion) return;
    if (serviceTierDispatcherPatchDisabled) return;
    // 上一轮没跑完就不要再起一轮：loadDispatcher() 会依次试四个前缀，
    // 没有这道去重时 scan 的频率就直接变成并发全量扫描的频率。
    if (serviceTierDispatcherPatchPromise) return;
    const loadDispatcher = async () => {
      const errors = [];
      // 26.1016 起 dispatcher 类定义并导出在 app-shared-*，app-initial-* 只导入不再导出，
      // setting-storage-* / vscode-api-* 也已不存在；缺这个前缀整层请求改写都装不上（#2657）。
      for (const assetPrefix of ["setting-storage-", "vscode-api-", "app-initial-", "app-shared-"]) {
        try {
          const module = await loadCodexAppModule(assetPrefix);
          const dispatcher = codexServiceTierDispatcherFromModule(module);
          if (dispatcher) return { dispatcher, assetPrefix };
          errors.push(`${assetPrefix}: dispatcher export unavailable`);
        } catch (error) {
          errors.push(`${assetPrefix}: ${error?.message || String(error)}`);
        }
      }
      throw new Error(`Codex dispatcher unavailable (${errors.join("; ")})`);
    };
    const patch = async () => {
      try {
        const { dispatcher, assetPrefix } = await loadDispatcher();
        if (!codexServiceTierDispatcherPatchable(dispatcher)) {
          throw new Error(`dispatcher is a non-writable RPC stub (${assetPrefix})`);
        }
        if (!dispatcher.__codexServiceTierOriginalDispatchMessage) {
          dispatcher.__codexServiceTierOriginalDispatchMessage = dispatcher.dispatchMessage.bind(dispatcher);
        }
        dispatcher.dispatchMessage = (type, payload) => {
          return dispatchCodexPlusMessage(dispatcher, type, payload);
        };
        installCodexRemoteSessionDispatcherSubscription(dispatcher, assetPrefix);
        window.__codexServiceTierRequestOverrideInstalled = codexServiceTierRequestOverrideVersion;
        serviceTierDispatcherPatchMissCount = 0;
        sendCodexPlusDiagnostic("service_tier_dispatcher_patch_installed", { assetPrefix });
      } catch (error) {
        serviceTierDispatcherPatchMissCount += 1;
        if (serviceTierDispatcherPatchMissCount === 1) {
          sendCodexPlusDiagnostic("service_tier_dispatcher_patch_failed", {
            errorName: error?.name || "",
            errorMessage: error?.message || String(error),
          });
        }
        if (serviceTierDispatcherPatchMissCount >= serviceTierDispatcherPatchMaxMisses
            && !serviceTierDispatcherPatchDisabled) {
          serviceTierDispatcherPatchDisabled = true;
          sendCodexPlusDiagnostic("service_tier_dispatcher_patch_skipped", {
            misses: serviceTierDispatcherPatchMissCount,
          });
        }
      } finally {
        serviceTierDispatcherPatchPromise = null;
      }
    };
    serviceTierDispatcherPatchPromise = patch();
  }

  // 保留旧调用入口，语音能力由独立的 opt-in 控件提供。
  function installDictationSupportPatch() {
    installCodexPlusDictation();
  }

  const codexAppServerPreparationTimeoutMs = 2000;

  // 发送前只读取辅助配置；桥接无回包时及时降级，迟到的结果也不再应用。
  // 写 RPC（尤其 thread/resume）不能这样竞速，否则会在 turn/start 后迟到修改会话。
  async function readCodexAppServerPreparation(path, payload, timeoutMs = codexAppServerPreparationTimeoutMs) {
    let timeoutId;
    try {
      return await Promise.race([
        postJson(path, payload),
        new Promise((_, reject) => {
          timeoutId = setTimeout(() => {
            sendCodexPlusDiagnostic("app_server_preparation_read_timeout", { path, timeoutMs });
            reject(new Error(`Codex++ preparation read timed out: ${path}`));
          }, timeoutMs);
        }),
      ]);
    } finally {
      clearTimeout(timeoutId);
    }
  }

  async function loadBackendSettingsState(timeoutMs = 0) {
    const seq = codexPlusBackendSettingsSeq;
    try {
      const settings = await (timeoutMs > 0
        ? readCodexAppServerPreparation("/settings/get", {}, timeoutMs)
        : postJson("/settings/get", {}));
      if (!settings || typeof settings !== "object" || (!("enhancementsEnabled" in settings) && !("providerSyncEnabled" in settings))) {
        throw new Error("invalid backend settings response");
      }
      if (seq !== codexPlusBackendSettingsSeq) {
        return false;
      }
      codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
      codexPlusBackendSettingsLoaded = true;
      globalThis.window?.__codexPlusNativeFastUnlock?.runtime?.update?.({
        enhancementsEnabled: codexPlusBackendSettings.enhancementsEnabled !== false,
        codexAppNativeFastUnlock: codexPlusBackendSettings.codexAppNativeFastUnlock === true,
        nativeFastLocalApiConfigured: codexPlusBackendSettings.nativeFastLocalApiConfigured === true,
      });
    } catch (_) {
      return false;
    }
    // 设置已加载成功；语音 UI 的异常不应使 provider/Fast 刷新误判为失败。
    // service-tier harness 在 70 分片提前返回，尚未初始化 93 分片的录音状态。
    if (!window.__CODEX_PLUS_TEST_SERVICE_TIER__) runScanStep(installCodexPlusDictation);
    return true;
  }
