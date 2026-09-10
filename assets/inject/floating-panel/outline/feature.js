/* Answer Outline lifecycle: current-answer binding, refresh, and invalidation. */

  function outlineBuild(message, sourceHash) {
    if (!message?.node) {
      outlineClearMarks();
      return { items: [], fingerprint: sourceHash || "", message: null };
    }
    const textLength = message.text.length;
    const raw = outlineCollectHeadingElements(outlineMarkdownRoot(message.node));
    const items = outlineNormalizeDisplayLevels(outlineDedupeItems(raw));
    const structuredEnough = items.length >= Math.max(MIN_OUTLINE_ITEMS, 3) && textLength >= 160;
    outlineClearMarks();
    if (textLength < MIN_OUTLINE_TEXT_LEN && !structuredEnough || items.length < MIN_OUTLINE_ITEMS) {
      return { items: [], fingerprint: `${sourceHash}|empty`, message: message.node };
    }
    outlineMarkItems(items);
    return {
      items,
      fingerprint: `${sourceHash}|${hashText(items.map((item) => `${item.level}:${item.text}`).join("|"))}`,
      message: message.node,
    };
  }

  function invalidateOutline(message = null, sourceHash = "") {
    outlineClearMarks();
    state.outlineItems = [];
    state.outlineStatus = chatBusy() ? "pending" : "idle";
    state.outlineError = "";
    state.outlineFingerprint = "";
    state.outlineSourceHash = "";
    state.outlineMessage = message?.node || null;
    if (state.activeTab === "outline" && state.panel) renderFloat({ preserveMorph: true });
  }

  // Outline refresh is keyed to the pinned latest answer, so passive scrolling cannot switch context.
  async function refreshOutline(options = {}) {
    if (!isCurrentRuntime() || !outlineEnabled()) return;
    if (state.outlineRefreshPromise) return state.outlineRefreshPromise;
    const requestContext = contextSnapshot();
    const requestEpoch = state.outlineEpoch;
    const requestCurrent = () => outlineEnabled()
      && requestEpoch === state.outlineEpoch
      && contextMatches(requestContext);
    state.outlineStatus = "pending";
    state.outlineError = "";
    if (state.activeTab === "outline") renderFloat({ preserveMorph: true });

    const task = Promise.resolve().then(() => {
      if (!requestCurrent()) return;
      const message = options.message || findLatestAssistantMessage();
      const sourceHash = options.assistantHash || hashText(message?.text || "");
      if (chatBusy()) {
        state.outlineError = "回答尚未完成，完成后再试";
        state.outlineStatus = "pending";
        scheduleScan(STREAM_IDLE_MS);
        return;
      }
      const result = outlineBuild(message, sourceHash);
      if (!requestCurrent()) return;
      state.outlineItems = result.items;
      state.outlineFingerprint = result.fingerprint;
      state.outlineSourceHash = sourceHash;
      state.outlineMessage = result.message;
      state.outlineStatus = result.items.length ? "ready" : "empty";
      state.outlineError = "";
    }).catch((error) => {
      if (!requestCurrent()) return;
      outlineClearMarks();
      state.outlineItems = [];
      state.outlineStatus = "error";
      state.outlineError = error?.message || "大纲暂不可用";
    }).finally(() => {
      if (!requestCurrent()) return;
      if (state.outlineRefreshPromise === task) state.outlineRefreshPromise = null;
      if (state.activeTab === "outline") renderFloat({ preserveMorph: true });
    });
    state.outlineRefreshPromise = task;
    return task;
  }
