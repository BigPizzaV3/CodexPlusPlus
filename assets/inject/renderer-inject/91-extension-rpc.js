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
   * capnweb 的 import id 按 push 顺序隐式分配，所以改写某一帧要等中间件时，同一端口后面的帧
   * 必须排队、按原顺序发出。没有拓展挂这些拦截点时一律同步放行，零开销。
   *
   * 语义（`ctx.on(name, async (params, next) => …)`，先 `await next()` 拿下游决定再改）：
   *   - `agent/pre-step`：params `{ agent, turn, step, messages, signal }`，决定是
   *     `{ kind: "enter", messages }` 或 `{ kind: "reject", reason? }`。拒绝时这一轮不发出。
   *   - `agent/request`：params `{ agent, turn, step, signal }`，返回 `{ provider, model, reasoningEffort }`。
   * 同一条用户消息只跑一次这两条链（按 clientUserMessageId 缓存），结果套用到它的所有候选帧上。
   */
  const codexPlusRpcInterceptorVersion = 1;
  const codexPlusRpcMiddlewareTimeoutMs = 5000;
  const codexPlusRpcOpaque = Symbol("codexPlusRpcOpaque");
  const codexPlusRpcSourceItem = Symbol("codexPlusRpcSourceItem");
  const codexPlusRpcPortQueues = new WeakMap();
  const codexPlusRpcTurnCache = new Map();
  const codexPlusRpcTurnCacheLimit = 64;

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

  function codexPlusRpcPlanSendRequest(frame) {
    const expr = frame[1];
    const args = expr[3];
    const method = args[0];
    if (typeof method !== "string" || !codexPlusRpcWantsRequestChain()) return null;
    return async () => {
      const params = codexPlusRpcDecode(args[1]);
      const next = await codexPlusRpcRunRequestChain(method, params);
      if (next === params) return null;
      return ["push", [...expr.slice(0, 3), [method, codexPlusRpcEncode(next), ...args.slice(2)], ...expr.slice(4)], ...frame.slice(2)];
    };
  }

  function codexPlusRpcPlanStartConversation(frame) {
    const expr = frame[1];
    const args = expr[3];
    if (!codexPlusRpcIsPlainObject(args[0]) || !Array.isArray(args[0].input) || !codexPlusRpcWantsTurnHooks()) return null;
    return async (signal) => {
      let params = codexPlusRpcDecode(args[0]);
      const outcome = await codexPlusRpcProcessTurn({
        messageId: params.clientUserMessageId,
        carriers: [{ method: "turn/start", params, conversationId: undefined, replace: (next) => { params = next; } }],
        settingsHolders: [],
        signal,
      });
      if (outcome.reject) {
        codexPlusRpcRejectNotice(outcome.reject);
        // push 帧不能丢（会打乱 import id），改成调一个不存在的方法，让这次调用以错误结束。
        return ["push", ["pipeline", expr[1], ["__codexPlusRejectedByExtension"], []], ...frame.slice(2)];
      }
      return ["push", [...expr.slice(0, 3), [codexPlusRpcEncode(params), ...args.slice(1)], ...expr.slice(4)], ...frame.slice(2)];
    };
  }

  function codexPlusRpcPlanResolve(frame) {
    const value = frame[2];
    if (!codexPlusRpcIsPlainObject(value) || !codexPlusRpcWantsTurnHooks()) return null;
    const hasStart = codexPlusRpcIsPlainObject(value.start?.request) && Array.isArray(value.start.request.input);
    const hasSteer = codexPlusRpcIsPlainObject(value.steer) && Array.isArray(value.steer.input);
    if (!hasStart && !hasSteer) return null;
    return async (signal) => {
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
  }

  /** 判断一帧要不要拦。返回 null（同步放行）或 async (signal) => 新帧 | null。 */
  function codexPlusRpcPlanFrame(frame) {
    if (!Array.isArray(frame)) return null;
    if (frame[0] === "push") {
      const expr = frame[1];
      if (!Array.isArray(expr) || expr[0] !== "pipeline" || !Array.isArray(expr[2]) || expr[2].length !== 1 || !Array.isArray(expr[3])) {
        return null;
      }
      if (expr[2][0] === "sendRequest") return codexPlusRpcPlanSendRequest(frame);
      if (expr[2][0] === "startConversation") return codexPlusRpcPlanStartConversation(frame);
      return null;
    }
    if (frame[0] === "resolve" && typeof frame[1] === "number" && frame.length > 2) return codexPlusRpcPlanResolve(frame);
    return null;
  }

  /** 带超时地跑一个改写计划；超时或出错返回 null（原样发出），同时 abort 交给拓展的 signal。 */
  async function codexPlusRpcRunPlan(plan) {
    const controller = new AbortController();
    let timer = null;
    try {
      return await Promise.race([
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
      return null;
    } finally {
      clearTimeout(timer);
    }
  }

  /** 某个端口开始等中间件：之后的帧全部排队，按原顺序逐个处理、发出。 */
  async function codexPlusRpcDrainPort(port, original, first) {
    const state = { queue: [] };
    codexPlusRpcPortQueues.set(port, state);
    try {
      let entry = first;
      while (entry) {
        const [message, rest, plan] = entry;
        const replaced = plan ? await codexPlusRpcRunPlan(plan) : null;
        try {
          original.call(port, replaced || message, ...rest);
        } catch (error) {
          // 已经是异步发送，异常没法还给调用方；记下来，后面的帧照常发。
          codexPlusMarkExtensionFailure("", `rpc 帧发送失败：${String(error?.message || error)}`);
        }
        const queued = state.queue.shift();
        if (!queued) break;
        let nextPlan = null;
        try {
          nextPlan = codexPlusRpcPlanFrame(queued[0]);
        } catch {}
        entry = [queued[0], queued[1], nextPlan];
      }
    } finally {
      codexPlusRpcPortQueues.delete(port);
    }
  }

  function installCodexPlusRpcFrameInterceptor() {
    const proto = window.MessagePort?.prototype;
    if (!proto || typeof proto.postMessage !== "function") return false;
    const installed = window.__codexPlusRpcFrameInterceptor;
    if (installed?.version >= codexPlusRpcInterceptorVersion && proto.postMessage === installed.patched) return true;
    // 重复注入时包原生的那个，不叠层。
    const original = installed?.original || proto.postMessage;
    const patched = function codexPlusRpcPostMessage(message, ...rest) {
      const state = codexPlusRpcPortQueues.get(this);
      if (state) {
        state.queue.push([message, rest]);
        return undefined;
      }
      let plan = null;
      try {
        plan = codexPlusRpcPlanFrame(message);
      } catch {}
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
