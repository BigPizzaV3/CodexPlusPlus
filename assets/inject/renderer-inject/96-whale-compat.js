  // 完整上游引擎的词法宿主：不替换页面全局 API；挂件关闭时释放所有副作用。
  const codexPlusWhaleFullStoragePrefix = "codexPlus.whale.full.v1.";
  function codexPlusWhaleFullNumber(value) {
    if (value === null || value === undefined || value === "") return null;
    const n = Number(value); return Number.isFinite(n) && n >= 0 ? n : null;
  }
  function codexPlusWhaleFullBuiltin(url) {
    let parsed;
    try { parsed = new URL(String(url), "https://codex-whale.invalid"); } catch { return ""; }
    const path = parsed.pathname, id = parsed.searchParams.get("id"), set = parsed.searchParams.get("set");
    const aliases = { "/dsh-whale/image.png": "DSniang1.png", "/dsh-whale/rua.gif": "rua.gif" };
    const fragments = { ya1: "Ya1.mp3", ya2: "Ya2.mp3", d1: "D1.mp3", d2: "D2.mp3", exp_orb: "minecraft-exp-orb.wav", end_a: "task-end-a.wav", taskEnd: "task-end-a.wav" };
    let file = aliases[path];
    if (path === "/dsh-whale/audio-fragment.wav") file = fragments[id];
    if (path === "/dsh-whale/bubble-img.png") file = ({ bimg_petpet: "bubble-petpet.gif", bimg_money1: "bubble-money1.gif" })[id];
    if (/^\/dsh-whale\/sound\/(press|release)\.mp3$/.test(path) && (!set || set === "duck" || set === "fx1")) file = fragments[(set === "fx1" ? "d" : "ya") + (path.includes("press") ? "1" : "2")];
    if (path === "/dsh-whale/role-image.png" && id === "default") file = "DSniang1.png";
    return file ? window.__CODEX_PLUS_WHALE_ASSETS__?.[file] || (file === "DSniang1.png" ? window.__CODEX_PLUS_WHALE_IMAGE__ : "") || "" : "";
  }

  function codexPlusWhaleFullDataResponse(source) {
    // 直接读取本地内嵌字节，避免 fetch(data:) 受到宿主 connect-src 限制。
    const match = /^data:((?:audio|image)\/[A-Za-z0-9.+-]+);base64,([A-Za-z0-9+/]*={0,2})$/.exec(source);
    if (!match) throw new Error("不支持的本地媒体数据");
    const bytes = Uint8Array.from(atob(match[2]), (character) => character.charCodeAt(0));
    return new Response(bytes, { status: 200, headers: { "Content-Type": match[1] } });
  }

  function createCodexPlusWhaleFullRuntime() {
    const state = { disposed: false, paused: document.hidden === true, timers: new Map(), rafs: new Map(), observers: new Set(), nodes: new Set(), listeners: [], audio: new Set(), pending: new Set(), media: new Map(), objectUrls: new Set(), subscriptions: [], dispatcher: null, sessionId: "", session: null, sessionAt: 0, sessionPending: null, history: null, historyAt: 0, historyPrices: "", historyPending: null, models: [], relayProfiles: [], turn: null, completed: new Set(), seq: 0, pendingWait: null, completion: null, controls: null, serial: 0 };
    const isolated = Object.create(null);
    const wrappedMedia = new WeakSet();
    const storage = {
      getItem(key) { return localStorage.getItem(codexPlusWhaleFullStoragePrefix + key); },
      setItem(key, value) { return localStorage.setItem(codexPlusWhaleFullStoragePrefix + key, String(value)); },
      removeItem(key) { return localStorage.removeItem(codexPlusWhaleFullStoragePrefix + key); },
    };
    try { state.seq = Math.max(0, Number(storage.getItem("dshw-last-seq")) || 0); } catch {}
    state.initialSeq = state.seq;
    state.priceRevision = 0;
    state.statRevision = 0;
    const active = () => !state.disposed;
    function listen(target, name, callback, options) {
      const wrapped = typeof callback === "function" ? function (event) { if (active()) callback.call(target, event); } : { handleEvent(event) { if (active()) callback?.handleEvent?.(event); } };
      target.addEventListener(name, wrapped, options);
      state.listeners.push({ target, name, callback, wrapped, options });
    }
    function unlisten(target, name, callback, options) {
      for (let i = state.listeners.length - 1; i >= 0; i--) {
        const entry = state.listeners[i];
        if (entry.target === target && entry.name === name && entry.callback === callback) {
          target.removeEventListener(name, entry.wrapped, options ?? entry.options); state.listeners.splice(i, 1);
        }
      }
    }
    function arm(id, timer) {
      if (!active() || state.paused) return;
      timer.native = setTimeout(() => {
        if (!state.timers.has(id) || !active() || state.paused) return;
        timer.native = null;
        if (!timer.repeat) state.timers.delete(id);
        try { timer.callback(...timer.args); } finally { if (timer.repeat && state.timers.has(id)) arm(id, timer); }
      }, timer.delay);
    }
    function schedule(callback, delay, repeat, args) {
      if (!active() || typeof callback !== "function") return 0;
      const id = ++state.serial, timer = { callback, delay: Math.max(0, Number(delay) || 0), repeat, args, native: null };
      state.timers.set(id, timer); arm(id, timer); return id;
    }
    function clearTimer(id) { const timer = state.timers.get(id); if (timer?.native !== null) clearTimeout(timer?.native); state.timers.delete(id); }
    function raf(callback) {
      if (!active()) return 0;
      const id = ++state.serial, frame = { callback, native: null };
      state.rafs.set(id, frame);
      if (!state.paused) frame.native = requestAnimationFrame((time) => { state.rafs.delete(id); if (active() && !state.paused) callback(time); });
      return id;
    }
    function cancelRaf(id) { const frame = state.rafs.get(id); if (frame?.native !== null) cancelAnimationFrame(frame?.native); state.rafs.delete(id); }
    function trackedObserver(Native) {
      if (typeof Native !== "function") return undefined;
      return class {
        constructor(callback) { this.records = []; this.native = new Native((entries, observer) => { if (active() && !state.paused) callback(entries, this); }); state.observers.add(this); }
        observe(target, options) { this.records.push([target, options]); if (active() && !state.paused) this.native.observe(target, options); }
        disconnect() { this.native.disconnect(); this.records = []; }
        takeRecords() { return this.native.takeRecords?.() || []; }
      };
    }
    function trackedAudioContext(options) {
      const Native = window.AudioContext || window.webkitAudioContext;
      if (!Native || !active()) throw new Error("音频不可用");
      const audio = new Native(options); state.audio.add(audio); return audio;
    }
    trackedAudioContext.prototype = (window.AudioContext || window.webkitAudioContext)?.prototype || Object.prototype;
    function bridge(path, payload) {
      if (!active() || state.paused) return Promise.reject(new Error("挂件已暂停"));
      return new Promise((resolve, reject) => {
        let done = false;
        const finish = (error, value) => { if (done) return; done = true; clearTimeout(timer); state.pending.delete(cancel); error ? reject(error) : resolve(value); };
        const cancel = () => finish(new Error("挂件请求已取消"));
        const timer = setTimeout(() => finish(new Error("挂件请求超时")), 45000);
        state.pending.add(cancel);
        Promise.resolve(postJson(path, payload)).then((value) => finish(active() ? null : new Error("挂件已关闭"), value), (error) => finish(error));
      });
    }
    function context() {
      const ref = currentSessionRef() || {}, id = String(ref.session_id || ""), host = String(ref.host_id || "local");
      if (id !== state.sessionId || host !== state.sessionHost) {
        state.sessionId = id; state.session = null; state.sessionAt = 0; state.sessionPending = null; state.turn = null; state.pendingWait = null; state.lastTurnResult = null; state.lastTurnPending = null;
      }
      state.sessionHost = host; state.sessionRef = ref; state.remote = host !== "local";
      return id;
    }
    function observe(turn, baseline = false) {
      if (!turn?.id) return;
      const previous = state.turn;
      if (!baseline && previous?.id === turn.id && previous.status === "running" && turn.status === "completed" && !state.completed.has(turn.id)) {
        state.seq += 1; state.completed.add(turn.id);
        if (state.completed.size > 128) state.completed.delete(state.completed.values().next().value);
      }
      if (baseline && turn.status === "completed") state.completed.add(turn.id);
      state.turn = { id: turn.id, status: turn.status, usage: turn.usage || null };
    }
    function setStatsEnabled(value) {
      const enabled = value !== false;
      if (state.codexStatsOn === enabled) return;
      state.codexStatsOn = enabled; state.statRevision += 1;
      state.history = null; state.historyAt = 0; state.historyPending = null; state.session = null; state.sessionAt = 0; state.sessionPending = null; state.lastTurnResult = null; state.lastTurnPending = null;
    }
    async function session() {
      const id = context();
      if (!id) return null;
      if (state.codexStatsOn === false || state.remote) return { status: "ok", sessionId: id, title: state.sessionRef?.title || "", lastTurn: state.turn, total: state.nativeUsage?.total || null, today: null, rateLimits: [] };
      if (state.session && Date.now() - state.sessionAt < 5000) return state.session;
      if (state.sessionPending) return state.sessionPending;
      const baseline = !state.session && !state.turn;
      const statRevision = state.statRevision;
      const pending = bridge("/whale/session", { ...state.sessionRef, session_id: id }).then((data) => {
        if (!active() || context() !== id || statRevision !== state.statRevision || state.codexStatsOn === false || state.remote) return null;
        state.session = data; state.sessionAt = Date.now();
        if (data?.status === "ok" && !(Date.now() - (state.nativeTurnAt || 0) < 10000 && state.turn?.id === data.lastTurn?.id && state.turn.status === "completed" && data.lastTurn.status === "running")) observe(data.lastTurn, baseline);
        return data;
      }).finally(() => { if (state.sessionPending === pending) state.sessionPending = null; });
      state.sessionPending = pending; return pending;
    }
    function prices() {
      const result = [];
      for (const model of state.models) {
        const price = model.price;
        if (!price || ![price.hit, price.miss, price.out].every((value) => value !== "" && codexPlusWhaleFullNumber(value) !== null)) continue;
        const matchIds = model.matchIds?.length ? model.matchIds : [model.id];
        const currency = String(price.cur || "USD").toUpperCase(), rate = currency === "CNY" ? 1 : codexPlusWhaleFullNumber(price.rate);
        if (!(rate > 0) || !["CNY", "USD"].includes(currency)) continue;
        // 匹配交给数据层：精确匹配优先，其次最长的大小写不敏感子串。
        for (const id of matchIds) result.push({ model: id, currency: "CNY", input: Number(price.miss) * rate, cachedInput: Number(price.hit) * rate, output: Number(price.out) * rate });
      }
      return result;
    }
    function pricingFingerprint() { return JSON.stringify(state.models.map((model) => ({ id: model.id, price: model.price, matchIds: model.matchIds }))); }
    async function history(force = false) {
      if (state.codexStatsOn === false) return { status: "disabled", complete: false, codex: { ok: false, disabled: true } };
      const quote = prices(), fingerprint = JSON.stringify(quote);
      if (!force && state.history && state.historyPrices === fingerprint && Date.now() - state.historyAt < 60000) return state.history;
      if (state.historyPending) return state.historyPending;
      const revision = state.priceRevision, statRevision = state.statRevision;
      const pending = bridge("/whale/history", { period: "all", page: 1, pageSize: 500, prices: quote }).then((data) => {
        if (statRevision !== state.statRevision || state.codexStatsOn === false) return { status: "disabled", complete: false, codex: { ok: false, disabled: true } };
        if (active() && revision !== state.priceRevision) return history(true);
        if (active()) { state.history = data; state.historyAt = Date.now(); state.historyPrices = fingerprint; } return data;
      }).finally(() => { if (state.historyPending === pending) state.historyPending = null; });
      state.historyPending = pending; return pending;
    }
    const toTokens = (period) => {
      if (state.codexStatsOn === false) return "—";
      const value = state.history?.periods?.[period]?.usage?.totalTokens ?? state.history?.machineSummary?.[({ today: "todayTokens", month: "monthTokens", all: "totalTokens" })[period]];
      const number = codexPlusWhaleFullNumber(value);
      if (number === null) return "—";
      return number.toLocaleString("zh-CN", { maximumFractionDigits: 0 });
    };
    function codexModel(model, summary) {
      const data = summary?.codex || summary?.machineSummary || {};
      const total = (period) => codexPlusWhaleFullNumber(summary?.periods?.[period]?.usage?.totalTokens);
      const limits = state.session?.rateLimits || summary?.rateLimits || [];
      const windows = limits.filter((entry) => entry.resetAt == null || Number(entry.resetAt) * 1000 > Date.now()).map((entry, index) => ({ usedPct: entry.usedPercent, windowMinutes: entry.windowMinutes || (index === 0 ? 300 : 10080), resetAt: entry.resetAt == null ? null : Number(entry.resetAt) * 1000 }));
      return { ...model, codex: { ...data, ok: summary?.status === "ok" || summary?.status === "partial" || data.ok === true, complete: summary?.complete === true, deferred: summary?.complete === false ? Math.max(1, summary?.scan?.deferred || 1) : 0, todayTokens: total("today") ?? data.todayTokens, monthTokens: total("month") ?? data.monthTokens, totalTokens: total("all") ?? data.totalTokens, sessions: summary?.scan?.sessions ?? data.sessions ?? 0, days7: data.days7 || (summary?.days || []).slice(-7).map((day) => ({ date: day.date, tokens: day.usage?.totalTokens })), windows: data.windows || { primary: windows[0] || null, secondary: windows[1] || null } }, planSupport: windows.length > 0, plan: windows.length ? { ok: true, usedPct: windows[0].usedPct, remainPct: windows[0].usedPct == null ? null : 100 - windows[0].usedPct, windows: windows.map((entry, index) => ({ id: index === 0 ? "rolling" : "weekly", key: index === 0 ? "rolling" : "weekly", label: index === 0 ? "5h" : "周", usedPct: entry.usedPct, remainPct: entry.usedPct == null ? null : 100 - entry.usedPct, resetAt: entry.resetAt })) } : model.plan };
    }
    function subscribe() {
      const dispatcher = window.__codexPlusRemoteSessionRecoveryDispatcher;
      if (state.dispatcher === dispatcher && state.onEvent) return;
      for (const unsubscribe of state.subscriptions.splice(0)) { try { unsubscribe(); } catch {} }
      state.dispatcher = dispatcher;
      const onEvent = (payload, method) => {
        if (!active() || state.paused) return;
        const data = payload?.params || payload || {}, turn = data.turn || data;
        const thread = String(data.threadId || data.thread_id || data.conversationId || "").replace(/^local:/, "");
        if (!thread || thread !== context().replace(/^local:/, "")) return;
        if (method === "thread/tokenUsage/updated") {
          state.nativeUsage = data.tokenUsage || null;
          if (state.turn && data.tokenUsage?.last) state.turn.usage = data.tokenUsage.last;
        } else if (["serverRequest/resolved", "item/tool/userInputAnswered", "item/approval/responded", "item/completed"].includes(method)) {
          const id = String(data.requestId || data.itemId || data.item?.id || data.id || "");
          if (state.pendingWait && (id === state.pendingWait.id || id === state.pendingWait.itemId)) { state.pendingWait = null; void state.controls?.pollWaitState?.(); }
        } else if (method === "turn/started" || method === "turn/completed") {
          const id = turn.id || data.turnId;
          observe({ id, status: method === "turn/started" ? "running" : ({ failed: "failed", interrupted: "aborted", aborted: "aborted" })[turn.status] || "completed", usage: turn.usage });
          state.nativeTurnAt = Date.now();
          state.pendingWait = null;
          state.sessionAt = 0;
          void state.controls?.pollLastTurn?.();
        } else {
          state.pendingWait = { id: String(payload?.id || data.requestId || data.itemId || data.id || ""), itemId: String(data.itemId || ""), kind: method.includes("requestUserInput") || method === "mcpServer/elicitation/request" ? "question" : "approval" };
          void state.controls?.pollWaitState?.();
        }
      };
      state.onEvent = onEvent;
      if (!dispatcher?.subscribe) return;
      for (const method of ["turn/started", "turn/completed", "thread/tokenUsage/updated", "item/tool/requestUserInput", "item/commandExecution/requestApproval", "item/fileChange/requestApproval", "item/permissions/requestApproval", "mcpServer/elicitation/request", "serverRequest/resolved", "item/tool/userInputAnswered", "item/approval/responded", "item/completed"]) {
        try { const remove = dispatcher.subscribe(method, (payload) => onEvent(payload, method)); if (typeof remove === "function") state.subscriptions.push(remove); } catch {}
      }
    }
    function nativeEnvelope(value, depth = 0) {
      if (!active() || state.paused || !value || typeof value !== "object" || depth > 4) return;
      if (typeof value.method === "string" && /^(turn\/(started|completed)|thread\/tokenUsage\/updated|item\/(tool\/requestUserInput|commandExecution\/requestApproval|fileChange\/requestApproval|permissions\/requestApproval|completed)|mcpServer\/elicitation\/request|serverRequest\/resolved)$/.test(value.method)) state.onEvent?.(value, value.method);
      if (value.id != null && (Object.hasOwn(value, "result") || Object.hasOwn(value, "error") || Object.hasOwn(value, "response")) && String(value.id) === state.pendingWait?.id) {
        state.pendingWait = null; void state.controls?.pollWaitState?.();
      }
      for (const key of ["message", "request", "payload", "data", "notification"]) if (value[key] && typeof value[key] === "object") nativeEnvelope(value[key], depth + 1);
    }
    async function lastTurn() {
      await session().catch(() => null);
      if (state.lastTurnResult?.body?.seq === state.seq) return state.lastTurnResult;
      if (state.lastTurnPending) return state.lastTurnPending;
      const seq = state.seq;
      const turn = state.turn?.status === "completed" ? state.turn : null;
      let amount = null, currency = null, usage = turn?.usage || state.session?.lastTurn?.usage || null;
      if (turn && seq > state.initialSeq && prices().length && state.codexStatsOn !== false && !state.remote) {
        const pending = (async () => {
        const details = await bridge("/whale/history", { period: "all", sessionId: context().replace(/^local:/, ""), turnId: turn.id, page: 1, pageSize: 500, prices: prices() }).catch(() => null);
        const totals = {};
        for (const record of details?.records || []) {
          if (record.turnId !== turn.id || !record.estimatedCost) continue;
          const cost = record.estimatedCost; totals[cost.currency] = (totals[cost.currency] || 0) + cost.amount;
        }
        const currencies = Object.keys(totals);
        if (currencies.length === 1) { currency = currencies[0]; amount = totals[currency]; }
        const result = { status: 200, body: { ok: true, seq, turn, amount, currency, usage, estimated: amount !== null } };
        if (active() && state.seq === seq) state.lastTurnResult = result;
        return result;
        })().finally(() => { if (state.lastTurnPending === pending) state.lastTurnPending = null; });
        state.lastTurnPending = pending; return pending;
      }
      const result = { status: 200, body: { ok: true, seq, turn, amount, currency, usage, estimated: amount !== null } };
      state.lastTurnResult = result; return result;
    }
    async function compatibilityJson(path, method, query, body) {
      subscribe();
      if (path === "/dsh-whale/size.json" && method === "GET" && state.startupConfig) return { status: 200, body: state.startupConfig };
      if (path === "/dsh-whale/last-turn.json") {
        return lastTurn();
      }
      if (path === "/dsh-whale/wait.json") {
        const data = await session().catch(() => null);
        return { status: 200, body: { ok: true, sessionName: data?.sessionName || data?.title || state.sessionRef?.title || "", pending: data?.pending || state.pendingWait || null } };
      }
      const response = await bridge("/whale/full", { path, method, query, body });
      if (path === "/dsh-whale/size.json" && method === "PUT" && response?.status === 200) {
        setStatsEnabled(body.codexStatsOn);
      }
      if (path === "/dsh-whale/api-models.json" && response?.body?.ok) {
        state.relayProfiles = response.body.relayProfiles || state.relayProfiles;
        setStatsEnabled(response.body.codexStatsOn);
        const previous = pricingFingerprint();
        state.models = response.body.models || state.models;
        if (pricingFingerprint() !== previous) { state.priceRevision += 1; state.history = null; state.historyAt = 0; state.historyPending = null; state.lastTurnResult = null; state.lastTurnPending = null; }
        if (method === "GET") {
          const statsOn = response.body.codexStatsOn !== false;
          const summary = statsOn ? await history().catch(() => null) : null;
          response.body.models = (response.body.models || []).map((model) => model.id === "codex" ? statsOn ? codexModel(model, summary) : { ...model, codex: { ok: false, disabled: true } } : model);
        }
        state.models = response.body.models || state.models;
      }
      if (path === "/dsh-whale/usage-records.json" && method === "GET") {
        state.recordsAccountBase = JSON.parse(JSON.stringify(response.body));
        const data = await history().catch(() => null);
        response.body = mergeRecords(response.body, data);
      }
      return response;
    }
    function mergeRecords(base, data) {
      if (!data?.today || !data?.days7 || !data?.all) return base;
      const merge = (account, local) => ({ ...local, ...account, modelTotal: local?.modelTotal, models: local?.models || [], estimatedCosts: local?.estimatedCosts, unpricedRecords: local?.unpricedRecords });
      const mergeDays = (account, local) => { const days = new Map((local || []).map((day) => [day.date, day])); for (const day of account || []) days.set(day.date, merge(day, days.get(day.date))); return [...days.values()].sort((a, b) => a.date.localeCompare(b.date)); };
      return { ...base, today: merge(base?.today, data.today), days7: mergeDays(base?.days7, data.days7), all: { ...base?.all, days: mergeDays(base?.all?.days, data.all.days), events: data.all.events || [] }, complete: data.complete, historyPagination: data.pagination, historyArchive: data.archive, ok: true };
    }
    const nativeFetch = window.fetch?.bind(window) || globalThis.fetch;
    function responseOf(result) {
      if (result?.data) {
        const bytes = Uint8Array.from(atob(result.data), (c) => c.charCodeAt(0));
        return new Response(bytes, { status: result.status || 200, headers: { "Content-Type": result.mimeType || "application/octet-stream" } });
      }
      return new Response(JSON.stringify(result?.body ?? null), { status: result?.status || 200, headers: { "Content-Type": "application/json" } });
    }
    async function fetchCompat(value, options = {}) {
      if (!active() || state.paused) throw new Error("挂件已暂停");
      const raw = String(value), builtin = codexPlusWhaleFullBuiltin(raw);
      if (builtin) return codexPlusWhaleFullDataResponse(builtin);
      const parsed = new URL(raw, "https://codex-whale.invalid");
      if (parsed.origin !== "https://codex-whale.invalid" || !parsed.pathname.startsWith("/dsh-whale/")) {
        // 上游只有本机资源 fetch；用户显式配置的超链接通过 window.open。
        if (raw.startsWith("data:")) return codexPlusWhaleFullDataResponse(raw);
        if (raw.startsWith("blob:")) return nativeFetch(raw, options);
        throw new Error("挂件资源只允许本机桥接");
      }
      const method = String(options.method || "GET").toUpperCase();
      const query = Object.fromEntries(parsed.searchParams.entries());
      let body = {};
      if (options.body) { try { body = JSON.parse(options.body); } catch { throw new Error("无效挂件请求"); } }
      if (options.signal?.aborted) throw new Error("挂件请求已取消");
      const result = await compatibilityJson(parsed.pathname, method, query, body);
      if (!active() || options.signal?.aborted) throw new Error("挂件请求已取消");
      if (result?.builtinFragmentId) {
        const uri = codexPlusWhaleFullBuiltin("/dsh-whale/audio-fragment.wav?id=" + encodeURIComponent(result.builtinFragmentId));
        if (!uri) throw new Error("内置音频缺失");
        return nativeFetch(uri);
      }
      return responseOf(result);
    }
    async function resolveMedia(path) {
      const builtin = codexPlusWhaleFullBuiltin(path); if (builtin) return builtin;
      if (state.media.has(path)) return state.media.get(path);
      const pending = (async () => {
      const result = await fetchCompat(path);
      if (!result.ok) throw new Error("素材加载失败");
      const blob = await result.blob();
      if (!active()) throw new Error("挂件已关闭");
      const objectUrl = URL.createObjectURL(blob); state.objectUrls.add(objectUrl); state.media.set(path, objectUrl); return objectUrl;
      })().catch((error) => { if (state.media.get(path) === pending) state.media.delete(path); throw error; });
      state.media.set(path, pending); return pending;
    }
    function mediaNode(node) {
      if (wrappedMedia.has(node)) return node;
      wrappedMedia.add(node);
      let proto = node, descriptor;
      while (proto && !descriptor) { descriptor = Object.getOwnPropertyDescriptor(proto, "src"); proto = Object.getPrototypeOf(proto); }
      let revision = 0;
      const set = (value) => { if (descriptor?.set) descriptor.set.call(node, value); else node.setAttribute("src", value); };
      try { Object.defineProperty(node, "src", { configurable: true, get() { return descriptor?.get ? descriptor.get.call(node) : node.getAttribute("src") || ""; }, set(value) {
        const rev = ++revision, raw = String(value || "");
        if (!raw.startsWith("/dsh-whale/")) { if (!raw || /^(data:image\/(png|jpeg|webp|gif|avif);|data:audio\/|blob:)/i.test(raw)) set(raw); return; }
        const builtin = codexPlusWhaleFullBuiltin(raw);
        if (builtin) { set(builtin); return; }
        void resolveMedia(raw).then((url) => { if (active() && rev === revision) set(url); }).catch(() => { if (active() && rev === revision) node.dispatchEvent?.(new Event("error")); });
      } }); } catch {}
      return node;
    }
    function mark(node) {
      if (!active()) throw new Error("挂件已关闭");
      if (!node) return node;
      node.setAttribute?.("data-codex-plus-ext", "whale-widget");
      if (["IMG", "AUDIO", "VIDEO", "SOURCE"].includes(node.tagName)) mediaNode(node);
      return node;
    }
    const head = new Proxy(document.head, { get(target, key) {
      if (["appendChild", "insertBefore", "append", "prepend", "replaceChildren"].includes(key)) return (...args) => { if (!active()) return args[0]; for (const node of args) if (node?.nodeType || node?.tagName) state.nodes.add(node); return target[key](...args); };
      const value = Reflect.get(target, key, target); return typeof value === "function" ? value.bind(target) : value;
    } });
    const doc = new Proxy(document, { get(target, key) {
      if (key === "head") return head;
      if (key === "createElement") return (...args) => mark(target.createElement(...args));
      if (key === "createElementNS") return (...args) => mark(target.createElementNS(...args));
      if (key === "addEventListener") return (name, callback, options) => listen(target, name, callback, options);
      if (key === "removeEventListener") return (name, callback, options) => unlisten(target, name, callback, options);
      const value = Reflect.get(target, key, target); return typeof value === "function" ? value.bind(target) : value;
    } });
    const win = new Proxy(window, { get(target, key) {
      if (key === "__codexPlusWhaleHost") return host;
      if (typeof key === "string" && (key.startsWith("__dsh") || key === "dshwRegisterMask")) return isolated[key];
      if (key === "document") return doc;
      if (key === "localStorage") return storage;
      if (key === "Image") return image;
      if (key === "Audio") return audio;
      if (key === "AudioContext" || key === "webkitAudioContext") return window.AudioContext || window.webkitAudioContext ? trackedAudioContext : undefined;
      if (key === "addEventListener") return (name, callback, options) => listen(target, name, callback, options);
      if (key === "removeEventListener") return (name, callback, options) => unlisten(target, name, callback, options);
      const value = Reflect.get(target, key, target); return typeof value === "function" ? value.bind(target) : value;
    }, set(target, key, value) { if (typeof key === "string" && (key.startsWith("__dsh") || key === "dshwRegisterMask")) { isolated[key] = value; return true; } return Reflect.set(target, key, value, target); } });
    function image(width, height) { return mark(new window.Image(width, height)); }
    image.prototype = window.Image?.prototype || Object.prototype;
    function audio(url) { const node = mark(new window.Audio()); if (url) node.src = url; return node; }
    audio.prototype = window.Audio?.prototype || Object.prototype;
    const host = {
      active, state, tokens: toTokens,
      attach(node) { if (active()) state.nodes.add(node); },
      detach(node) { state.nodes.delete(node); },
      initialSound() { return state.startupConfig?.sound !== false; },
      complete(data, show) { state.completion = data; show(data.amount); },
      completionCost(amount) {
        const completion = state.completion;
        if (amount == null) return completion?.usage?.totalTokens == null ? "用量暂不可用" : Number(completion.usage.totalTokens).toLocaleString("zh-CN") + " tokens";
        return Number(amount).toLocaleString("zh-CN", { maximumFractionDigits: 6 }) + " " + (completion?.currency || "") + "（估算）";
      },
      completionModules(modules) {
        const unknown = state.completion?.amount == null;
        return modules.map((module) => module.type === "text" ? { ...module, text: String(module.text).replace("上一轮对话消耗:", unknown ? "Codex 任务完成" : "上一轮费用估算:").replace(/^¥\s*/, "") } : module).concat(unknown ? [{ type: "text", text: "未配置单价，费用未知", size: 2, color: "#9fb0d9" }] : []);
      },
      recordsPager(body, data, render) {
        const pagination = data.historyPagination;
        if (!pagination) return;
        const area = mark(document.createElement("div")); area.className = "dshwv-usage-hint"; area.style.cssText = "display:flex;flex-wrap:wrap;gap:6px;align-items:center;margin:4px 0 10px";
        const page = pagination.page || 1, pages = Math.max(1, pagination.pages || 1);
        const label = mark(document.createElement("span")); label.textContent = `明细第 ${page} / ${pages} 页 · 共 ${pagination.total || 0} 条${data.complete === false ? " · 正在统计" : ""}`; area.appendChild(label);
        const input = mark(document.createElement("input")); input.type = "search"; input.placeholder = "搜索全部明细：日期或模型"; input.value = state.recordsSearch || ""; input.className = "dshwv-colnat"; input.style.width = "100%"; area.appendChild(input);
        const request = async (next) => {
          state.recordsRevision = (state.recordsRevision || 0) + 1;
          const revision = state.recordsRevision, priceRevision = state.priceRevision;
          const result = await bridge("/whale/history", { period: "all", page: next, pageSize: pagination.pageSize || 500, search: input.value, prices: prices() }).catch(() => null);
          if (!active() || !body.isConnected || revision !== state.recordsRevision || priceRevision !== state.priceRevision || !result) return;
          state.recordsSearch = input.value; render(mergeRecords(state.recordsAccountBase, result));
        };
        for (const [text, next, disabled] of [["上一页", page - 1, page <= 1], ["下一页", page + 1, page >= pages]]) {
          const button = mark(document.createElement("button")); button.type = "button"; button.textContent = text; button.className = "dshwv-snapbtn dshwv-snapbtn-no"; button.disabled = disabled; button.addEventListener("click", () => { void request(next); }); area.appendChild(button);
        }
        const search = mark(document.createElement("button")); search.type = "button"; search.textContent = "搜索"; search.className = "dshwv-snapbtn dshwv-snapbtn-ok"; search.addEventListener("click", () => { void request(1); }); area.appendChild(search); input.addEventListener("keydown", (event) => { if (event.key === "Enter") { event.preventDefault(); void request(1); } });
        if (data.historyArchive?.detailRetentionDays) { const note = mark(document.createElement("span")); note.textContent = `明细保留 ${data.historyArchive.detailRetentionDays} 天；更早用量保留在日汇总中。`; note.style.width = "100%"; area.appendChild(note); }
        body.appendChild(area);
      },
      fail(error) { state.initializationError = error?.name || "Error"; console.warn("[Codex++] 完整鲸鱼初始化失败", state.initializationError); },
      bind(controls) { state.controls = controls; subscribe(); },
      defaultBubbleItems(items) {
        const copy = JSON.parse(JSON.stringify(items));
        if (copy[0]?.modules) copy[0].modules = [
          { type: "text", text: "Codex 本机用量", size: 8, bold: true },
          { type: "today", modelId: "codex", size: 20, rgb: "indigo", tpl: "{tokens_today}" },
          { type: "today", modelId: "codex", size: 4, color: "#9fb0d9", tpl: "今日 tokens · 近7天 {tokens_week}" },
          { type: "balance", modelId: "current-provider", size: 3, tpl: "供应商余额 {balance}" },
          { type: "session", size: 3, tpl: "{session}", len: 20 },
        ];
        return copy;
      },
      profileSelect(selected) {
        const select = mark(document.createElement("select")); select.className = "dshwv-colnat";
        const profiles = state.relayProfiles.length ? state.relayProfiles : [{ id: selected || "", name: selected || "当前供应商" }];
        for (const profile of profiles) { const option = mark(document.createElement("option")); option.value = profile.id; option.textContent = profile.name || profile.id; select.appendChild(option); }
        select.value = selected || profiles[0]?.id || ""; return select;
      },
    };
    const mutation = trackedObserver(window.MutationObserver), resize = trackedObserver(window.ResizeObserver);
    const environment = [win, doc, fetchCompat, storage, (fn, ms, ...args) => schedule(fn, ms, false, args), clearTimer, (fn, ms, ...args) => schedule(fn, ms, true, args), clearTimer, raf, cancelRaf, mutation, resize, trackedAudioContext, trackedAudioContext, image, audio];
    async function initialize() {
      const configuration = await bridge("/whale/full", { path: "/dsh-whale/size.json", method: "GET", query: {}, body: {} });
      if (configuration?.status !== 200 || !configuration.body) throw new Error("完整挂件设置读取失败");
      state.startupConfig = configuration.body;
      setStatsEnabled(configuration.body.codexStatsOn);
      let legacy;
      try { legacy = JSON.parse(localStorage.getItem("codexPlus.whaleWidget.v1") || "null"); } catch {}
      if (!legacy || storage.getItem("legacy-migrated") === "1") return;
      if (configuration.body.hasSavedConfig === true) { storage.setItem("legacy-migrated", "1"); return; }
      // 只迁移用户实际保存过的旧偏好；保存失败保留旧数据并允许下次重试。
      const send = async (path, method, body) => {
        const result = await bridge("/whale/full", { path: "/dsh-whale/" + path, method, query: {}, body });
        if (result?.status !== 200 || result.body?.ok === false) throw new Error("旧挂件偏好迁移失败");
        return result.body;
      };
      if (typeof legacy.image === "string" && /^data:image\/(png|jpeg|webp|gif);base64,/.test(legacy.image) && legacy.image.length <= 1400000) {
        const roles = await send("roles.json", "GET", {});
        let imported = (roles.roles || []).find((role) => role.name === "原挂件角色");
        if (!imported) {
          const result = await send("roles.json", "POST", { name: "原挂件角色", image: legacy.image, format: legacy.image.startsWith("data:image/gif;") ? "gif" : "png" });
          imported = (result.roles || []).find((role) => role.name === "原挂件角色");
        }
        if (imported?.id) storage.setItem("dshw-role", imported.id);
      }
      if (typeof legacy.phrase === "string" && legacy.phrase.trim()) {
        const config = await send("bubble.json", "GET", {});
        if (!config.config?.items?.length) await send("bubble.json", "POST", { v: 1, tapAdvance: false, lib: [], items: [{ kind: "custom", modules: [{ type: "text", text: legacy.phrase.slice(0, 120), size: 8 }, { type: "today", modelId: "codex", size: 12, tpl: "今日 {tokens_today} tokens" }] }] });
      }
      const limits = Object.values(legacy.thresholds || {}).find((value) => value && typeof value === "object");
      if (limits) {
        const patch = { models: { "current-provider": {} } };
        if (codexPlusWhaleFullNumber(limits.low) > 0) patch.models["current-provider"].alert = { on: true, below: Number(limits.low) };
        if (codexPlusWhaleFullNumber(limits.budget) > 0) patch.models["current-provider"].budget = { on: true, amount: Number(limits.budget) };
        await send("usage-settings.json", "PUT", patch);
      }
      const base = Math.max(122, Math.min(250, Math.min(window.innerWidth || 1024, window.innerHeight || 768) * 0.28));
      const config = { ...configuration.body, sound: legacy.sound === true, turnCostOn: legacy.completion !== false };
      if (codexPlusWhaleFullNumber(legacy.size) > 0) config.scale = Math.min(2.5, Math.max(0.6, Number(legacy.size) / (base * 0.5945)));
      if (codexPlusWhaleFullNumber(legacy.x) !== null && codexPlusWhaleFullNumber(legacy.y) !== null) {
        const fullSize = base * (config.scale || 1.5), right = Math.max(0, (window.innerWidth || 1024) - Number(legacy.x) - Number(legacy.size || 88)), bottom = Math.max(0, (window.innerHeight || 768) - Number(legacy.y) - Number(legacy.size || 88));
        storage.setItem("dshw-pos", JSON.stringify({ v: 2, hAnchor: legacy.x < (window.innerWidth || 1024) / 2 ? "left" : "right", hDist: legacy.x < (window.innerWidth || 1024) / 2 ? Math.max(0, legacy.x - fullSize * 0.4055) : right, vAnchor: legacy.y < (window.innerHeight || 768) / 2 ? "top" : "bottom", vDist: legacy.y < (window.innerHeight || 768) / 2 ? Math.max(0, legacy.y - fullSize * 0.4055) : bottom }));
      }
      if (legacy.snap === false) storage.setItem("dshw-snap", JSON.stringify({ v: 3, mode: "off" }));
      const result = await send("size.json", "PUT", config); state.startupConfig = { ...config, ...result, hasSavedConfig: true };
      storage.setItem("legacy-migrated", "1");
    }
    function pause(paused) {
      if (state.paused === paused || !active()) return;
      state.paused = paused;
      if (paused) {
        for (const timer of state.timers.values()) { if (timer.native !== null) clearTimeout(timer.native); timer.native = null; }
        for (const frame of state.rafs.values()) { if (frame.native !== null) cancelAnimationFrame(frame.native); frame.native = null; }
        for (const observer of state.observers) observer.native.disconnect();
        for (const cancel of [...state.pending]) cancel();
        for (const audio of state.audio) { try { void audio.suspend?.().catch?.(() => {}); } catch {} }
        state.turn = null; state.sessionAt = 0; state.pendingWait = null;
      } else {
        for (const [id, timer] of state.timers) arm(id, timer);
        for (const [id, frame] of state.rafs) frame.native = requestAnimationFrame((time) => { state.rafs.delete(id); if (active() && !state.paused) frame.callback(time); });
        for (const observer of state.observers) for (const [target, options] of observer.records) observer.native.observe(target, options);
        context(); subscribe(); void state.controls?.refresh?.(false);
      }
    }
    function dispose() {
      if (state.disposed) return; state.disposed = true;
      for (const id of [...state.timers.keys()]) clearTimer(id);
      for (const id of [...state.rafs.keys()]) cancelRaf(id);
      for (const observer of state.observers) observer.disconnect(); state.observers.clear();
      for (const entry of state.listeners.splice(0)) entry.target.removeEventListener(entry.name, entry.wrapped, entry.options);
      for (const unsubscribe of state.subscriptions.splice(0)) { try { unsubscribe(); } catch {} }
      for (const cancel of [...state.pending]) cancel();
      for (const context of state.audio) { try { void context.close?.().catch?.(() => {}); } catch {} } state.audio.clear();
      for (const node of state.nodes) { if (node.parentNode) node.parentNode.removeChild(node); } state.nodes.clear();
      for (const url of state.objectUrls) URL.revokeObjectURL(url); state.objectUrls.clear(); state.media.clear();
      state.controls = null;
    }
    listen(document, "visibilitychange", () => pause(document.hidden === true));
    listen(window, "message", (event) => {
      if (event.source !== window || (event.origin && event.origin !== "null" && event.origin !== window.location?.origin)) return;
      nativeEnvelope(event.data);
    }, true);
    listen(window, "codex-message-from-view", (event) => nativeEnvelope(event.detail), true);
    // 设置迁移和首次读取在异步等待中也要记录短任务，绑定原 UI 后统一由 seq 消费。
    subscribe();
    return { state, environment, host, active, pause, dispose, initialize, fetch: fetchCompat, context, session, history, observe, subscribe, nativeEnvelope, prices };
  }
