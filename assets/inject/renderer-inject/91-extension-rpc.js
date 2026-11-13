  /**
   * 拓展的 RPC 层拦截点：`agent/pre-step`、`agent/request` 和 `codex/request`。
   *
   * 新版 Codex 把 app-server manager 搬进了宿主进程，页面经 MessagePort 用 capnweb 协议调它，
   * 页面里已经没有发 turn/start 的请求客户端（见 70-model-catalog 的类捕获，只剩旧路径）。
   * 页面能看到的只有 RPC 帧，所以在 MessagePort.prototype.postMessage 上按帧拦截：
   *
   *   - `["push", ["pipeline", id, ["sendRequest"], [method, params, opts]]]`：manager 的通用请求，
   *     走 `codex/request`。
   *   - `["push", ["pipeline", id, ["startConversation"], [params, callbacks]]]`：新会话的首轮，
   *     params 里带 input / collaborationMode，按 turn/start 处理。
   *   - `["resolve", exportId, { start: { request }, steer, resume, … }]`：宿主发起轮次前回调页面要参数，
   *     `start.request` 就是 turn/start 的原样参数，`steer` 是会话忙时改走 turn/steer 的候选。
   *
   * capnweb 的 import id 按 push 顺序隐式分配，所以改写 push 时后面的帧必须按原顺序发出。
   * 本地上下文刷新单独等待 resolve，允许它的预检请求通过；没有拓展或上下文刷新时同步放行。
   *
   * 语义（`ctx.on(name, async (params, next) => …)`，先 `await next()` 拿下游决定再改）：
   *   - `agent/pre-step`：params `{ agent, turn, step, messages, signal }`，决定是
   *     `{ kind: "enter", messages }` 或 `{ kind: "reject", reason? }`。拒绝时这一轮不发出。
   *   - `agent/request`：params `{ agent, turn, step, signal }`，返回 `{ provider, model, reasoningEffort }`。
   * 同一条用户消息只跑一次这两条链（按 clientUserMessageId 缓存），结果套用到它的所有候选帧上。
   */
  const codexPlusRpcInterceptorVersion = 19;
  const codexPlusRpcMiddlewareTimeoutMs = 5000;
  const codexPlusRpcPreparationTimeoutMs = 120000;
  const codexPlusRpcOpaque = Symbol("codexPlusRpcOpaque");
  const codexPlusRpcSourceItem = Symbol("codexPlusRpcSourceItem");
  const codexPlusRpcPushFailure = Symbol("codexPlusRpcPushFailure");
  const codexPlusRpcPortQueues = new WeakMap();
  const codexPlusRpcRejectionPorts = window.__codexPlusRpcRejectionPorts ||= new WeakMap();
  const codexPlusRpcCreationFailures = window.__codexPlusRpcCreationFailures ||= new WeakMap();
  const codexPlusRpcResumeGates = window.__codexPlusRpcResumeGates ||= new WeakMap();
  const codexPlusRpcTurnCache = new Map();
  const codexPlusRpcTurnCacheLimit = 64;
  const codexPlusRpcHostReceivers = window.__codexPlusRpcHostReceivers
    ||= new WeakMap();
  const codexPlusRpcTurnPreparations = window.__codexPlusRpcTurnPreparations ||= new Map();

  async function codexPlusRpcSerializeTurn(threadId, prepare) {
    if (typeof threadId !== "string" || !threadId) return prepare();
    const previous = codexPlusRpcTurnPreparations.get(threadId);
    let release;
    const tail = new Promise((resolve) => { release = resolve; });
    codexPlusRpcTurnPreparations.set(threadId, tail);
    await previous;
    try {
      return await prepare();
    } finally {
      if (codexPlusRpcTurnPreparations.get(threadId) === tail) codexPlusRpcTurnPreparations.delete(threadId);
      release();
    }
  }

  function codexPlusRpcContextEnabled() {
    return typeof codexPerModelContextEnabled === "function" && codexPerModelContextEnabled();
  }

  function codexPlusRpcReceiverHost(port, expr) {
    let receivers = codexPlusRpcHostReceivers.get(port);
    if (expr[2][0] === "subscribe") {
      const hostId = expr[3][0]?.key?.hostId;
      if (typeof hostId === "string" && hostId) {
        if (!receivers) codexPlusRpcHostReceivers.set(port, receivers = new Map());
        const previous = receivers.get(expr[1]);
        receivers.set(expr[1], previous === undefined || previous === hostId ? hostId : "");
      }
    }
    return receivers?.get(expr[1]) || "";
  }

  function codexPlusRpcIsPlainObject(value) {
    if (!value || typeof value !== "object") return false;
    const proto = Object.getPrototypeOf(value);
    return proto === Object.prototype || proto === null;
  }

  /** capnweb 值 → 普通 JS 值。数组编码成 `[[…]]`；其它数组是带标签的值（导出的回调、错误等），原样包起来不给改。 */
  function codexPlusRpcDecode(value) {
    if (Array.isArray(value)) {
      if (value.length === 1 && Array.isArray(value[0])) return value[0].map(codexPlusRpcDecode);
      return { [codexPlusRpcOpaque]: value };
    }
    if (codexPlusRpcIsPlainObject(value)) {
      const out = {};
      for (const key of Object.keys(value)) out[key] = codexPlusRpcDecode(value[key]);
      return out;
    }
    return value;
  }

  function codexPlusRpcEncode(value) {
    if (value && typeof value === "object" && Object.prototype.hasOwnProperty.call(value, codexPlusRpcOpaque)) {
      return value[codexPlusRpcOpaque];
    }
    if (Array.isArray(value)) return [value.map(codexPlusRpcEncode)];
    if (codexPlusRpcIsPlainObject(value)) {
      const out = {};
      for (const key of Object.keys(value)) out[key] = codexPlusRpcEncode(value[key]);
      return out;
    }
    return value;
  }

  function codexPlusRpcHasListeners(name) {
    try {
      return !!codexPlusExtensionListeners.get(name)?.size;
    } catch {
      return false;
    }
  }

  function codexPlusRpcWantsRequestChain() {
    try {
      return codexPlusExtensionInterceptors.has("codex/request");
    } catch {
      return false;
    }
  }

  function codexPlusRpcWantsTurnHooks() {
    return codexPlusRpcHasListeners("agent/pre-step")
      || codexPlusRpcHasListeners("agent/request")
      || codexPlusRpcWantsRequestChain();
  }

  /**
   * 可等待下游的 waterfall：监听器按注册顺序 `handler(params, next)`，`next()` 不带参数、返回下游决定，
   * 链尾是 fallback。另外做了出错隔离：监听器抛错或 reject 时记到该拓展名下，按调用 next() 处理，
   * 不能因为一个拓展挡住 Codex 发消息。返回 undefined 也按 next() 处理。
   */
  function codexPlusRunListenerWaterfall(name, params, fallback) {
    const chain = [...(codexPlusExtensionListeners.get(name) || [])];
    const step = (index) => {
      if (index >= chain.length) return Promise.resolve().then(fallback);
      const item = chain[index];
      let downstream = null;
      const next = () => (downstream ||= step(index + 1));
      const outcome = runCodexPlusExtensionCallback(item.scriptKey, `on:${name}`, () => item.handler(params, next));
      if (!outcome.ok) return next();
      return Promise.resolve(outcome.value).then(
        (value) => (value === undefined ? next() : value),
        (error) => {
          codexPlusMarkExtensionFailure(item.scriptKey, `on:${name}: ${String(error?.stack || error?.message || error)}`);
          return next();
        },
      );
    };
    return step(0);
  }

  /** Codex 的 input 条目 → 拓展看到的用户消息。非文本条目（图片、提及等）原样放进 content，改写回去时保留。 */
  function codexPlusRpcInputToMessages(input, id) {
    const content = (Array.isArray(input) ? input : []).map((item) => {
      if (item?.type !== "text") return { ...item };
      const part = { type: "text", text: String(item.text ?? "") };
      Object.defineProperty(part, codexPlusRpcSourceItem, { value: item });
      return part;
    });
    return [{ role: "user", id: String(id || ""), source: { kind: "user" }, content }];
  }

  /** 拓展消息 → Codex 的 input 条目。文本没改的条目沿用原条目（保留 text_elements 等标注）。 */
  function codexPlusRpcMessagesToInput(messages) {
    const items = [];
    for (const message of Array.isArray(messages) ? messages : []) {
      const content = typeof message?.content === "string"
        ? [{ type: "text", text: message.content }]
        : Array.isArray(message?.content) ? message.content : [];
      for (const part of content) {
        if (!part || typeof part !== "object") continue;
        if (part.type !== "text") {
          items.push({ ...part });
          continue;
        }
        const text = String(part.text ?? "");
        const source = part[codexPlusRpcSourceItem];
        items.push(source && source.text === text ? source : { type: "text", text, text_elements: [] });
      }
    }
    return items;
  }

  function codexPlusRpcSplitModel(model) {
    const id = typeof model === "string" ? model : "";
    const slash = id.indexOf("/");
    return slash > 0 ? { provider: id.slice(0, slash), base: id.slice(slash + 1) } : { provider: undefined, base: id };
  }

  /**
   * 对一条用户消息跑 `agent/pre-step` 与 `agent/request`。
   * 返回 { reject } 或 { input, model, reasoningEffort }（没变的字段为 undefined）。
   */
  async function codexPlusRpcRunTurnHooks({ messageId, conversationId, input, settings, signal }) {
    const agent = Object.freeze({
      id: String(conversationId || messageId || ""),
      session: Object.freeze({ id: String(conversationId || "") }),
    });
    const result = {};
    if (codexPlusRpcHasListeners("agent/pre-step")) {
      const messages = codexPlusRpcInputToMessages(input, messageId);
      const decision = await codexPlusRunListenerWaterfall(
        "agent/pre-step",
        { agent, turn: undefined, step: 1, messages, signal, conversationId },
        () => ({ kind: "enter", messages }),
      );
      if (decision?.kind === "reject") return { reject: String(decision.reason || "已被拓展拦截") };
      if (decision?.kind === "enter" && Array.isArray(decision.messages) && decision.messages !== messages) {
        result.input = codexPlusRpcMessagesToInput(decision.messages);
      }
    }
    if (codexPlusRpcHasListeners("agent/request")) {
      const { provider, base } = codexPlusRpcSplitModel(settings?.model);
      const seed = Object.freeze({
        provider,
        model: typeof settings?.model === "string" ? settings.model : undefined,
        reasoningEffort: settings?.reasoning_effort ?? undefined,
      });
      const config = await codexPlusRunListenerWaterfall(
        "agent/request",
        { agent, turn: undefined, step: 1, signal, conversationId },
        () => seed,
      );
      if (config && typeof config === "object" && config !== seed) {
        if (typeof config.model === "string" && config.model && config.model !== seed.model) {
          result.model = config.model;
        } else if (typeof config.provider === "string" && config.provider && config.provider !== provider && base) {
          // 只换了供应商：沿用 Codex++ 的「供应商/模型」写法拼回去。
          result.model = `${config.provider}/${base}`;
        }
        if (typeof config.reasoningEffort === "string" && config.reasoningEffort !== seed.reasoningEffort) {
          result.reasoningEffort = config.reasoningEffort;
        }
      }
    }
    return result;
  }

  /** 同一条用户消息的多个候选帧共用一次 hooks 结果。 */
  function codexPlusRpcTurnHooksOnce(key, run) {
    if (!key) return run();
    const cached = codexPlusRpcTurnCache.get(key);
    if (cached) return cached;
    const pending = run();
    codexPlusRpcTurnCache.set(key, pending);
    while (codexPlusRpcTurnCache.size > codexPlusRpcTurnCacheLimit) {
      codexPlusRpcTurnCache.delete(codexPlusRpcTurnCache.keys().next().value);
    }
    return pending;
  }

  function codexPlusRpcApplySettings(holder, outcome) {
    const settings = holder?.collaborationMode?.settings;
    if (!codexPlusRpcIsPlainObject(settings)) return;
    if (outcome.model !== undefined) settings.model = outcome.model;
    if (outcome.reasoningEffort !== undefined) settings.reasoning_effort = outcome.reasoningEffort;
  }

  /** 跑 `codex/request` 链；返回新的 params，没改返回原对象。 */
  async function codexPlusRpcRunRequestChain(method, params) {
    if (!codexPlusRpcWantsRequestChain()) return params;
    const result = await codexPlusRunWaterfall("codex/request", { method, params });
    return result && typeof result === "object" && result.params !== undefined ? result.params : params;
  }

  function codexPlusRpcRejectNotice(reason) {
    try {
      if (typeof showToast === "function") showToast(reason, { type: "warn" });
    } catch {}
  }

  function codexPlusRpcRejectPush(frame, reason) {
    const rejected = [...frame];
    rejected[codexPlusRpcPushFailure] = ["error", "Error", reason];
    return rejected;
  }

  /**
   * 处理一轮对话的参数（就地改 decoded 里的对象）。
   * carriers：[{ method, params, conversationId }]，第一个用来取 input；settings 是本轮的模型设置
   * （缺省取第一个 carrier 的）。settingsHolders：需要同步模型设置的对象（resume 等）。返回 { reject } 或 {}。
   */
  async function codexPlusRpcProcessTurn({ messageId, carriers, settings, settingsHolders, signal }) {
    const primary = carriers[0];
    const outcome = await codexPlusRpcTurnHooksOnce(messageId, () => codexPlusRpcRunTurnHooks({
      messageId,
      conversationId: primary.conversationId,
      input: primary.params.input,
      settings: settings || primary.params.collaborationMode?.settings,
      signal,
    }));
    if (outcome.reject) return outcome;
    for (const carrier of carriers) {
      if (outcome.input) carrier.params.input = outcome.input.map((item) => (codexPlusRpcIsPlainObject(item) ? { ...item } : item));
      codexPlusRpcApplySettings(carrier.params, outcome);
      if (outcome.model !== undefined && carrier.params.model != null) carrier.params.model = outcome.model;
      if (outcome.reasoningEffort !== undefined && carrier.params.effort != null) carrier.params.effort = outcome.reasoningEffort;
    }
    for (const holder of settingsHolders) codexPlusRpcApplySettings(holder, outcome);
    for (const carrier of carriers) {
      const next = await codexPlusRpcRunRequestChain(carrier.method, carrier.params);
      if (next !== carrier.params && codexPlusRpcIsPlainObject(next)) carrier.replace(next);
    }
    return {};
  }

  function codexPlusRpcPlanSendRequest(frame, hostId) {
    const expr = frame[1];
    const args = expr[3];
    const method = args[0];
    const refreshContext = hostId === "local" && codexPlusRpcContextEnabled()
      && ["thread/start", "thread/fork"].includes(method);
    if (typeof method !== "string" || (!codexPlusRpcWantsRequestChain() && !refreshContext)) return null;
    const plan = async () => {
      const params = codexPlusRpcDecode(args[1]);
      const next = await codexPlusRpcRunRequestChain(method, params);
      if (next === params) return null;
      return ["push", [...expr.slice(0, 3), [method, codexPlusRpcEncode(next), ...args.slice(2)], ...expr.slice(4)], ...frame.slice(2)];
    };
    if (refreshContext) {
      plan.context = async (nextFrame) => {
        const nextExpr = nextFrame[1];
        const nextArgs = nextExpr[3];
        const params = await prepareCodexThreadContextStart(method, codexPlusRpcDecode(nextArgs[1]), hostId);
        return ["push", [...nextExpr.slice(0, 3), [method, codexPlusRpcEncode(params), ...nextArgs.slice(2)],
          ...nextExpr.slice(4)], ...nextFrame.slice(2)];
      };
      plan.contextFailure = (reason) => codexPlusRpcRejectPush(frame, reason);
    }
    return plan;
  }

  function codexPlusRpcPlanStartConversation(frame, hostId) {
    const expr = frame[1];
    const args = expr[3];
    const refreshContext = hostId === "local" && codexPlusRpcContextEnabled();
    if (!codexPlusRpcIsPlainObject(args[0]) || !Array.isArray(args[0].input)
        || (!codexPlusRpcWantsTurnHooks() && !refreshContext)) return null;
    const plan = async (signal) => {
      let params = codexPlusRpcDecode(args[0]);
      const outcome = await codexPlusRpcProcessTurn({
        messageId: params.clientUserMessageId,
        carriers: [{ method: "turn/start", params, conversationId: undefined, replace: (next) => { params = next; } }],
        settingsHolders: [],
        signal,
      });
      if (outcome.reject) {
        codexPlusRpcRejectNotice(outcome.reject);
        return codexPlusRpcRejectPush(frame, outcome.reject);
      }
      return ["push", [...expr.slice(0, 3), [codexPlusRpcEncode(params), ...args.slice(1)], ...expr.slice(4)], ...frame.slice(2)];
    };
    if (refreshContext) {
      plan.context = async (nextFrame) => {
        if (nextFrame[1][2][0] !== "startConversation") return nextFrame;
        const nextExpr = nextFrame[1];
        const nextArgs = nextExpr[3];
        const params = await prepareCodexThreadContextStart("thread/start", codexPlusRpcDecode(nextArgs[0]), hostId);
        return ["push", [...nextExpr.slice(0, 3), [codexPlusRpcEncode(params), ...nextArgs.slice(1)],
          ...nextExpr.slice(4)], ...nextFrame.slice(2)];
      };
      plan.contextFailure = (reason) => codexPlusRpcRejectPush(frame, reason);
    }
    return plan;
  }

  function codexPlusRpcPlanResume(frame, hostId, plan) {
    const expr = frame[1];
    const method = expr[2][0];
    const index = method === "sendRequest" ? 1 : 0;
    const isResumeRequest = expr[3][0] === "thread/resume"
      || (expr[3][0] === "send-cli-request-for-host" && expr[3][1]?.method === "thread/resume");
    const options = method === "sendRequest" && isResumeRequest ? expr[3][2] : null;
    const internalResume = options?.__codexPlusProviderResume;
    if (internalResume !== undefined) {
      const inner = plan;
      plan = async (signal) => {
        expr[3][2] = { ...options };
        delete expr[3][2].__codexPlusProviderResume;
        return inner ? inner(signal) : null;
      };
    }
    if (hostId !== "local" || typeof prepareCodexThreadProviderResume !== "function"
        || (method === "sendRequest" && expr[3][0] !== "thread/resume")
        || !codexPlusRpcIsPlainObject(expr[3][index])) return plan;
    if ([expr[3][index].hostId, expr[3][index].host_id]
        .some((host) => typeof host === "string" && host !== "local")) return plan;
    // 在中间件改写前识别内部恢复；它已持有会话准备锁，不能再次排队等待自己。
    const original = expr[3][index];
    const preparation = typeof codexThreadProviderPreparations !== "undefined"
      ? codexThreadProviderPreparations.get(original.threadId || original.conversationId) : null;
    const resuming = preparation?.resuming;
    if (method === "sendRequest" && resuming && internalResume === resuming) return plan;
    if (method === "sendRequest" && typeof codexGatewayPreparations !== "undefined"
        && internalResume !== undefined && internalResume === codexGatewayPreparations.get(original.threadId)?.resuming) return plan;
    const managed = typeof codexManagedSessionConnection === "function" && codexManagedSessionConnection();
    const previousManaged = (original.modelProvider ?? original.model_provider
      ?? original.config?.model_provider) === "codex_plus_managed";
    const inspectManaged = typeof codexRemoteSessionActiveProfile === "function" && !!codexRemoteSessionActiveProfile();
    if (!codexPlusBackendSettings.providerSyncEnabled && !preparation?.tail
        && !managed && !previousManaged && !inspectManaged) return plan;
    plan ||= async () => null;
    plan.resumeIndex = index;
    plan.before = async (wireParams, dataOnly, _candidate, signal) => {
      signal?.throwIfAborted();
      const params = codexPlusRpcDecode(wireParams);
      if (!codexPlusBackendSettings.providerSyncEnabled && !managed && !previousManaged && !inspectManaged) {
        await preparation.tail;
        signal?.throwIfAborted();
        return wireParams;
      }
      const client = await codexPlusAppServerClient(hostId);
      signal?.throwIfAborted();
      const prepared = await prepareCodexThreadProviderResume(params, hostId,
        (method, params, options) => client.sendRequest(method, params, options), client, null, signal,
        false, previousManaged || inspectManaged);
      signal?.throwIfAborted();
      if (prepared && dataOnly && method === "resumeConversation") {
        const cached = await client.getConversation(params.conversationId);
        signal?.throwIfAborted();
        const error = cached?.providerBlock?.error;
        if (error?.type === "modelProviderMismatch" && error.modelProvider === prepared.sourceProvider
            && error.currentModelProvider === prepared.targetProvider) {
          return { ...wireParams, intent: "open" };
        }
      }
      return wireParams;
    };
    return plan;
  }

  function codexPlusRpcDataOnly(value) {
    if (Array.isArray(value)) {
      return value.length === 1 && Array.isArray(value[0]) && value[0].every(codexPlusRpcDataOnly);
    }
    return !codexPlusRpcIsPlainObject(value) || Object.values(value).every(codexPlusRpcDataOnly);
  }

  function codexPlusRpcPlanResolve(frame) {
    const value = frame[2];
    if (!codexPlusRpcIsPlainObject(value)) return null;
    const hasStart = codexPlusRpcIsPlainObject(value.start?.request) && Array.isArray(value.start.request.input);
    const hasSteer = codexPlusRpcIsPlainObject(value.steer) && Array.isArray(value.steer.input);
    const refreshContext = hasStart && value.executionHostId === "local" && codexPlusRpcContextEnabled();
    if (!hasStart && !hasSteer) return null;
    const localResume = value.executionHostId === "local"
      && codexPlusRpcIsPlainObject(value.resume)
      && typeof value.conversationId === "string" && value.conversationId === value.resume.conversationId
      && (value.start?.request?.threadId === undefined || value.start.request.threadId === value.conversationId);
    const prepareGateway = hasStart && localResume
      && typeof codexGatewayRecoveryEnabled === "function" && codexGatewayRecoveryEnabled();
    const prepareProvider = localResume && typeof prepareCodexThreadProviderResume === "function"
      && (codexPlusBackendSettings.providerSyncEnabled
        || (typeof codexRemoteSessionActiveProfile === "function" && codexRemoteSessionActiveProfile()));
    const wantsHooks = codexPlusRpcWantsTurnHooks();
    if (!wantsHooks && !prepareProvider && !prepareGateway && !refreshContext) return null;
    const plan = async (signal) => {
      if (!wantsHooks) return null;
      const decoded = codexPlusRpcDecode(value);
      const conversationId = decoded.conversationId;
      const carriers = [];
      if (hasStart) {
        carriers.push({
          method: "turn/start",
          params: decoded.start.request,
          conversationId,
          replace: (next) => { decoded.start.request = next; },
        });
      }
      if (hasSteer) {
        carriers.push({
          method: "turn/steer",
          params: decoded.steer,
          conversationId,
          replace: (next) => { decoded.steer = next; },
        });
      }
      const messageId = decoded.start?.request?.clientUserMessageId || decoded.steer?.clientUserMessageId;
      const outcome = await codexPlusRpcProcessTurn({
        messageId,
        carriers,
        // 宿主常先只要 steer 候选（不带模型设置），这时从 resume 取，免得缓存下一份空配置。
        settings: decoded.start?.request?.collaborationMode?.settings || decoded.resume?.collaborationMode?.settings,
        settingsHolders: [decoded.resume].filter(Boolean),
        signal,
      });
      if (outcome.reject) {
        codexPlusRpcRejectNotice(outcome.reject);
        return ["reject", frame[1], ["error", "Error", outcome.reject]];
      }
      return ["resolve", frame[1], codexPlusRpcEncode(decoded), ...frame.slice(3)];
    };
    if (refreshContext || prepareGateway) {
      plan.context = async (nextFrame, signal) => {
        signal?.throwIfAborted();
        if (nextFrame[0] !== "resolve") return nextFrame;
        const decoded = codexPlusRpcDecode(nextFrame[2]);
        let client;
        try {
          client = await codexPlusAppServerClient("local");
          if (!client) throw new Error("app-server client unavailable");
        } catch (error) {
          signal?.throwIfAborted();
          if (refreshContext) throw error;
          codexPlusMarkExtensionFailure("", `网关连接检查不可用：${String(error?.message || error)}`);
          return nextFrame;
        }
        signal?.throwIfAborted();
        if (prepareGateway) {
          await prepareCodexGatewayReconnect("turn/start",
            { ...decoded.start.request, threadId: decoded.conversationId }, "local",
            (method, params, options) => client.sendRequest(method, params, options), client, signal);
        }
        if (!refreshContext) return nextFrame;
        const sendRequest = async (method, params, options) => {
          signal?.throwIfAborted();
          if (method === "thread/unsubscribe") {
            await client.unsubscribeInactiveConversation(params.threadId);
            signal?.throwIfAborted();
            // 宿主方法会内部记录并吞掉失败；原始请求幂等，确保原生错误传回当前发送。
          }
          const result = method === "thread/resume" && params.config
            ? await client.resumeThread(params) : await client.sendRequest(method, params, options);
          signal?.throwIfAborted();
          return result;
        };
        await refreshCodexThreadModelBeforeTurn({ hostId: "local" }, sendRequest,
          "turn/start", decoded.start.request);
        signal?.throwIfAborted();
        return nextFrame;
      };
      plan.contextFailure = (reason) => ["reject", frame[1], ["error", "Error", reason]];
    }
    if (prepareProvider) {
      // 暂存纯数据字段，其余候选/export 先交原生导入，失败时照常释放。
      plan.resumeIndex = "conversationId";
      plan.before = async (conversationId, _dataOnly, candidate, signal) => {
        signal?.throwIfAborted();
        if (candidate.executionHostId !== "local" || conversationId !== candidate.resume?.conversationId
            || (candidate.start?.request?.threadId !== undefined
              && candidate.start.request.threadId !== conversationId)) return conversationId;
        const decoded = codexPlusRpcDecode(candidate);
        const params = { ...decoded.resume };
        if (prepareGateway) {
          const start = decoded.start.request;
          params.model = start.model ?? params.model;
          params.effort = start.effort ?? params.effort;
          params.collaborationMode = start.collaborationMode ?? params.collaborationMode;
        }
        const provider = decoded.resume.modelProvider
          ?? decoded.resume.model_provider ?? decoded.resume.config?.model_provider
          ?? decoded.steer?.restoreMessage?.submissionOptions?.modelProvider;
        if (provider !== undefined) params.modelProvider = provider;
        let inspectManaged = provider === "codex_plus_managed"
          || !!window.__codexPlusManagedSessionStarts?.has(conversationId)
          || (typeof codexManagedSessionConnection === "function" && !!codexManagedSessionConnection())
          || (typeof codexThreadProviderPreparations !== "undefined"
            && (!!codexThreadProviderPreparations.get(conversationId)?.tail
              || codexThreadProviderPreparations.get(conversationId)?.preparedProvider === "codex_plus_managed"));
        let client;
        const sendRequest = async (method, requestParams, options) => {
          signal?.throwIfAborted();
          client ||= await codexPlusAppServerClient("local");
          signal?.throwIfAborted();
          return client.sendRequest(method, requestParams, options);
        };
        if (!codexPlusBackendSettings.providerSyncEnabled && !inspectManaged) {
          let cacheTimer;
          try {
            const result = await Promise.race([
              (async () => {
                const found = await codexPlusAppServerClient("local");
                return { client: found, cached: await found.getConversation(conversationId) };
              })(),
              new Promise((_, reject) => {
                cacheTimer = setTimeout(() => reject(new Error("读取会话缓存超时")), codexAppServerPreparationTimeoutMs);
              }),
            ]);
            signal?.throwIfAborted();
            client = result.client;
            const cached = result.cached;
            inspectManaged = cached?.modelProvider === "codex_plus_managed";
            if (!inspectManaged && cached?.threadRuntimeStatus?.type === "active") {
              const prepared = codexThreadProviderPreparations.get(conversationId);
              const connected = prepared?.client === client && prepared.preparedProvider
                ? prepared.preparedProvider : cached.modelProvider;
              const target = codexRemoteSessionTargetProvider();
              if (connected && target && connected !== target) warnCodexActiveOldProvider();
            }
          } catch (error) {
            signal?.throwIfAborted();
            sendCodexPlusDiagnostic("thread_provider_cache_read_failed", { errorMessage: String(error?.message || error) });
          } finally {
            clearTimeout(cacheTimer);
          }
        }
        if (!codexPlusBackendSettings.providerSyncEnabled && !inspectManaged) return conversationId;
        client ||= await codexPlusAppServerClient("local");
        signal?.throwIfAborted();
        await prepareCodexThreadProviderResume(params, "local", sendRequest, client, null, signal, hasStart, inspectManaged);
        signal?.throwIfAborted();
        return conversationId;
      };
    }
    return plan;
  }

  /** 判断一帧要不要拦。返回 null（同步放行）或 async (signal) => 新帧 | null。 */
  function codexPlusRpcPlanFrame(frame, port) {
    if (!Array.isArray(frame)) return null;
    if (frame[0] === "push") {
      const expr = frame[1];
      if (!Array.isArray(expr) || expr[0] !== "pipeline" || !Array.isArray(expr[2]) || expr[2].length !== 1 || !Array.isArray(expr[3])) {
        return null;
      }
      const hostId = codexPlusRpcReceiverHost(port, expr);
      if (expr[2][0] === "sendRequest") {
        return codexPlusRpcPlanResume(frame, hostId, codexPlusRpcPlanSendRequest(frame, hostId));
      }
      if (["resumeConversation", "resumeThread"].includes(expr[2][0])) {
        return codexPlusRpcPlanResume(frame, hostId, null);
      }
      if (expr[2][0] === "startConversation") return codexPlusRpcPlanStartConversation(frame, hostId);
      return null;
    }
    if (frame[0] === "resolve" && typeof frame[1] === "number" && frame.length > 2) return codexPlusRpcPlanResolve(frame);
    return null;
  }

  /** 带超时地跑一个改写计划；超时或出错返回 null（原样发出），同时 abort 交给拓展的 signal。 */
  async function codexPlusRpcRunPlan(plan, frame, deferContext = false) {
    const controller = new AbortController();
    let timer = null;
    let replaced = null;
    try {
      replaced = await Promise.race([
        Promise.resolve().then(() => plan(controller.signal)),
        new Promise((resolve) => {
          timer = setTimeout(() => {
            controller.abort(new Error("Codex++ 拓展中间件超时"));
            resolve(null);
          }, codexPlusRpcMiddlewareTimeoutMs);
        }),
      ]);
    } catch (error) {
      codexPlusMarkExtensionFailure("", `rpc 拦截失败，已原样发出：${String(error?.message || error)}`);
    } finally {
      clearTimeout(timer);
    }
    if (!plan.context || deferContext || replaced?.[codexPlusRpcPushFailure]) return replaced;
    try {
      return await plan.context(replaced || frame);
    } catch (error) {
      const reason = String(error?.message || error);
      codexPlusRpcRejectNotice(reason);
      return plan.contextFailure(reason);
    }
  }

  function codexPlusRpcIncomingFrame(port, incoming) {
    const creation = codexPlusRpcCreationFailures.get(port);
    const rejections = codexPlusRpcRejectionPorts.get(port);
    const gates = codexPlusRpcResumeGates.get(port);
    if (!creation && !rejections && !gates) return incoming;
    if (incoming === null) {
      creation?.callbacks.clear();
      gates?.pending.clear();
      for (const controller of gates?.controllers?.values() || []) {
        controller.abort(new Error("会话准备已取消"));
      }
      gates?.controllers?.clear();
      return incoming;
    }
    if (incoming?.[0] === "release") {
      creation?.callbacks.delete(incoming[1]);
      const releasedGate = gates?.pending.delete(incoming[1]);
      if (releasedGate) {
        gates.controllers?.get(incoming[1])?.abort(new Error("会话准备已取消"));
        gates.controllers?.delete(incoming[1]);
      }
      if (rejections?.pending.delete(incoming[1]) || releasedGate) {
        // 未知 import 的 resolve 是协议空操作；正半整数不属于任何原生或本层编号。
        return ["resolve", 0.5, ["undefined"]];
      }
    }
    const expr = incoming?.[1];
    if (!creation || incoming?.[0] !== "push" || expr?.[0] !== "pipeline"
        || expr[2]?.length !== 0 || !Array.isArray(expr[3])) return incoming;
    const failures = creation.callbacks.get(expr[1]);
    const messageId = expr[3][0]?.clientUserMessageId;
    const error = failures?.get(messageId);
    if (!error) return incoming;
    failures.delete(messageId);
    if (!failures.size) creation.callbacks.delete(expr[1]);
    const id = creation.nextId--;
    creation.pending.add(id);
    const transformed = ["push", [...expr.slice(0, 3), [...expr[3], ["promise", id]], ...expr.slice(4)], ...incoming.slice(2)];
    queueMicrotask(() => port.dispatchEvent(new MessageEvent("message", {
      data: ["reject", id, error],
    })));
    return transformed;
  }

  /** 某个端口开始等中间件：之后的帧全部排队，按原顺序逐个处理、发出。 */
  async function codexPlusRpcDrainPort(port, original, first) {
    let finish;
    const state = { queue: [], done: new Promise((resolve) => { finish = resolve; }) };
    codexPlusRpcPortQueues.set(port, state);
    try {
      let entry = first;
      while (entry) {
        const [message, rest, plan] = entry;
        const deferContext = message[0] === "resolve" && plan?.before && plan.context;
        const replaced = plan ? await codexPlusRpcRunPlan(plan, message, deferContext) : null;
        try {
          const failure = replaced?.[codexPlusRpcPushFailure];
          const readInputs = failure && replaced[1][2][0] === "startConversation"
            && replaced[1][3][1]?.readThreadCreationInputs;
          if (readInputs?.[0] === "export" && typeof readInputs[1] === "number") {
            let creation = codexPlusRpcCreationFailures.get(port);
            if (!creation) {
              creation = { nextId: -0.5, pending: new Set(), callbacks: new Map() };
              codexPlusRpcCreationFailures.set(port, creation);
            }
            let failures = creation.callbacks.get(readInputs[1]);
            if (!failures) creation.callbacks.set(readInputs[1], failures = new Map());
            failures.set(replaced[1][3][0].clientUserMessageId, failure);
            original.call(port, [...replaced], ...rest);
          } else if (failure) {
            let rejections = codexPlusRpcRejectionPorts.get(port);
            if (!rejections) {
              rejections = { nextId: -0.5, pending: new Set() };
              codexPlusRpcRejectionPorts.set(port, rejections);
            }
            // 原生 export id 是整数；附加 promise 使用独立编号，release 由本层消费。
            // 保留原参数中的全部 export；Capnweb 等参数 promise 拒绝后不会调用后端方法。
            const id = rejections.nextId--;
            rejections.pending.add(id);
            const expr = replaced[1];
            original.call(port, ["push", [...expr.slice(0, 3), [...expr[3], ["promise", id]],
              ...expr.slice(4)], ...replaced.slice(2)], ...rest);
            original.call(port, ["reject", id, failure]);
          } else if (plan?.before && replaced?.[0] !== "reject") {
            let gates = codexPlusRpcResumeGates.get(port);
            if (!gates) {
              gates = { nextId: -0.25, pending: new Set() };
              codexPlusRpcResumeGates.set(port, gates);
            }
            const id = gates.nextId--;
            gates.pending.add(id);
            const controller = new AbortController();
            (gates.controllers ||= new Map()).set(id, controller);
            const next = replaced || message;
            const resolving = next[0] === "resolve";
            const expr = next[1];
            const args = resolving ? { ...next[2] } : [...expr[3]];
            const params = args[plan.resumeIndex];
            const dataOnly = codexPlusRpcDataOnly(params);
            // 含 export 的参数留在原 push 导入，失败时由 Capnweb 正常释放。
            if (resolving || dataOnly) args[plan.resumeIndex] = ["promise", id];
            else args.push(["promise", id]);
            original.call(port, resolving
              ? ["resolve", next[1], args, ...next.slice(3)]
              : ["push", [...expr.slice(0, 3), args, ...expr.slice(4)], ...next.slice(2)], ...rest);
            // 先释放整个发送队列，再让预检经同一端口发出。
            void state.done.then(async () => {
              let timer;
              try {
                controller.signal.throwIfAborted();
                if (!gates.pending.has(id)) return params;
                const timeout = new Promise((_, reject) => {
                  timer = setTimeout(() => {
                    const error = new Error("会话准备超时，请重试");
                    controller.abort(error);
                    reject(error);
                  }, codexPlusRpcPreparationTimeoutMs);
                });
                const prepare = async () => {
                  controller.signal.throwIfAborted();
                  if (!gates.pending.has(id)) return params;
                  const prepared = await plan.before(params, dataOnly, resolving ? next[2] : args, controller.signal);
                  controller.signal.throwIfAborted();
                  if (deferContext && gates.pending.has(id)) await plan.context(next, controller.signal);
                  controller.signal.throwIfAborted();
                  return prepared;
                };
                const work = resolving ? codexPlusRpcSerializeTurn(next[2].conversationId, prepare) : prepare();
                return await Promise.race([work, timeout]);
              } finally {
                clearTimeout(timer);
                gates.controllers.delete(id);
              }
            }).then(
              (prepared) => {
                if (gates.pending.has(id)) original.call(port, ["resolve", id, dataOnly ? prepared : ["undefined"]]);
              },
              (error) => {
                if (gates.pending.has(id)) {
                  const reason = String(error?.message || error);
                  codexPlusRpcRejectNotice(reason);
                  original.call(port, ["reject", id, ["error", error?.name || "Error", reason]]);
                }
              },
            ).catch((error) => codexPlusMarkExtensionFailure("", `rpc 帧发送失败：${String(error?.message || error)}`));
          } else {
            original.call(port, replaced || message, ...rest);
          }
        } catch (error) {
          // 已经是异步发送，异常没法还给调用方；记下来，后面的帧照常发。
          codexPlusMarkExtensionFailure("", `rpc 帧发送失败：${String(error?.message || error)}`);
        }
        const queued = state.queue.shift();
        if (!queued) break;
        let nextPlan = null;
        try {
          nextPlan = codexPlusRpcPlanFrame(queued[0], port);
        } catch {}
        entry = [queued[0], queued[1], nextPlan];
      }
    } finally {
      codexPlusRpcPortQueues.delete(port);
      finish();
    }
  }

  function installCodexPlusRpcFrameInterceptor() {
    const proto = window.MessagePort?.prototype;
    if (!proto || typeof proto.postMessage !== "function") return false;
    const installed = window.__codexPlusRpcFrameInterceptor;
    if (installed?.version >= codexPlusRpcInterceptorVersion && proto.postMessage === installed.patched) return true;
    // 重复注入时包原生的那个，不叠层。
    const original = installed?.original || proto.postMessage;
    window.__codexPlusRpcIncomingFrame = codexPlusRpcIncomingFrame;
    const eventProto = window.MessageEvent?.prototype;
    if (eventProto && !window.__codexPlusRpcMessageData) {
      const descriptor = Object.getOwnPropertyDescriptor(eventProto, "data");
      if (descriptor?.get) {
        const events = new WeakMap();
        Object.defineProperty(eventProto, "data", {
          ...descriptor,
          get() {
            if (events.has(this)) return events.get(this);
            const data = descriptor.get.call(this);
            const transformed = window.__codexPlusRpcIncomingFrame(this.currentTarget, data);
            if (transformed !== data) events.set(this, transformed);
            return transformed;
          },
        });
        window.__codexPlusRpcMessageData = true;
      }
    }
    const patched = function codexPlusRpcPostMessage(message, ...rest) {
      if (message?.[0] === "release" && codexPlusRpcCreationFailures.get(this)?.pending.delete(message[1])) {
        return undefined;
      }
      const state = codexPlusRpcPortQueues.get(this);
      let plan = null;
      try {
        plan = codexPlusRpcPlanFrame(message, this);
      } catch {}
      // resolve 不分配隐式 RPC 序号。单独等待它，预检发出的请求才能通过同一端口。
      if (message?.[0] === "resolve" && plan?.context && !plan.before) {
        const port = this;
        void (async () => {
          if (state) await state.done;
          const replaced = await codexPlusRpcRunPlan(plan, message);
          original.call(port, replaced || message, ...rest);
        })().catch((error) => {
          codexPlusMarkExtensionFailure("", `rpc 帧发送失败：${String(error?.message || error)}`);
        });
        return undefined;
      }
      if (state) {
        state.queue.push([message, rest]);
        return undefined;
      }
      if (!plan) return original.call(this, message, ...rest);
      void codexPlusRpcDrainPort(this, original, [message, rest, plan]);
      return undefined;
    };
    proto.postMessage = patched;
    window.__codexPlusRpcFrameInterceptor = { version: codexPlusRpcInterceptorVersion, original, patched };
    return true;
  }

  try {
    installCodexPlusRpcFrameInterceptor();
  } catch {}
