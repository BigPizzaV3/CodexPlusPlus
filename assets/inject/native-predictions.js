(() => {
  // 原生输入预测（实验）：只在本机对话 + 自定义 API 供应商时，让 Codex 自带的灰字预测与 Tab 接受可用。
  // 补丁只放宽原生预测路径上与该场景相关的门控，采样通过 app-server 的 thread/sample 完成；
  // ChatGPT 登录、远端 / SSH / 云端对话全部回到原生实现。
  const root = typeof window === "object" ? window : {};
  const id = "[A-Za-z_$][\\w$]*";
  const featureGate = "`4290031608`";
  const handle = "window.__codexPlusNativePredictions";
  const promptVersion = "codex-plus-native-predictions-v1";
  const sampleDeadlineMs = 45_000;
  const providerTtlMs = 5 * 60_000;
  const maxCachedSamples = 32;
  const maxSuggestionLength = 600;
  // 官方身份不归本功能管：它们走原生控制器或保持原样。
  const officialProviders = new Set(["openai", "amazon-bedrock", "amazonbedrock"]);

  // 状态挂在 window 上：设置心跳会重新执行本脚本，进行中的采样和去重缓存不能丢。
  const state = root.__codexPlusNativePredictionsState && typeof root.__codexPlusNativePredictionsState === "object"
    ? root.__codexPlusNativePredictionsState
    : {
      settings: null,
      officialGate: false,
      preference: null,
      marks: {},
      unsupported: false,
      samples: new Map(),
      providers: new Map(),
      pending: new Map(),
      diagnostics: [],
      listening: false,
      sequence: 0,
    };
  root.__codexPlusNativePredictionsState = state;

  const runtime = {
    update(settings) {
      state.settings = settings && typeof settings === "object" ? {
        enhancementsEnabled: settings.enhancementsEnabled === true,
        codexAppNativePredictions: settings.codexAppNativePredictions === true,
        nativePredictionsLocalApiConfigured: settings.nativePredictionsLocalApiConfigured === true,
      } : null;
      const status = runtime.status();
      if (!status.enabled) cancelAll("disabled");
      return status;
    },
    status() {
      const settings = state.settings;
      const enhancementsEnabled = settings?.enhancementsEnabled === true;
      const optedIn = settings?.codexAppNativePredictions === true;
      return {
        enabled: enhancementsEnabled && optedIn,
        enhancementsEnabled,
        optedIn,
        localApiConfigured: settings?.nativePredictionsLocalApiConfigured === true,
        unsupported: state.unsupported === true,
      };
    },
    // 只有本机执行端 + 后端确认当前生效配置是自定义 API 时才放宽；远端主机一律不放宽。
    relax(hostId) {
      const status = runtime.status();
      return hostId === "local" && status.enabled && status.localApiConfigured && !status.unsupported;
    },
  };

  function diagnostic(event, detail = {}) {
    // 只记录事件与原因，不记录对话内容、预测文本或请求参数。
    const entry = { event, ...detail, at: Date.now() };
    state.diagnostics.push(entry);
    if (state.diagnostics.length > 50) state.diagnostics.splice(0, state.diagnostics.length - 50);
    try {
      const bridge = root.__codexSessionDeleteBridge;
      if (typeof bridge === "function") {
        Promise.resolve(bridge("/diagnostics/log", { event: `native_predictions.${event}`, detail })).catch(() => {});
      }
    } catch { /* 诊断失败不影响预测。 */ }
  }

  // ---------- 模块改写 ----------

  function iife(name, body, argument) {
    // 参数名带前缀，避免与压缩后的外层绑定重名；外层表达式只求值一次。
    return `((${name})=>${body})(${argument})`;
  }

  function gateText(gate, host, kind) {
    return `(${iife("__codexPlusGate", `${handle}?.gate?.(__codexPlusGate,${host},\`${kind}\`)??__codexPlusGate`, gate)})`;
  }

  // 每个目标块只认一处完整结构；任何一项缺失、重复或绑定冲突都整体放弃该块。
  const structures = {
    "app-initial": [
      {
        kind: "availability",
        pattern: [
          `${id}=${id}\\(${id},\\(\\{get:(?<get>${id})\\}\\)=>`,
          `(?<gate>\\k<get>\\(${id},${featureGate}\\))&&`,
          `(?<preference>${id}\\(\\k<get>,${id}\\(\\)\\.enabled\\))&&`,
          `${id}\\.cumberbatch!=null\\)`,
        ].join(""),
        edits: [
          { group: "gate", text: (g) => gateText(g.gate, "`local`", "availability") },
          {
            group: "preference",
            text: (g) => `(${iife("__codexPlusPreference", `(${handle}?.observePreference?.(__codexPlusPreference),__codexPlusPreference)`, g.preference)})`,
          },
        ],
      },
    ],
    "composer-content": [
      {
        kind: "catch-up",
        pattern: [
          `(?<service>${id})=${id}\\.cumberbatch,${id};`,
          `[\\s\\S]{0,160}?\\[\`cumberbatch\`,${id}\\]`,
          `[\\s\\S]{0,240}?\\{let\\{signal:(?<signal>${id})\\}=${id},(?<target>${id})=\\{\\.\\.\\.${id},generationTrigger:\`catch_up\`\\};`,
          `[\\s\\S]{0,320}?let (?<callback>${id})=new ${id}\\(${id}=>\\{${id}\\(\\{scope:${id},target:\\k<target>,ineligibility:${id}\\}\\)\\}\\);`,
          `return ${id}\\(${id}\\((?<call>\\k<service>\\.requestUserMessagePrediction\\(\\k<target>,\\k<callback>\\))\\),\\k<signal>\\)`,
        ].join(""),
        distinct: ["service", "signal", "target", "callback"],
        edits: [
          {
            group: "call",
            text: (g) => `(${handle}?.request?.(${g.service},${g.target},${g.callback},${g.signal},null)??${g.call})`,
          },
        ],
      },
    ],
    "initialize-app-host-services": [
      {
        kind: "live",
        pattern: [
          `async function ${id}\\((?<scope>${id}),(?<client>${id}),(?<event>${id}),(?<current>${id})\\)\\{`,
          `let (?<service>${id})=${id}\\?\\.cumberbatch;`,
          `if\\(\\k<service>!=null&&!${id}\\(\\k<event>\\.hostId\\)&&\\k<current>\\(\\)&&(?<gate>(?<check>${id})\\(\\k<scope>,${featureGate}\\))\\)try\\{`,
          `[\\s\\S]{0,200}?getCachedConversationMetadata\\(\\k<event>\\.conversationId\\)[\\s\\S]{0,12}?`,
          `if\\(!\\k<current>\\(\\)\\|\\|!(?<recheck>\\k<check>\\(\\k<scope>,${featureGate}\\))\\|\\|`,
          `[\\s\\S]{0,500}?let (?<target>${id})=\\{hostId:\\k<event>\\.hostId,threadId:\\k<event>\\.conversationId,latestTurnId:\\k<event>\\.turnId,generationTrigger:\`live\`\\},(?<callback>${id})=`,
          `[\\s\\S]{0,240}?(?<call>\\k<service>\\.requestUserMessagePrediction\\(\\k<target>,\\k<callback>\\))`,
        ].join(""),
        distinct: ["scope", "client", "event", "current", "service", "check", "target", "callback"],
        edits: [
          { group: "gate", text: (g) => gateText(g.gate, `${g.event}.hostId`, "live") },
          { group: "recheck", text: (g) => gateText(g.recheck, `${g.event}.hostId`, "live") },
          {
            group: "call",
            text: (g) => `(${handle}?.request?.(${g.service},${g.target},${g.callback},null,${g.current})??${g.call})`,
          },
        ],
      },
    ],
    "general-settings": [
      {
        kind: "settings-row",
        pattern: [
          `(?<flag>${id})=(?<gate>${id}\\(${featureGate}\\)),(?<service>${id})=${id}\\.cumberbatch,`,
          `[\\s\\S]{0,240}?\\[\`cumberbatch-availability\`,`,
          `[\\s\\S]{0,360}?\\k<flag>&&\\k<service>!=null\\?async ${id}=>\\k<service>!=null&&(?<remote>\\k<service>\\.isEnabledByRemoteConfig\\(\\)):`,
          `[\\s\\S]{0,600}?if\\(!\\k<flag>\\|\\|`,
          `[\\s\\S]{0,900}?\`settings\\.general\\.messagePredictions\\.label\``,
          `[\\s\\S]{0,1600}?onChange:(?<value>${id})=>\\{${id}\\(${id},${id},\\k<value>\\),${id}\\(\\{scope:${id},globallyEnabled:\\k<value>\\}\\)(?<notify>)\\}`,
        ].join(""),
        distinct: ["flag", "service", "value"],
        edits: [
          { group: "gate", text: (g) => gateText(g.gate, "`local`", "settings") },
          { group: "remote", text: (g) => `(${handle}?.remoteConfig?.(${g.service})??${g.remote})` },
          { group: "notify", text: (g) => `,${handle}?.observePreference?.(${g.value})` },
        ],
      },
    ],
  };

  const targets = [
    { name: "app-initial", prefix: "app-initial-", unique: true },
    { name: "composer-content", prefix: "composer-content-", via: ["app-initial-"] },
    { name: "initialize-app-host-services", prefix: "initialize-app-host-services-", via: ["app-main-"] },
    { name: "general-settings", prefix: "general-settings-", via: ["app-main-", "settings-lazy-routes-"] },
  ];

  function plan(source, url, target) {
    const diagnostics = { url, target, reason: "ready", matched: 0, matches: {} };
    const failed = (reason) => ({ ok: false, replacements: [], diagnostics: { ...diagnostics, reason } });
    const definitions = structures[target];
    if (!definitions) return failed("unknown-target");
    if (typeof source !== "string" || !source || typeof url !== "string" || !url.trim()) return failed("invalid-input");
    const replacements = [];
    for (const definition of definitions) {
      const matches = [...source.matchAll(new RegExp(definition.pattern, "gd"))];
      diagnostics.matches[definition.kind] = matches.length;
      if (matches.length === 0) continue;
      diagnostics.matched++;
      if (matches.length > 1) return failed("ambiguous-structure");
      const [match] = matches;
      const names = (definition.distinct ?? []).map((name) => match.groups[name]);
      if (new Set(names).size !== names.length) return failed("shadowed-bindings");
      for (const edit of definition.edits) {
        const [start, end] = match.indices.groups[edit.group] ?? [];
        if (!Number.isInteger(start) || !Number.isInteger(end)) return failed("unsupported-shape");
        const text = edit.text(match.groups);
        if (/[\r\n]/.test(text) || /[\r\n]/.test(source.slice(start, end))) return failed("unsupported-shape");
        replacements.push({ kind: `${definition.kind}:${edit.group}`, start, end, text });
      }
    }
    if (diagnostics.matched !== definitions.length) return failed(diagnostics.matched === 0 ? "incomplete-structure" : "partial-structure");
    replacements.sort((left, right) => left.start - right.start || left.end - right.end);
    if (replacements.some((item, index) => index > 0 && item.start < replacements[index - 1].end)) return failed("overlapping-structure");
    return { ok: true, replacements, diagnostics };
  }

  function transform(source, url, target) {
    const result = plan(source, url, target);
    let patchedSource = source;
    if (result.ok) {
      for (const replacement of [...result.replacements].reverse()) {
        patchedSource = patchedSource.slice(0, replacement.start) + replacement.text + patchedSource.slice(replacement.end);
      }
    }
    return { ...result, patchedSource };
  }

  const rewriter = {
    id: "predictions",
    targets,
    enabled: () => runtime.status().enabled,
    plan,
  };

  // ---------- 原生预测路径的运行时 ----------

  // 统一返回布尔值：原生门控已通过时原样保留；否则只为本机自定义 API 放宽。
  function gate(official, hostId, kind) {
    const value = official === true;
    state.officialGate = value;
    if (typeof kind === "string") state.marks[kind] = true;
    if (value) return true;
    // 后台完成的回合只在可用性门控也已打补丁时采样，避免预测生成了却无处显示。
    if (kind === "live" && state.marks.availability !== true) return false;
    return runtime.relax(hostId);
  }

  function observePreference(value) {
    if (typeof value === "boolean") state.preference = value;
    return value;
  }

  // 设置页的可见性：官方 remote config 只对 ChatGPT 登录有效，自定义 API 由本功能声明可用。
  function remoteConfig(service) {
    if (!runtime.relax("local")) return undefined;
    void service;
    return Promise.resolve(true);
  }

  function customProvider(provider) {
    return typeof provider === "string" && provider.trim() !== "" && !officialProviders.has(provider.trim().toLowerCase());
  }

  function bridge() {
    const value = root.electronBridge;
    return value && typeof value.sendMessageFromView === "function" ? value : null;
  }

  function nextId(kind) {
    state.sequence += 1;
    const random = typeof root.crypto?.randomUUID === "function"
      ? root.crypto.randomUUID()
      : `${Date.now().toString(36)}-${state.sequence}-${Math.random().toString(36).slice(2)}`;
    // thread/sample: 前缀与原生内部请求一致：被放弃后主进程收到的迟到回包会被静默丢弃。
    return `${kind}:codex-plus-${random}`;
  }

  function send(message) {
    const target = bridge();
    if (!target) return false;
    try {
      Promise.resolve(target.sendMessageFromView(message)).catch(() => {});
      return true;
    } catch {
      return false;
    }
  }

  function ensureListener() {
    if (state.listening || typeof root.addEventListener !== "function") return;
    state.listening = true;
    root.addEventListener("message", (event) => {
      const data = event?.data;
      if (!data || typeof data !== "object") return;
      if (data.type === "mcp-response" && (data.hostId ?? "local") === "local") {
        const key = String(data.message?.id ?? "");
        const pending = state.pending.get(key);
        if (!pending) return;
        state.pending.delete(key);
        pending(data.message);
        return;
      }
      if (data.type === "mcp-notification" && (data.hostId ?? "local") === "local" && data.method === "turn/started") {
        const threadId = data.params?.threadId;
        if (typeof threadId !== "string") return;
        // 源对话开了新回合：采样已不可能对应最新回合，立即取消。
        for (const entry of [...state.samples.values()]) {
          if (entry.threadId === threadId && !entry.done) cancelEntry(entry, "source-turn-started");
        }
      }
    });
  }

  function request(method, params, { timeoutMs = 10_000 } = {}) {
    ensureListener();
    const requestId = nextId(method);
    return new Promise((resolve) => {
      let timer = null;
      const finish = (message) => {
        if (timer != null) root.clearTimeout?.(timer);
        state.pending.delete(requestId);
        resolve(message ?? null);
      };
      state.pending.set(requestId, finish);
      if (!send({ type: "mcp-request", hostId: "local", request: { id: requestId, method, params }, coalesce: false })) {
        finish(null);
        return;
      }
      timer = root.setTimeout?.(() => finish(null), timeoutMs) ?? null;
    });
  }

  async function threadProvider(threadId) {
    if (typeof threadId !== "string" || !threadId) return null;
    const cached = state.providers.get(threadId);
    if (cached && Date.now() - cached.at < providerTtlMs) return cached.value;
    const message = await request("thread/read", { threadId, includeTurns: false });
    const thread = message?.result?.thread;
    if (!thread || typeof thread !== "object") return null;
    const value = {
      provider: typeof thread.modelProvider === "string" ? thread.modelProvider : null,
      model: typeof thread.model === "string" && thread.model ? thread.model : null,
    };
    state.providers.set(threadId, { at: Date.now(), value });
    if (state.providers.size > 64) state.providers.delete(state.providers.keys().next().value);
    return value;
  }

  const responseSchema = JSON.stringify({
    type: "object",
    properties: { suggestion: { type: ["string", "null"] } },
    required: ["suggestion"],
    additionalProperties: false,
  });

  // 自行编写的提示词：只要求模型给出用户下一句可能输入的话，不调用工具、不回答问题。
  const prompt = [
    "Predict the next message the user is most likely to type into the composer of this conversation.",
    "Base it on the whole conversation, especially the assistant's latest reply.",
    "Write it in the user's own voice and in the same language the user has been using.",
    "Keep it to one short, concrete sentence the user could send as-is.",
    "Do not answer, explain, greet, or speak as the assistant; do not repeat the assistant's text; do not call tools.",
    "If there is no confident prediction, use null.",
    "",
    `Return only JSON matching this schema: ${responseSchema}`,
  ].join("\n");

  function parseSuggestion(text) {
    if (typeof text !== "string") return { ok: false };
    let body = text.trim();
    const fenced = /^```(?:json)?\s*([\s\S]*?)\s*```$/i.exec(body);
    if (fenced) body = fenced[1];
    let value;
    try {
      value = JSON.parse(body);
    } catch {
      const start = body.indexOf("{");
      const end = body.lastIndexOf("}");
      if (start < 0 || end <= start) return { ok: false };
      try { value = JSON.parse(body.slice(start, end + 1)); } catch { return { ok: false }; }
    }
    if (!value || typeof value !== "object" || !("suggestion" in value)) return { ok: false };
    if (value.suggestion == null) return { ok: true, suggestion: null };
    if (typeof value.suggestion !== "string") return { ok: false };
    const suggestion = value.suggestion.trim();
    if (!suggestion) return { ok: true, suggestion: null };
    return { ok: true, suggestion: suggestion.length > maxSuggestionLength ? suggestion.slice(0, maxSuggestionLength).trimEnd() : suggestion };
  }

  function numberOrUndefined(value) {
    const number = typeof value === "string" ? Number(value) : value;
    return Number.isFinite(number) ? number : undefined;
  }

  function unsupportedError(error) {
    const text = String(error?.message ?? "");
    return error?.code === -32601 || /method not found|unknown variant|unknown method/i.test(text);
  }

  function settle(entry, value) {
    if (entry.done) return;
    entry.done = true;
    if (entry.timer != null) root.clearTimeout?.(entry.timer);
    entry.timer = null;
    state.pending.delete(entry.requestId);
    entry.resolve(value);
  }

  function cancelEntry(entry, reason) {
    if (entry.done) return;
    // 先结束本地请求（迟到回包从此被丢弃），再清理主进程里仍在排队的请求，
    // 最后用原始请求 ID 调用 thread/sample/cancel 停掉已经开始的采样。
    settle(entry, null);
    send({ type: "mcp-request-abandon", hostId: "local", requestId: entry.requestId });
    const cancelId = nextId("thread/sample/cancel");
    let timer = null;
    state.pending.set(cancelId, (message) => {
      if (timer != null) root.clearTimeout?.(timer);
      diagnostic("sample_cancel_acknowledged", { reason, error: message?.error?.code ?? null });
    });
    timer = root.setTimeout?.(() => state.pending.delete(cancelId), 10_000) ?? null;
    send({
      type: "mcp-request",
      hostId: "local",
      request: { id: cancelId, method: "thread/sample/cancel", params: { requestId: entry.requestId } },
      coalesce: false,
    });
    // 超时结果保留在缓存里，避免同一回合反复消耗额度；其它取消原因允许之后重新请求。
    if (reason !== "deadline" && state.samples.get(entry.key) === entry) state.samples.delete(entry.key);
    diagnostic("sample_canceled", { reason });
  }

  function cancelAll(reason) {
    for (const entry of [...(state.samples?.values?.() ?? [])]) {
      if (!entry.done) cancelEntry(entry, reason);
    }
  }

  function startSample(target, details) {
    ensureListener();
    const requestId = nextId("thread/sample");
    const entry = {
      key: `${target.threadId}\u0000${target.latestTurnId}`,
      threadId: target.threadId,
      turnId: target.latestTurnId,
      requestId,
      done: false,
      waiters: 0,
      timer: null,
      resolve: null,
      promise: null,
    };
    entry.promise = new Promise((resolve) => { entry.resolve = resolve; });
    const requestedAt = new Date().toISOString();
    const started = Date.now();
    state.pending.set(requestId, (message) => {
      if (entry.done) return;
      if (message?.error) {
        if (unsupportedError(message.error)) {
          state.unsupported = true;
          diagnostic("sample_unsupported", { code: message.error.code ?? null });
        } else {
          diagnostic("sample_failed", { code: message.error.code ?? null });
        }
        settle(entry, null);
        return;
      }
      const result = message?.result;
      const parsed = parseSuggestion(result?.text);
      if (!parsed.ok) {
        diagnostic("sample_unparsable", {});
        settle(entry, null);
        return;
      }
      settle(entry, {
        requestedAt,
        hostKind: "local",
        text: parsed.suggestion,
        responseId: typeof result.responseId === "string" && result.responseId ? result.responseId : requestId,
        generationTrigger: target.generationTrigger,
        sourceThreadId: target.threadId,
        sourceTurnId: target.latestTurnId,
        model: details?.model ?? null,
        reasoningEffort: null,
        serviceTier: null,
        parentReasoningEffort: null,
        configuredReasoningEffort: null,
        promptVersion,
        totalDurationMs: Date.now() - started,
        inputTokens: numberOrUndefined(result.inputTokens),
        cachedInputTokens: numberOrUndefined(result.cachedInputTokens),
      });
      diagnostic("sample_completed", { suggested: parsed.suggestion != null, durationMs: Date.now() - started });
    });
    // 采样继承已加载源对话的模型、供应商和推理强度；不传 effort，不创建任何临时对话或回合。
    const sent = send({
      type: "mcp-request",
      hostId: "local",
      request: {
        id: requestId,
        method: "thread/sample",
        params: {
          threadId: target.threadId,
          expectedTurnId: target.latestTurnId,
          prompt,
          turnTrigger: "composer_predictions",
          disableMcp: true,
        },
      },
      coalesce: false,
    });
    if (!sent) {
      diagnostic("sample_transport_unavailable", {});
      settle(entry, null);
      return entry;
    }
    entry.timer = root.setTimeout?.(() => cancelEntry(entry, "deadline"), sampleDeadlineMs) ?? null;
    diagnostic("sample_started", { trigger: target.generationTrigger ?? null });
    return entry;
  }

  function attach(entry, signal) {
    entry.waiters += 1;
    if (!signal || typeof signal.addEventListener !== "function") return entry.promise;
    let detached = false;
    const detach = () => {
      if (detached) return;
      detached = true;
      entry.waiters -= 1;
      // 所有等待方都放弃时才取消；后台（live）请求没有 signal，会一直持有到完成。
      if (entry.waiters <= 0 && !entry.done) cancelEntry(entry, "abandoned");
    };
    if (signal.aborted) {
      detach();
      return entry.promise;
    }
    signal.addEventListener("abort", detach, { once: true });
    entry.promise.finally(() => signal.removeEventListener?.("abort", detach));
    return entry.promise;
  }

  function sample(target, signal, details) {
    const key = `${target.threadId}\u0000${target.latestTurnId}`;
    let entry = state.samples.get(key);
    if (!entry) {
      // 同一对话只保留最新回合；旧回合仍在进行的采样直接取消。
      for (const [otherKey, other] of [...state.samples]) {
        if (other.threadId !== target.threadId) continue;
        if (!other.done) cancelEntry(other, "superseded");
        state.samples.delete(otherKey);
      }
      entry = startSample(target, details);
      state.samples.set(key, entry);
      while (state.samples.size > maxCachedSamples) {
        const [oldestKey, oldest] = state.samples.entries().next().value;
        if (!oldest.done) cancelEntry(oldest, "evicted");
        state.samples.delete(oldestKey);
      }
    }
    return attach(entry, signal);
  }

  async function delegate(service, target, callback) {
    // 原生控制器的结果是可释放的 RPC 句柄：等待完成后释放，和原调用方的处理保持一致。
    const pending = service.requestUserMessagePrediction(target, callback);
    try {
      return await pending;
    } finally {
      try { pending?.[Symbol.dispose]?.(); } catch { /* 句柄已释放。 */ }
    }
  }

  // 返回 undefined 表示交还原调用；返回 Promise 表示由本功能接管。
  function requestPrediction(service, target, callback, signal, isCurrent) {
    const status = runtime.status();
    if (!status.enabled) return undefined;
    if (!target || typeof target !== "object") return undefined;
    const officialAllowed = state.officialGate === true && typeof service?.requestUserMessagePrediction === "function";
    // 远端 / SSH / 云端对话：原生门控通过时原样走官方，否则保持“无预测”的原状。
    if (target.hostId !== "local" || !status.localApiConfigured || status.unsupported) {
      return officialAllowed ? undefined : Promise.resolve(null);
    }
    return (async () => {
      const details = await threadProvider(target.threadId);
      if (!customProvider(details?.provider)) {
        return officialAllowed ? delegate(service, target, callback) : null;
      }
      if (state.preference !== true) {
        diagnostic("sample_skipped", { reason: "preference" });
        return null;
      }
      if (typeof isCurrent === "function" && !isCurrent()) return null;
      if (typeof target.threadId !== "string" || typeof target.latestTurnId !== "string" || !target.latestTurnId) return null;
      return sample(target, signal, details);
    })();
  }

  const api = {
    plan,
    transform,
    rewriter,
    runtime,
    gate,
    observePreference,
    remoteConfig,
    request: requestPrediction,
    prompt,
    parseSuggestion,
    deadlineMs: sampleDeadlineMs,
    state,
  };
  if (typeof module === "object" && module.exports) module.exports = api;
  if (typeof window === "object") {
    window.__codexPlusNativePredictions = api;
    window.__codexPlusNativeFastUnlock?.registerRewriter?.(rewriter);
  }
})();
