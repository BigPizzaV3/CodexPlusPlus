(scope, binding, lease) => {
  const key = "__xuanMobileReplyObserver";
  const maxTextLength = 256 * 1024;
  const previous = window[key];
  if (previous?.binding === binding && typeof previous.renew === "function") {
    return previous.renew(scope, lease);
  }
  previous?.dispose();
  const state = {
    binding, scope: new Set(scope), items: new Map(), expires: Date.now() + lease,
    unsubscribers: [], dispatcher: null, timer: null,
  };
  const emit = (threadId, item) => {
    clearTimeout(item.timer);
    item.timer = null;
    if (!state.scope.has(threadId) || Date.now() > state.expires) return;
    try {
      window[binding](JSON.stringify({
        threadId, itemId: item.id, sequence: item.sequence, text: item.text,
        complete: item.complete,
      }));
      if (item.complete && state.items.get(threadId) === item) {
        state.items.delete(threadId);
      }
    } catch {
      state.dispose();
    }
  };
  const observe = (method, payload) => {
    const params = payload?.params || payload;
    const threadId = params?.threadId;
    if (!state.scope.has(threadId)) return;
    const id = params.itemId || params.item?.id;
    if (typeof id !== "string" || !id || id.length > 128) return;
    if (method !== "item/agentMessage/delta" && params.item?.type !== "agentMessage") return;
    let item = state.items.get(threadId);
    if (!item || item.id !== id) {
      // 中途加入时不能把缺少前缀的 delta 当作完整正文，等待完整消息事件或日志补齐。
      if (method === "item/agentMessage/delta") return;
      if (item?.timer) emit(threadId, item);
      // 新消息先锚定历史，后续只发送累计正文，桥接重放不会重复追加片段。
      item = { id, text: "", sequence: 0, complete: false, timer: null,
        suppressed: !!params.item?.channel && !["commentary", "final"].includes(params.item.channel) };
      state.items.set(threadId, item);
    }
    if (item.complete || item.suppressed) return;
    if (method === "item/agentMessage/delta") {
      if (typeof params.delta !== "string") return;
      if (item.text.length + params.delta.length > maxTextLength) {
        clearTimeout(item.timer);
        state.items.delete(threadId);
        return;
      }
      item.text += params.delta;
    } else if (typeof params.item.text === "string") {
      if (params.item.text.length > maxTextLength) {
        clearTimeout(item.timer);
        state.items.delete(threadId);
        return;
      }
      item.text = params.item.text;
    }
    item.complete = method === "item/completed";
    item.sequence++;
    if (method === "item/started" || item.complete) {
      emit(threadId, item);
    } else if (!item.timer) {
      item.timer = setTimeout(() => emit(threadId, item), 500);
    }
  };
  state.renew = (nextScope, nextLease) => {
    state.scope = new Set(nextScope);
    state.expires = Date.now() + nextLease;
    for (const id of state.items.keys()) {
      if (!state.scope.has(id)) {
        clearTimeout(state.items.get(id).timer);
        state.items.delete(id);
      }
    }
    return state.attach();
  };
  state.attach = () => {
    const dispatcher = window.__codexPlusRemoteSessionRecoveryDispatcher;
    if (!dispatcher || typeof dispatcher.subscribe !== "function") return false;
    if (state.dispatcher === dispatcher) return true;
    for (const unsubscribe of state.unsubscribers) {
      if (typeof unsubscribe === "function") unsubscribe();
    }
    state.unsubscribers = [];
    state.dispatcher = dispatcher;
    for (const method of ["item/started", "item/agentMessage/delta", "item/completed"]) {
      state.unsubscribers.push(dispatcher.subscribe(method, payload => observe(method, payload)));
    }
    return true;
  };
  state.dispose = () => {
    clearInterval(state.timer);
    for (const unsubscribe of state.unsubscribers) {
      if (typeof unsubscribe === "function") unsubscribe();
    }
    state.unsubscribers = [];
    for (const item of state.items.values()) clearTimeout(item.timer);
    state.items.clear();
    state.scope.clear();
    if (window[key] === state) delete window[key];
  };
  // CDP 异常断开时也会自动撤销观察者，不留下永久监听和正文缓存。
  state.timer = setInterval(() => {
    if (Date.now() > state.expires) state.dispose();
  }, Math.min(lease, 5000));
  window[key] = state;
  return state.attach();
}
