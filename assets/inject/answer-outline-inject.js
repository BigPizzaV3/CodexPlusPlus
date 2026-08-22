(() => {
  "use strict";

  const API_KEY = "__codexAnswerOutline";
  const BRIDGE_KEY = "__codexSessionDeleteBridge";
  const ROOT_ATTR = "data-codex-answer-outline-target";
  const HIGHLIGHT_CLASS = "codex-answer-outline-target-flash";
  const STYLE_ID = "codex-answer-outline-engine-style";
  const TURN_SELECTOR = "div.contents[data-content-search-turn-key]";
  const SEMANTIC_SELECTOR = "h1,h2,h3,h4,h5,h6,[role='heading']";
  const PSEUDO_SELECTOR = "p,div,li,strong,b";
  const TABLE_SELECTOR = [
    "table", "thead", "tbody", "tfoot", "tr", "td", "th",
    "[role='table']", "[role='row']", "[role='cell']",
    "[role='columnheader']", "[role='rowheader']",
  ].join(",");
  const MAX_ITEMS = 24;
  const MIN_TITLE_LENGTH = 2;
  const MAX_TITLE_LENGTH = 56;
  const TOP_OFFSET = 28;
  const SETTLE_DELAY_MS = 140;
  const SETTLE_WINDOW_MS = 720;
  const REFRESH_DELAY_MS = 220;
  const BRIDGE_TIMEOUT_MS = 4000;

  const previous = window[API_KEY];
  if (previous && typeof previous.destroy === "function") previous.destroy();

  const state = {
    enabled: false,
    status: "idle",
    error: "",
    items: [],
    message: null,
    messageId: "",
    sourceHash: "",
    observer: null,
    timer: 0,
    refreshPromise: null,
    scrollCleanup: null,
    listeners: new Set(),
    nodeIds: new WeakMap(),
    nodeSequence: 0,
    destroyed: false,
  };

  function normalizeText(value) {
    return String(value || "")
      .replace(/\u00a0/g, " ")
      .replace(/[ \t]+\n/g, "\n")
      .replace(/\n{3,}/g, "\n\n")
      .replace(/[ \t]{2,}/g, " ")
      .trim();
  }

  function hashText(value) {
    const text = normalizeText(value).slice(-12000);
    let hash = 2166136261;
    for (let index = 0; index < text.length; index += 1) {
      hash ^= text.charCodeAt(index);
      hash = Math.imul(hash, 16777619);
    }
    return (hash >>> 0).toString(36);
  }

  function clamp(value, min, max) {
    return Math.min(max, Math.max(min, value));
  }

  function visible(node) {
    if (!(node instanceof Element)) return false;
    const rect = node.getBoundingClientRect();
    return rect.width > 8 && rect.height > 8;
  }

  function nodeId(node, prefix = "node") {
    if (!(node instanceof Node)) return "";
    const existing = state.nodeIds.get(node);
    if (existing) return existing;
    const id = `${prefix}-${++state.nodeSequence}`;
    state.nodeIds.set(node, id);
    return id;
  }

  function bridgeCall(path, payload = {}) {
    if (typeof window[BRIDGE_KEY] !== "function") {
      return Promise.resolve({ status: "failed", error: "page bridge is not installed" });
    }
    let timer = 0;
    const timeout = new Promise((resolve) => {
      timer = window.setTimeout(
        () => resolve({ status: "failed", error: "page bridge timed out" }),
        BRIDGE_TIMEOUT_MS,
      );
    });
    return Promise.race([Promise.resolve(window[BRIDGE_KEY](path, payload)), timeout])
      .finally(() => window.clearTimeout(timer));
  }

  function publicState() {
    return {
      enabled: state.enabled,
      status: state.status,
      error: state.error,
      messageId: state.messageId,
      sourceHash: state.sourceHash,
      items: state.items.map(({ element, ...item }) => ({ ...item })),
    };
  }

  function notify() {
    const snapshot = publicState();
    for (const listener of state.listeners) {
      try {
        listener(snapshot);
      } catch {}
    }
  }

  function subscribe(listener) {
    if (typeof listener !== "function") return () => {};
    state.listeners.add(listener);
    listener(publicState());
    return () => state.listeners.delete(listener);
  }

  function threadRoots() {
    return Array.from(document.querySelectorAll(".thread-scroll-container"))
      .filter((node) => node instanceof HTMLElement)
      .filter(visible)
      .sort((left, right) => {
        const leftRect = left.getBoundingClientRect();
        const rightRect = right.getBoundingClientRect();
        return rightRect.width * rightRect.height - leftRect.width * leftRect.height;
      });
  }

  function roleNode(turn, role) {
    return Array.from(turn.querySelectorAll("[data-message-author-role]"))
      .find((node) => node.getAttribute("data-message-author-role")?.toLowerCase() === role) || null;
  }

  function compareTurnKeys(left, right) {
    return String(left || "").localeCompare(String(right || ""), undefined, {
      numeric: true,
      sensitivity: "base",
    });
  }

  function latestAnswer() {
    const root = threadRoots()[0];
    if (!root) return null;
    const turns = Array.from(root.querySelectorAll(TURN_SELECTOR))
      .map((turn) => {
        const assistant = roleNode(turn, "assistant");
        const text = normalizeText(assistant?.innerText || assistant?.textContent);
        if (!assistant || text.length < 8) return null;
        return {
          node: assistant,
          turn,
          user: roleNode(turn, "user"),
          text,
          turnKey: turn.getAttribute("data-content-search-turn-key") || nodeId(turn, "turn"),
        };
      })
      .filter(Boolean);
    return turns.reduce((latest, candidate) => {
      if (!latest) return candidate;
      return compareTurnKeys(candidate.turnKey, latest.turnKey) > 0 ? candidate : latest;
    }, null);
  }

  function markdownRoot(messageNode) {
    if (!(messageNode instanceof Element)) return null;
    return messageNode.querySelector([
      "[class*='markdownContent']",
      "[class*='markdown-content']",
      ".markdown",
      ".prose",
      "article",
    ].join(",")) || messageNode;
  }

  function protectedSurface(node) {
    return !(node instanceof Element) || Boolean(node.closest([
      "[contenteditable='true']", "textarea", "input", "form", ".ProseMirror",
    ].join(",")));
  }

  function inCodeLike(node) {
    return !(node instanceof Element)
      || Boolean(node.closest("pre,code,kbd,samp,[data-code-block],.cm-editor,.monaco-editor"));
  }

  function inTableLike(node) {
    return !(node instanceof Element) || Boolean(node.closest(TABLE_SELECTOR));
  }

  function headingLevel(node) {
    const tagMatch = /^h([1-6])$/i.exec(node?.tagName || "");
    if (tagMatch) return Number(tagMatch[1]);
    const ariaLevel = Number(node?.getAttribute?.("aria-level"));
    return Number.isFinite(ariaLevel) ? clamp(ariaLevel, 1, 6) : 0;
  }

  function stripMarkers(value) {
    return normalizeText(value)
      .replace(/^#{1,6}\s+/, "")
      .replace(/\s+#{1,6}$/, "")
      .replace(/[：:]$/, "")
      .trim();
  }

  function numbering(value) {
    const text = normalizeText(value);
    const match = text.match(/^((?:第[一二三四五六七八九十百零\d]+[章节部分步]|[一二三四五六七八九十]+[、.．]|（?\d{1,2}）|\(\d{1,2}\)|\d{1,2}(?:\.\d+)*[、.．)]))\s*/);
    if (!match) return { prefix: "", label: stripMarkers(text), levelHint: 0 };
    const prefix = match[1];
    const dottedDepth = (prefix.match(/[.．]/g) || []).length;
    return {
      prefix,
      label: stripMarkers(text.slice(match[0].length)),
      levelHint: clamp(dottedDepth + 1, 1, 6),
    };
  }

  function chapterTitle(value) {
    const text = normalizeText(value);
    if (text.length > 32) return false;
    return /^(摘要|简介|概述|概览|前言|背景|目标|现状|问题(?:分析)?|原因(?:分析)?|分析|方案|解决方案|步骤|实施步骤|实现|验证|验证结果|测试|测试结果|结果|结论|最终结论|总结|建议|后续建议|注意(?:事项)?|说明|补充说明|附录|下一步)(?:\s*[：:—-]\s*\S.*)?$/i.test(text)
      || /^(abstract|introduction|overview|background|goals?|problems?|causes?|analysis|solutions?|steps?|implementation|verification|tests?|results?|conclusions?|summary|recommendations?|notes?|appendix|next steps?)(?:\s*[:：—-]\s*\S.*)?$/i.test(text);
  }

  function noiseTitle(value) {
    const text = normalizeText(value);
    if (text.length < MIN_TITLE_LENGTH || text.length > MAX_TITLE_LENGTH) return true;
    if (/^https?:\/\//i.test(text)) return true;
    if (/^[\w./~-]+\.(js|ts|json|md|py|sh|log|png|jpg)$/i.test(text)) return true;
    if (/^[\d\s:./-]+$/.test(text)) return true;
    if (/^(OK|PASS|FAIL|true|false|null)$/i.test(text)) return true;
    return /^(复制|copy|edit|编辑|share|分享|continue|继续|retry|重试|刷新)$/i.test(text);
  }

  function pseudoHeading(value) {
    const text = normalizeText(value);
    return /^#{1,6}\s+\S/.test(text)
      || /^第[一二三四五六七八九十百零\d]+[章节部分步]/.test(text)
      || /^[一二三四五六七八九十]+[、.．]\s*\S{2,}/.test(text)
      || /^（?\d{1,2}）\s*\S{2,}/.test(text)
      || /^\(\d{1,2}\)\s*\S{2,}/.test(text)
      || /^\d{1,2}(?:\.\d+)*[、.．)]\s*\S{2,}/.test(text)
      || chapterTitle(text);
  }

  function ownsLine(node, text) {
    if (!(node instanceof Element)) return false;
    const ownText = normalizeText(node.innerText || node.textContent);
    if (ownText !== text) return false;
    const parentText = normalizeText(node.parentElement?.innerText || node.parentElement?.textContent);
    return parentText === text || node.matches("p,li,strong,b,[role='heading']");
  }

  function candidate(node, kind) {
    if (!(node instanceof HTMLElement) || !visible(node)) return null;
    if (protectedSurface(node) || inCodeLike(node) || inTableLike(node)) return null;
    const raw = normalizeText(node.innerText || node.textContent);
    const parsed = numbering(raw);
    const label = parsed.label;
    if (noiseTitle(label)) return null;
    const rawLevel = headingLevel(node) || parsed.levelHint;
    if (kind === "pseudo" && (!pseudoHeading(raw) || !ownsLine(node, raw))) return null;
    if (!rawLevel && !chapterTitle(label)) return null;
    return {
      element: node,
      text: `${parsed.prefix}${parsed.prefix && label ? " " : ""}${label}`.trim(),
      labelText: label,
      numberPrefix: parsed.prefix,
      rawLevel: rawLevel || 2,
      kind,
    };
  }

  function collect(root) {
    const semantic = Array.from(root.querySelectorAll(SEMANTIC_SELECTOR))
      .map((node) => candidate(node, "semantic"))
      .filter(Boolean);
    const pseudo = Array.from(root.querySelectorAll(PSEUDO_SELECTOR))
      .map((node) => candidate(node, "pseudo"))
      .filter(Boolean);
    const combined = [...semantic, ...pseudo]
      .sort((left, right) => {
        if (left.element === right.element) return left.kind === "semantic" ? -1 : 1;
        const position = left.element.compareDocumentPosition(right.element);
        return position & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1;
      });
    const seenElements = new Set();
    const seenTitles = new Set();
    const items = [];
    for (const item of combined) {
      const key = normalizeText(item.text).toLocaleLowerCase().replace(/[：:]/g, "");
      if (seenElements.has(item.element) || seenTitles.has(key)) continue;
      seenElements.add(item.element);
      seenTitles.add(key);
      items.push(item);
      if (items.length >= MAX_ITEMS) break;
    }
    const minimumLevel = Math.min(...items.map((item) => item.rawLevel), 1);
    return items.map((item, index) => ({
      ...item,
      id: `outline-${state.messageId}-${index + 1}`,
      level: item.rawLevel,
      displayLevel: Math.max(0, item.rawLevel - minimumLevel),
    }));
  }

  function clearMarks() {
    document.querySelectorAll(`[${ROOT_ATTR}]`).forEach((node) => node.removeAttribute(ROOT_ATTR));
  }

  function mark(items) {
    clearMarks();
    for (const item of items) item.element.setAttribute(ROOT_ATTR, item.id);
  }

  function mapBridgeItems(root, rawItems) {
    const elements = Array.from(root.querySelectorAll(`${SEMANTIC_SELECTOR},${PSEUDO_SELECTOR}`));
    return rawItems.map((item, index) => {
      const label = normalizeText(item.title || item.labelText || item.text);
      const element = elements.find((node) => stripMarkers(node.innerText || node.textContent).replace(/^[\d一二三四五六七八九十]+[、.．)]\s*/, "") === label);
      if (!element) return null;
      return {
        id: `outline-${state.messageId}-bridge-${index + 1}`,
        element,
        text: label,
        labelText: label,
        numberPrefix: item.numberPrefix || "",
        level: Number(item.level) || 1,
        rawLevel: Number(item.level) || 1,
        displayLevel: Math.max(0, Number(item.level || 1) - 1),
        kind: "bridge",
      };
    }).filter(Boolean);
  }

  async function build(answer) {
    const root = markdownRoot(answer.node);
    if (!root) return [];
    const localItems = collect(root);
    if (localItems.length >= 2) return localItems;
    const response = await bridgeCall("/answer-outline/parse", {
      enabled: true,
      text: answer.text,
      maxItems: MAX_ITEMS,
    });
    if (!Array.isArray(response?.items)) return localItems;
    const bridged = mapBridgeItems(root, response.items);
    return bridged.length > localItems.length ? bridged : localItems;
  }

  async function loadEnabled() {
    const response = await bridgeCall("/stepwise/settings", {});
    if (response?.settings) {
      state.enabled = response.settings.answerOutlineEnabled === true;
    }
    return state.enabled;
  }

  async function refresh(options = {}) {
    if (state.destroyed || !state.enabled) return publicState();
    if (state.refreshPromise && options.force !== true) return state.refreshPromise;
    const answer = latestAnswer();
    const messageId = answer?.turnKey || nodeId(answer?.node, "answer");
    const sourceHash = hashText(answer?.text || "");
    if (!options.force && messageId === state.messageId && sourceHash === state.sourceHash) {
      return publicState();
    }
    state.status = "pending";
    state.error = "";
    notify();
    const task = Promise.resolve().then(async () => {
      if (!answer) throw new Error("未找到当前回答");
      const items = await build(answer);
      if (state.destroyed) return;
      state.message = answer;
      state.messageId = messageId;
      state.sourceHash = sourceHash;
      state.items = items;
      state.status = items.length ? "ready" : "empty";
      mark(items);
    }).catch((error) => {
      if (state.destroyed) return;
      clearMarks();
      state.items = [];
      state.status = "error";
      state.error = String(error?.message || error || "大纲暂不可用");
    }).finally(() => {
      if (state.refreshPromise === task) state.refreshPromise = null;
      notify();
    });
    state.refreshPromise = task;
    return task;
  }

  function scrollContainer(element) {
    let node = element?.parentElement;
    while (node && node !== document.documentElement) {
      const style = getComputedStyle(node);
      if (/(auto|scroll|overlay)/.test(style.overflowY || style.overflow)
        && node.scrollHeight > node.clientHeight + 4) return node;
      node = node.parentElement;
    }
    return document.scrollingElement || document.documentElement;
  }

  function documentScroller(container) {
    return container === document.scrollingElement
      || container === document.documentElement
      || container === document.body;
  }

  function scrollBounds(container) {
    if (documentScroller(container)) {
      const maximum = Math.max(0, document.documentElement.scrollHeight - window.innerHeight);
      return { current: window.scrollY, maximum };
    }
    return { current: container.scrollTop, maximum: Math.max(0, container.scrollHeight - container.clientHeight) };
  }

  function targetScrollTop(element, container, align = "start") {
    const bounds = scrollBounds(container);
    const targetRect = element.getBoundingClientRect();
    if (documentScroller(container)) {
      const desired = align === "end"
        ? window.scrollY + targetRect.bottom - window.innerHeight + TOP_OFFSET
        : window.scrollY + targetRect.top - TOP_OFFSET;
      return clamp(desired, 0, bounds.maximum);
    }
    const containerRect = container.getBoundingClientRect();
    const desired = align === "end"
      ? container.scrollTop + targetRect.bottom - containerRect.bottom + TOP_OFFSET
      : container.scrollTop + targetRect.top - containerRect.top - TOP_OFFSET;
    return clamp(desired, 0, bounds.maximum);
  }

  function writeScroll(container, top, behavior = "smooth") {
    if (documentScroller(container)) window.scrollTo({ top, behavior });
    else container.scrollTo({ top, behavior });
  }

  function settleScroll(element, container, align) {
    state.scrollCleanup?.();
    let cancelled = false;
    const started = Date.now();
    const timer = window.setInterval(() => {
      if (cancelled || !element.isConnected || Date.now() - started > SETTLE_WINDOW_MS) {
        window.clearInterval(timer);
        return;
      }
      const expected = targetScrollTop(element, container, align);
      const current = scrollBounds(container).current;
      if (Math.abs(expected - current) > 2) writeScroll(container, expected, "auto");
    }, SETTLE_DELAY_MS);
    state.scrollCleanup = () => {
      cancelled = true;
      window.clearInterval(timer);
      state.scrollCleanup = null;
    };
  }

  function flash(element) {
    element.classList.remove(HIGHLIGHT_CLASS);
    void element.offsetWidth;
    element.classList.add(HIGHLIGHT_CLASS);
    window.setTimeout(() => element.classList.remove(HIGHLIGHT_CLASS), 1200);
  }

  function jumpTo(id) {
    const item = state.items.find((candidate) => candidate.id === id);
    if (!item?.element?.isConnected) return false;
    const container = scrollContainer(item.element);
    const top = targetScrollTop(item.element, container, "start");
    writeScroll(container, top);
    settleScroll(item.element, container, "start");
    flash(item.element);
    return true;
  }

  function jumpToAnchor(anchor) {
    const answer = state.message || latestAnswer();
    if (!answer) return false;
    const isStart = anchor === "start";
    const target = isStart ? answer.user || answer.turn || answer.node : answer.node.lastElementChild || answer.node;
    if (!(target instanceof Element)) return false;
    const container = scrollContainer(target);
    writeScroll(container, targetScrollTop(target, container, isStart ? "start" : "end"));
    settleScroll(target, container, isStart ? "start" : "end");
    flash(isStart ? answer.node : target);
    return true;
  }

  function installStyle() {
    if (document.getElementById(STYLE_ID)) return;
    const style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      .${HIGHLIGHT_CLASS} {
        animation: codex-answer-outline-flash 1.2s ease-out;
        border-radius: 8px;
      }
      @keyframes codex-answer-outline-flash {
        0% { background: color-mix(in srgb, #3878ff 22%, transparent); }
        100% { background: transparent; }
      }
    `;
    document.head.appendChild(style);
  }

  function scheduleRefresh(delay = REFRESH_DELAY_MS) {
    if (state.destroyed || !state.enabled) return;
    window.clearTimeout(state.timer);
    state.timer = window.setTimeout(() => void refresh(), delay);
  }

  function observe() {
    state.observer?.disconnect();
    const root = document.body || document.documentElement;
    if (!root) return false;
    state.observer = new MutationObserver((records) => {
      const relevant = records.some((record) => record.type === "characterData" || record.addedNodes.length);
      if (relevant) scheduleRefresh();
    });
    state.observer.observe(root, { childList: true, subtree: true, characterData: true });
    return true;
  }

  async function syncSettings(patch = {}) {
    if (typeof patch.answerOutlineEnabled === "boolean") {
      state.enabled = patch.answerOutlineEnabled;
    } else {
      await loadEnabled();
    }
    if (!state.enabled) {
      clearMarks();
      state.items = [];
      state.message = null;
      state.messageId = "";
      state.sourceHash = "";
      state.status = "disabled";
      notify();
      return publicState();
    }
    installStyle();
    observe();
    await refresh({ force: true });
    return publicState();
  }

  function destroy() {
    state.destroyed = true;
    window.clearTimeout(state.timer);
    state.observer?.disconnect();
    state.scrollCleanup?.();
    clearMarks();
    state.listeners.clear();
    document.getElementById(STYLE_ID)?.remove();
    if (window[API_KEY]?.instanceId === instanceId) delete window[API_KEY];
  }

  async function start() {
    await loadEnabled();
    if (state.destroyed || !state.enabled) {
      state.status = "disabled";
      notify();
      return;
    }
    installStyle();
    if (!observe()) {
      document.addEventListener("DOMContentLoaded", () => void start(), { once: true });
      return;
    }
    await refresh({ force: true });
  }

  const instanceId = `outline-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  window[API_KEY] = {
    version: "1.0.0",
    instanceId,
    state,
    current: publicState,
    subscribe,
    refresh,
    jumpTo,
    jumpToAnchor,
    syncSettings,
    destroy,
  };
  void start();
})();
