/* Shared answer-context and DOM conversation helpers. */

  function roleFromElement(node) {
    if (!(node instanceof Element)) return "";
    const explicit = node.getAttribute("data-message-author-role");
    if (explicit) return explicit.toLowerCase();

    const text = elementText(node);
    if (/^(assistant|codex|assistant\s+said)\b/i.test(text)) return "assistant";
    if (/^(user|you)\b/i.test(text)) return "user";
    return "";
  }

  function threadRoots() {
    return Array.from(document.querySelectorAll(".thread-scroll-container"))
      .filter((node) => node instanceof HTMLElement)
      .filter((node) => visibleElement(node) && !state.root?.contains(node));
  }

  function threadRootOf(node) {
    if (!(node instanceof Element)) return null;
    return node.closest?.(".thread-scroll-container") || null;
  }

  function stablePaneKeyForRoot(root) {
    if (!(root instanceof Element)) return "";
    let current = root;
    for (let depth = 0; current && depth < 10; depth += 1, current = current.parentElement) {
      const controller = current.getAttribute("data-app-shell-tab-panel-controller");
      if (controller) return `pane:controller:${controller}`;
      const focusArea = current.getAttribute("data-app-shell-focus-area");
      if (focusArea) return `pane:focus:${focusArea}`;
      const anchorHost = current.getAttribute("data-pip-anchor-host");
      if (anchorHost) return `pane:anchor:${anchorHost === "codex-main-thread" ? "main" : anchorHost}`;
    }

    const roots = threadRoots();
    if (roots.length <= 1) return "pane:main";
    const ordered = roots
      .map((node) => ({ node, left: visibleRect(node)?.left ?? Number.POSITIVE_INFINITY }))
      .sort((left, right) => left.left - right.left);
    const index = Math.max(0, ordered.findIndex((item) => item.node === root));
    return index === 0 ? "pane:main" : `pane:secondary:${index}`;
  }

  function nodeIdentity(node, prefix = "node") {
    if (!(node instanceof Element)) return "";
    const explicit = [
      node.getAttribute("data-conversation-id"),
      node.getAttribute("data-session-id"),
      node.getAttribute("data-thread-id"),
      node.getAttribute("data-message-id"),
      node.getAttribute("data-turn-id"),
      node.id,
    ].find(Boolean);
    if (explicit) return `${prefix}:${explicit}`;
    if (!state.nodeKeys.has(node)) {
      state.nodeKeySeq += 1;
      state.nodeKeys.set(node, `${prefix}:${state.nodeKeySeq}`);
    }
    return state.nodeKeys.get(node);
  }

  function sessionIdForRoot(root) {
    if (!(root instanceof Element)) return "";

    const conversationMarkers = [
      "data-above-composer-conversation-id",
      "data-response-annotation-conversation",
    ];
    for (const attribute of conversationMarkers) {
      const marker = root.hasAttribute?.(attribute)
        ? root
        : root.querySelector?.(`[${attribute}]`);
      const value = marker?.getAttribute?.(attribute);
      if (value) return String(value);
    }

    let current = root;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      const value = [
        current.getAttribute?.("data-conversation-id"),
        current.getAttribute?.("data-session-id"),
        current.getAttribute?.("data-thread-id"),
      ].find(Boolean);
      if (value) return String(value);
    }

    // Side chats do not expose the main conversation marker; their tab ID is stable.
    current = root;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      const tabId = current.getAttribute?.("data-tab-id");
      if (tabId) return String(tabId);
    }

    const descendant = root.querySelector?.("[data-conversation-id], [data-session-id], [data-thread-id]");
    const descendantValue = [
      descendant?.getAttribute?.("data-conversation-id"),
      descendant?.getAttribute?.("data-session-id"),
      descendant?.getAttribute?.("data-thread-id"),
    ].find(Boolean);
    if (descendantValue) return String(descendantValue);
    const links = Array.from(root.querySelectorAll("a[href*='/c/'], a[href*='/conversation/']"));
    for (const link of links) {
      const match = String(link.getAttribute("href") || "").match(/\/(?:c|conversation)\/([^/?#]+)/i);
      if (match?.[1]) return match[1];
    }
    const routeMatch = location.pathname.match(/\/(?:c|conversation)\/([^/?#]+)/i);
    const paneKey = stablePaneKeyForRoot(root);
    if ((paneKey === "pane:anchor:main" || paneKey === "pane:main") && routeMatch?.[1]) return routeMatch[1];
    if (threadRoots().length <= 1 && routeMatch?.[1]) return routeMatch[1];
    return paneKey;
  }

  function assistantMessageId(message) {
    if (message?.turnKey) return `turn:${message.turnKey}`;
    const node = message?.node;
    if (!(node instanceof Element)) return "";
    return nodeIdentity(node, "assistant");
  }

  function resetContextContent() {
    state.stepwiseEpoch += 1;
    state.latestTurnAnchor = null;
    state.lastAssistantHash = "";
    state.lastAssistantAt = 0;
    state.currentHash = "";
    state.prompts = [];
    state.promptPreviewIndex = 0;
    state.bridgeActiveKey = "";
    state.bridgePendingHash = "";
    state.bridgePendingRequestId = 0;
    state.bridgePendingMode = stepwiseGenerationMode();
    state.bridgeStatus = "idle";
    state.bridgeError = "";
    state.promptContext = null;
    state.outlineRefreshPromise = null;
    invalidateOutline();
  }

  // Conversation tracking pins one thread and latest completed turn independently of virtualized DOM mounts.
  function installContextTracking() {
    if (!state.pointerHandler) {
      state.pointerHandler = (event) => {
        if (pinThreadFromTarget(event.target, "pointer")) scheduleScan(0);
      };
      document.addEventListener("pointerdown", state.pointerHandler, true);
    }
    if (!state.focusHandler) {
      state.focusHandler = (event) => {
        if (pinThreadFromTarget(event.target, "focus")) scheduleScan(0);
      };
      document.addEventListener("focusin", state.focusHandler, true);
    }
    if (!state.selectionHandler) {
      state.selectionHandler = () => {
        const selection = document.getSelection();
        const node = selection?.anchorNode;
        const target = node instanceof Element ? node : node?.parentElement;
        if (target && pinThreadFromTarget(target, "selection")) scheduleScan(0);
      };
      document.addEventListener("selectionchange", state.selectionHandler, true);
    }
  }

  function removeContextTracking() {
    if (state.pointerHandler) document.removeEventListener("pointerdown", state.pointerHandler, true);
    if (state.focusHandler) document.removeEventListener("focusin", state.focusHandler, true);
    if (state.selectionHandler) document.removeEventListener("selectionchange", state.selectionHandler, true);
    state.pointerHandler = null;
    state.focusHandler = null;
    state.selectionHandler = null;
  }

  function setActiveThreadRoot(root, reason = "resolve") {
    if (!(root instanceof HTMLElement) || !root.isConnected) return false;
    const paneKey = stablePaneKeyForRoot(root);
    const sessionId = sessionIdForRoot(root);
    const previous = state.activeContext;
    const sessionChanged = previous.sessionId !== sessionId;
    const identityChanged = previous.paneKey !== paneKey || sessionChanged;
    if (!identityChanged && previous.paneRoot === root) return false;
    if (!identityChanged) {
      state.activeContext = {
        ...previous,
        paneRoot: root,
      };
      if (state.pinnedPaneKey === paneKey && state.pinnedSessionId === sessionId) {
        state.pinnedThreadRoot = root;
      }
      pushDiagnostic("context:rebound", {
        reason,
        paneKey,
        sessionId,
        generation: state.activeContext.generation,
        paneCount: threadRoots().length,
        paneRect: rectSummary(root),
      });
      renderFloat();
      return true;
    }
    state.activeContext = {
      paneRoot: root,
      paneKey,
      sessionId,
      assistantMessageId: "",
      generation: previous.generation + 1,
    };
    if (state.pinnedPaneKey === paneKey && state.pinnedThreadRoot === root) {
      state.pinnedSessionId = sessionId;
    }
    resetContextContent();
    pushDiagnostic("context:changed", {
      reason,
      paneKey,
      sessionId,
      sessionChanged,
      generation: state.activeContext.generation,
      paneCount: threadRoots().length,
      paneRect: rectSummary(root),
    });
    renderFloat();
    return true;
  }

  function contextSnapshot() {
    return {
      runtimeGeneration: state.runtimeGeneration,
      generation: state.activeContext.generation,
      paneKey: state.activeContext.paneKey,
      sessionId: state.activeContext.sessionId,
      assistantMessageId: state.activeContext.assistantMessageId,
    };
  }

  function contextMatches(snapshot) {
    if (!snapshot) return false;
    if (!isCurrentRuntime(snapshot.runtimeGeneration)) return false;
    const current = state.activeContext;
    return snapshot.generation === current.generation &&
      snapshot.paneKey === current.paneKey &&
      snapshot.sessionId === current.sessionId &&
      snapshot.assistantMessageId === current.assistantMessageId;
  }

  function pinThreadFromTarget(target, reason) {
    if (!(target instanceof Element) || state.root?.contains(target)) return false;
    const root = threadRootOf(target);
    if (!root) return false;
    state.pinnedPaneKey = stablePaneKeyForRoot(root);
    state.pinnedSessionId = sessionIdForRoot(root);
    state.pinnedThreadRoot = root;
    state.pinnedThreadAt = Date.now();
    state.threadActivity.set(root, state.pinnedThreadAt);
    return setActiveThreadRoot(root, reason);
  }

  function rootMatchesContext(root, paneKey, sessionId) {
    if (!(root instanceof Element) || !paneKey) return false;
    if (stablePaneKeyForRoot(root) !== paneKey) return false;
    return !sessionId || sessionIdForRoot(root) === sessionId;
  }

  function rootForContext(paneKey, sessionId, roots = threadRoots()) {
    if (!paneKey) return null;
    return roots.find((root) => rootMatchesContext(root, paneKey, sessionId))
      || roots.find((root) => stablePaneKeyForRoot(root) === paneKey)
      || null;
  }

  function resolveActiveThreadRoot() {
    const roots = threadRoots();
    if (!roots.length) {
      state.activeContext.paneRoot = null;
      return null;
    }
    const current = state.activeContext.paneRoot;
    if (current?.isConnected && roots.includes(current)) {
      const sessionId = sessionIdForRoot(current);
      if (sessionId !== state.activeContext.sessionId) setActiveThreadRoot(current, "session-change");
      return current;
    }
    const pinned = rootForContext(state.pinnedPaneKey, state.pinnedSessionId, roots)
      || (state.pinnedThreadRoot?.isConnected && roots.includes(state.pinnedThreadRoot) ? state.pinnedThreadRoot : null);
    if (pinned) {
      state.pinnedThreadRoot = pinned;
      setActiveThreadRoot(pinned, "pinned");
      return pinned;
    }
    const rebound = rootForContext(state.activeContext.paneKey, state.activeContext.sessionId, roots);
    if (rebound) {
      setActiveThreadRoot(rebound, "active-rebound");
      return rebound;
    }
    const focused = threadRootOf(document.activeElement);
    if (focused && roots.includes(focused)) {
      setActiveThreadRoot(focused, "focus");
      return focused;
    }
    const fallback = roots[0];
    setActiveThreadRoot(fallback, roots.length === 1 ? "single-pane" : "fallback");
    return fallback;
  }

  function activePaneCue() {
    const roots = threadRoots();
    const active = state.activeContext.paneRoot;
    const centerCue = paneCueForTrack({ direction: "single", angle: null }, CHIP_HEIGHT);
    if (roots.length < 2 || !active?.isConnected) return centerCue;
    const activeRect = visibleRect(active);
    if (!activeRect) return centerCue;
    const rects = roots.map(visibleRect).filter(Boolean);
    if (rects.length < 2) return centerCue;
    const bounds = {
      left: Math.min(...rects.map((rect) => rect.left)),
      top: Math.min(...rects.map((rect) => rect.top)),
      right: Math.max(...rects.map((rect) => rect.right)),
      bottom: Math.max(...rects.map((rect) => rect.bottom)),
    };
    const boundsWidth = Math.max(1, bounds.right - bounds.left);
    const boundsHeight = Math.max(1, bounds.bottom - bounds.top);
    const offsetX = ((activeRect.left + activeRect.width / 2) - (bounds.left + boundsWidth / 2)) / (boundsWidth / 2);
    const offsetY = ((activeRect.top + activeRect.height / 2) - (bounds.top + boundsHeight / 2)) / (boundsHeight / 2);
    if (Math.abs(offsetX) < 0.01 && Math.abs(offsetY) < 0.01) return centerCue;
    const angle = Math.atan2(offsetY, offsetX);
    const direction = Math.abs(offsetX) >= Math.abs(offsetY)
      ? (offsetX < 0 ? "left" : "right")
      : (offsetY < 0 ? "top" : "bottom");
    return paneCueForTrack({ direction, angle }, CHIP_HEIGHT);
  }

  function paneCueForTrack(paneCue, trackHeight = CHIP_HEIGHT) {
    if (paneCue.direction === "single" || !Number.isFinite(paneCue.angle)) {
      return { direction: "single", angle: null, x: CHIP_WIDTH / 2, y: trackHeight / 2 };
    }
    const point = capsuleBoundaryPoint(paneCue.angle, CHIP_WIDTH, trackHeight);
    return {
      direction: paneCue.direction,
      angle: paneCue.angle,
      x: point.x,
      y: point.y,
    };
  }

  function capsuleBoundaryPoint(angle, width, height) {
    const halfWidth = width / 2;
    const halfHeight = height / 2;
    const radius = halfHeight;
    const innerHalfWidth = Math.max(0, halfWidth - radius);
    const cosine = Math.cos(angle);
    const sine = Math.sin(angle);
    let inside = 0;
    let outside = Math.hypot(halfWidth, halfHeight) + radius;
    for (let index = 0; index < 24; index += 1) {
      const distance = (inside + outside) / 2;
      const x = Math.abs(cosine * distance) - innerHalfWidth;
      const y = Math.abs(sine * distance);
      const outsideX = Math.max(x, 0);
      const outsideY = Math.max(y, 0);
      const signedDistance = Math.hypot(outsideX, outsideY) + Math.min(Math.max(x, y), 0) - radius;
      if (signedDistance <= 0) inside = distance;
      else outside = distance;
    }
    return {
      x: Math.round((halfWidth + cosine * inside) * 10) / 10,
      y: Math.round((halfHeight + sine * inside) * 10) / 10,
    };
  }

  function chatRoot() {
    return resolveActiveThreadRoot();
  }

  function elementCenter(rect) {
    if (!rect) return { x: 0, y: 0 };
    return {
      x: rect.left + rect.width / 2,
      y: rect.top + rect.height / 2,
    };
  }

  function horizontalOverlapRatio(left, right) {
    if (!left || !right) return 0;
    const overlap = Math.max(0, Math.min(left.right, right.right) - Math.max(left.left, right.left));
    return overlap / Math.max(1, Math.min(left.width, right.width));
  }

  function ignoredComposerContainer(node, targetRoot = null) {
    if (!(node instanceof Element)) return true;
    if (state.root?.contains(node)) return true;
    const blockedAncestor = node.closest([
      `[${ROOT_ATTR}="true"]`,
      `[${PAYLOAD_ATTR}="true"]`,
      "nav",
      "[role='dialog']",
      "[aria-modal='true']",
      "[role='menu']",
      "[role='listbox']",
    ].join(","));
    if (blockedAncestor) return true;

    const activeRoot = targetRoot || chatRoot();
    if (activeRoot?.contains(node)) return false;

    const nodeAside = node.closest("aside");
    if (!nodeAside) return false;

    const activeAside = activeRoot?.closest("aside");
    return !(activeAside && nodeAside === activeAside);
  }

  function composerCandidateScore(node, rootRect, targetRoot = null) {
    const rect = visibleRect(node);
    if (!rect || !rootRect) return -Infinity;
    if (rect.width < 120 || rect.height < 20) return -Infinity;
    if (rect.bottom < window.innerHeight * 0.35) return -Infinity;
    if (ignoredComposerContainer(node, targetRoot)) return -Infinity;

    const overlap = horizontalOverlapRatio(rect, rootRect);
    const center = elementCenter(rect);
    const rootCenter = elementCenter(rootRect);
    const centerDrift = Math.abs(center.x - rootCenter.x) / Math.max(1, rootRect.width);
    const centerInsideRoot = center.x >= rootRect.left - 24 && center.x <= rootRect.right + 24;
    if (overlap < 0.45 && !centerInsideRoot) return -Infinity;

    const lowerScreen = rect.bottom / Math.max(1, window.innerHeight);
    const widthMatch = Math.min(rect.width, rootRect.width) / Math.max(1, Math.max(rect.width, rootRect.width));
    return overlap * 100 + lowerScreen * 24 + widthMatch * 18 - centerDrift * 48;
  }

  function mainComposerCandidate(candidates, targetRoot = null) {
    const root = targetRoot || chatRoot();
    const rootRect = visibleRect(root);
    const ranked = candidates
      .map((node) => ({ node, score: composerCandidateScore(node, rootRect, root) }))
      .filter((item) => Number.isFinite(item.score))
      .sort((left, right) => right.score - left.score);
    if (ranked[0]?.node) return ranked[0].node;

    if (targetRoot || threadRoots().length > 1) return null;

    const fallback = candidates
      .map((node) => ({ node, score: globalComposerCandidateScore(node) }))
      .filter((item) => Number.isFinite(item.score))
      .sort((left, right) => right.score - left.score)[0];
    if (fallback?.node) {
      pushDiagnostic("composer:global-fallback", {
        score: fallback.score,
        targetTag: fallback.node.tagName || "",
        targetRole: fallback.node.getAttribute?.("role") || "",
        targetClass: String(fallback.node.className || "").slice(0, 120),
        targetRect: rectSummary(fallback.node),
      });
    }
    return fallback?.node || null;
  }

  function globalComposerCandidateScore(node) {
    const rect = visibleRect(node);
    if (!rect || rect.width < 120 || rect.height < 20) return -Infinity;
    if (rect.bottom < window.innerHeight * 0.35 || ignoredComposerContainer(node)) return -Infinity;

    const label = normalizeText([
      node.getAttribute?.("aria-label"),
      node.getAttribute?.("placeholder"),
      node.getAttribute?.("data-placeholder"),
    ].filter(Boolean).join(" "));
    if (/search|find|查找|搜索/i.test(label)) return -Infinity;

    let score = rect.bottom / Math.max(1, window.innerHeight) * 40;
    score += Math.min(rect.width / Math.max(1, window.innerWidth), 1) * 20;
    if (node.matches?.("div.ProseMirror")) score += 160;
    if (node instanceof HTMLTextAreaElement) score += 130;
    if (node.getAttribute?.("role") === "textbox") score += 90;
    if (node.isContentEditable) score += 70;
    if (/message|prompt|send|ask|消息|输入|提问|发送/i.test(label)) score += 60;
    return score;
  }

  function composerCandidates(targetRoot = null) {
    const scope = targetRoot || document;
    return Array.from(
      scope.querySelectorAll(
        [
          "textarea",
          "[contenteditable='true']",
          "[role='textbox']",
          "div.ProseMirror",
        ].join(",")
      )
    ).filter((node) => {
      if (!(node instanceof HTMLElement)) return false;
      const rect = node.getBoundingClientRect();
      if (rect.width < 120 || rect.height < 20) return false;
      if (rect.bottom < window.innerHeight * 0.35) return false;
      if (targetRoot && threadRootOf(node) !== targetRoot) return false;
      if (ignoredComposerContainer(node, targetRoot)) return false;
      return true;
    });
  }

  function buttonLabel(node) {
    return normalizeText(node.getAttribute("aria-label") || node.getAttribute("title") || node.textContent || "");
  }

  function sendButtonLabel(label) {
    return /^(send message|send|add to queue|发送消息|发送|提交|加入队列|添加到队列)$/i.test(label);
  }

  function stopButtonLabel(label) {
    return /^(stop|停止)$/i.test(label);
  }

  function iconPathData(node) {
    return Array.from(node.querySelectorAll?.("svg path") || [])
      .map((path) => path.getAttribute("d") || "")
      .join("\n");
  }

  function stopButtonIcon(node) {
    const data = iconPathData(node);
    return /H14\.25C14\.9404 4\.5 15\.5 5\.05964 15\.5 5\.75V14\.25C15\.5 14\.9404/.test(data);
  }

  function stopButton(node) {
    return stopButtonLabel(buttonLabel(node)) || stopButtonIcon(node);
  }

  function disabledButton(node) {
    return Boolean(node.disabled || node.getAttribute("aria-disabled") === "true" || node.dataset.disabled === "true");
  }

  function submitButtonCandidate(button, containerRect) {
    const label = buttonLabel(button);
    if (stopButton(button)) return false;
    if (sendButtonLabel(label)) return true;
    if (label) return false;

    const rect = visibleRect(button);
    if (!rect || !containerRect) return false;
    const className = String(button.className || "");
    const compactIcon = rect.width >= 24 && rect.width <= 48 && rect.height >= 24 && rect.height <= 48;
    const composerIcon = className.includes("size-token-button-composer") || className.includes("bg-token-foreground");
    const lowerRight = rect.left > containerRect.left + containerRect.width * 0.58 &&
      rect.top > containerRect.top + containerRect.height * 0.42;
    return compactIcon && composerIcon && lowerRight;
  }

  function nearbySubmitButton(target, options = {}) {
    const includeDisabled = options.includeDisabled === true;
    const targetRoot = options.root || threadRootOf(target);
    let current = target?.parentElement || null;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      if (current === document.body || current === document.documentElement) break;
      if (state.root?.contains(current)) return null;
      if (targetRoot && !targetRoot.contains(current)) break;
      const buttons = Array.from(current.querySelectorAll("button,[role='button']"))
        .filter((node) => node instanceof HTMLElement && !state.root?.contains(node) && visibleElement(node) && (includeDisabled || !disabledButton(node)));

      const labeled = buttons.find((button) => sendButtonLabel(buttonLabel(button)));
      if (labeled) return labeled;

      const rect = visibleRect(current);
      if (rect && rect.width > 260 && rect.height > 52) {
        const lowerRight = buttons
          .filter((button) => !stopButton(button))
          .filter((button) => submitButtonCandidate(button, rect))
          .sort((a, b) => b.getBoundingClientRect().right - a.getBoundingClientRect().right);
        if (lowerRight.length) return lowerRight[0];
      }
    }
    return null;
  }

  function chatSurfaceReady() {
    if (!chatRoot()) return false;
    return !chatBusy();
  }

  function chatBusy() {
    const root = chatRoot();
    if (!root) return false;

    return Array.from(root.querySelectorAll("button,[role='button']")).some((node) => {
      if (!visibleElement(node)) return false;
      const label = normalizeText(node.getAttribute("aria-label") || node.textContent || "");
      return /^(停止|stop)$/i.test(label);
    });
  }

  function setScanStatus(status, details = {}) {
    const key = `${status}:${JSON.stringify(details)}`;
    state.scanStatus = status;
    state.scanBusy = status === "manual-refresh-busy" || Boolean(details.busy);
    if (state.lastScanStatus === key) return false;
    state.lastScanStatus = key;
    pushDiagnostic(`scan:${status}`, details);
    return true;
  }

  function composerBusy(target, options = {}) {
    const targetRoot = options.root || threadRootOf(target);
    let hasStopButton = false;
    let current = target?.parentElement || null;
    for (let depth = 0; current && depth < 8; depth += 1, current = current.parentElement) {
      if (current === document.body || current === document.documentElement) break;
      if (state.root?.contains(current)) return false;
      if (targetRoot && !targetRoot.contains(current)) break;
      const buttons = Array.from(current.querySelectorAll("button,[role='button']"))
        .filter((node) => node instanceof HTMLElement && visibleElement(node));
      if (buttons.some((node) => !disabledButton(node) && sendButtonLabel(buttonLabel(node)))) return false;
      if (buttons.some((node) => stopButton(node))) hasStopButton = true;
    }
    return hasStopButton;
  }

  // Message discovery tolerates ChatGPT's changing DOM while preferring semantic role and action-row signals.
  function messageCandidates() {
    const root = chatRoot();
    if (!root) return [];

    const selectors = [
      "[data-message-author-role]",
      "[data-thread-find-target]",
      "[data-testid*='message' i]",
      "[data-test-id*='message' i]",
      "article",
    ].join(",");

    return Array.from(root.querySelectorAll(selectors))
      .map((node) => ({
        node,
        role: roleFromElement(node),
        text: elementText(node),
      }))
      .filter((item) => item.text.length > 8);
  }

  function actionButton(node) {
    const label = normalizeText(node.getAttribute("aria-label") || node.textContent || "");
    return /^(复制|喜欢|不喜欢|从此处开始分叉|挂钩|copy|like|dislike|fork)/i.test(label);
  }

  function classTokenMatch(node, token) {
    return node instanceof Element && Array.from(node.classList || []).some((className) => className === token);
  }

  function assistantBubbleCandidates() {
    const root = chatRoot();
    if (!root) return [];

    return Array.from(root.querySelectorAll(".group.flex.min-w-0.flex-col"))
      .filter((node) => {
        if (!(node instanceof HTMLElement)) return false;
        if (state.root?.contains(node)) return false;
        if (classTokenMatch(node, "items-end")) return false;
        const text = directText(node);
        if (text.length < 24 || text.length > MAX_TEXT_LENGTH) return false;
        return true;
      })
      .map((node) => ({
        node,
        role: "assistant",
        text: elementText(node),
      }));
  }

  function roleFromMessageLabel(label) {
    const text = normalizeText(label?.textContent || "");
    if (/^(你说|you said|user)\s*[:：]?$/i.test(text)) return "user";
    if (/^(ChatGPT|assistant|codex)(?:\s+说|\s+said)?\s*[:：]?$/i.test(text)) return "assistant";
    return "";
  }

  function labeledMessageContainer(turn, role) {
    if (!(turn instanceof Element)) return null;
    const labels = Array.from(turn.querySelectorAll("h4.sr-only"));
    for (let index = labels.length - 1; index >= 0; index -= 1) {
      const label = labels[index];
      if (roleFromMessageLabel(label) !== role) continue;
      const container = label.parentElement;
      if (!(container instanceof Element)) continue;
      if (role === "user" && !classTokenMatch(container, "items-end")) continue;
      if (role === "assistant" && !classTokenMatch(container, "group")) continue;
      return container;
    }
    return null;
  }

  function labeledMessageText(container) {
    if (!(container instanceof Element)) return "";
    const clone = stripOwnUi(container.cloneNode(true));
    clone.querySelectorAll?.("h4.sr-only,button,[role='button'],svg").forEach((item) => item.remove());
    return normalizeText(clone.textContent || "");
  }

  function conversationTurn(turn) {
    if (!(turn instanceof Element)) return null;
    const turnKey = normalizeText(turn.getAttribute("data-content-search-turn-key") || "");
    const userNode = labeledMessageContainer(turn, "user");
    const assistantNode = labeledMessageContainer(turn, "assistant");
    const userText = labeledMessageText(userNode);
    const assistantText = labeledMessageText(assistantNode);
    return {
      node: turn,
      turnKey,
      userText,
      assistantMessage: assistantText.length > 8 ? {
        node: assistantNode,
        role: "assistant",
        text: assistantText,
        turnKey,
      } : null,
    };
  }

  function conversationTurns() {
    const root = chatRoot();
    if (!root) return [];
    return Array.from(root.querySelectorAll(CONVERSATION_TURN_SELECTOR))
      .map(conversationTurn)
      .filter(Boolean);
  }

  function compareConversationTurnKeys(left, right) {
    if (left === right) return 0;
    return left < right ? -1 : 1;
  }

  function latestConversationTurnByKey(turns) {
    return turns.reduce((latest, turn) => {
      if (!turn?.turnKey) return latest;
      if (!latest || compareConversationTurnKeys(latest.turnKey, turn.turnKey) < 0) return turn;
      return latest;
    }, null);
  }

  function nextLatestTurnAnchor(previous, turns, sessionId) {
    const mounted = latestConversationTurnByKey(turns);
    if (!mounted) return previous;
    const sameSession = Boolean(sessionId) && previous?.sessionId === sessionId;
    if (sameSession && compareConversationTurnKeys(mounted.turnKey, previous.turnKey) < 0) return previous;

    const sameTurn = sameSession && previous?.turnKey === mounted.turnKey;
    const assistant = mounted.assistantMessage;
    return {
      sessionId,
      turnKey: mounted.turnKey,
      userText: mounted.userText || (sameTurn ? previous.userText : ""),
      assistantText: assistant?.text || (sameTurn ? previous.assistantText : ""),
      turnNode: mounted.node || (sameTurn ? previous.turnNode : null),
      assistantNode: assistant?.node || (sameTurn ? previous.assistantNode : null),
    };
  }

  function assistantMessageFromTurnAnchor(anchor) {
    if (!anchor?.assistantText || anchor.assistantText.length <= 8) return null;
    return {
      node: anchor.assistantNode,
      role: "assistant",
      text: anchor.assistantText,
      turnKey: anchor.turnKey,
      userText: anchor.userText,
      turnNode: anchor.turnNode,
    };
  }

  function updateLatestTurnAnchor(turns) {
    state.latestTurnAnchor = nextLatestTurnAnchor(
      state.latestTurnAnchor,
      turns,
      state.activeContext.sessionId,
    );
    return state.latestTurnAnchor;
  }

  function latestMessageByDocumentOrder(candidates) {
    return candidates
      .filter((item) => item?.node instanceof Node && item.text?.length > 8)
      .sort((left, right) => {
        if (left.node === right.node) return 0;
        const position = left.node.compareDocumentPosition(right.node);
        if (position & Node.DOCUMENT_POSITION_FOLLOWING) return -1;
        if (position & Node.DOCUMENT_POSITION_PRECEDING) return 1;
        if (left.node.contains(right.node)) return -1;
        if (right.node.contains(left.node)) return 1;
        return 0;
      })
      .at(-1) || null;
  }

  function actionRowForMessage(root) {
    const buttons = Array.from(root.querySelectorAll("button,[role='button']")).filter(actionButton);
    for (const button of buttons) {
      let current = button.parentElement;
      for (let depth = 0; current && depth < 5; depth += 1, current = current.parentElement) {
        const rect = visibleRect(current);
        if (!rect || rect.height > 96) continue;
        const count = Array.from(current.querySelectorAll("button,[role='button']")).filter(actionButton).length;
        if (count >= 2) return current;
      }
    }
    return null;
  }

  function containsActionRow(node) {
    return Boolean(node && actionRowForMessage(node));
  }

  function assistantContainerForActionRow(actionRow) {
    let current = actionRow?.parentElement;

    for (let depth = 0; current && depth < 7; depth += 1, current = current.parentElement) {
      const text = directText(current);
      if (text.length < 24) continue;
      if (text.length > MAX_TEXT_LENGTH) continue;
      if (!containsActionRow(current)) continue;
      return current;
    }

    return null;
  }

  function allActionRows() {
    const root = chatRoot();
    if (!root) return [];

    const rows = [];
    const seen = new Set();
    const buttons = Array.from(root.querySelectorAll("button,[role='button']")).filter(actionButton);

    for (const button of buttons) {
      let current = button.parentElement;
      for (let depth = 0; current && depth < 5; depth += 1, current = current.parentElement) {
        if (seen.has(current)) continue;
        if (!visibleElement(current)) continue;
        const rect = visibleRect(current);
        if (!rect || rect.height > 96) continue;
        const count = Array.from(current.querySelectorAll("button,[role='button']")).filter(actionButton).length;
        if (count < 2) continue;
        seen.add(current);
        rows.push(current);
        break;
      }
    }

    return rows;
  }

  function findLatestAssistantMessage() {
    const turns = conversationTurns();
    if (turns.length || state.latestTurnAnchor) {
      return assistantMessageFromTurnAnchor(updateLatestTurnAnchor(turns));
    }

    const candidates = [];
    const rows = allActionRows();
    for (let index = 0; index < rows.length; index += 1) {
      const node = assistantContainerForActionRow(rows[index]);
      const text = elementText(node);
      if (text.length > 8) candidates.push({ node, role: "assistant", text });
    }

    candidates.push(...messageCandidates().filter((item) => item.role === "assistant"));
    candidates.push(...assistantBubbleCandidates());
    return latestMessageByDocumentOrder(candidates);
  }

  function findPreviousUserText(message) {
    const snapshotUserText = normalizeText(message?.userText || "");
    if (snapshotUserText) return shortText(snapshotUserText, 2000);

    const assistantNode = message?.node || message;
    const turn = assistantNode?.closest?.(CONVERSATION_TURN_SELECTOR);
    const turnUserText = conversationTurn(turn)?.userText || "";
    if (turnUserText) return shortText(turnUserText, 2000);

    const candidates = messageCandidates();
    const before = candidates.filter((item) => {
      if (item.node === assistantNode) return false;
      if (!(item.node instanceof Node) || !(assistantNode instanceof Node)) return false;
      return Boolean(item.node.compareDocumentPosition(assistantNode) & Node.DOCUMENT_POSITION_FOLLOWING);
    });

    for (let cursor = before.length - 1; cursor >= 0; cursor -= 1) {
      const item = before[cursor];
      if (item.role === "user") return shortText(item.text, 2000);
      if (/^(user|you)\b/i.test(item.text)) return shortText(item.text, 2000);
    }
    return "";
  }
