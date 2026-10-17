  // 提示词增强：把输入框草稿交给后端模型改写，预览确认后才写回；取消时输入框保持原样。
  // 输入框定位与 footer 选择复用 93-dictation，两个入口挂在同一个 composer footer 上。
  window.__codexPlusPromptEnhanceRuntime?.dispose?.();
  const codexPlusPromptEnhance = { phase: "idle", revision: 0, target: null, text: "", error: "" };
  registerCodexPlusExtensionSelector('[data-codex-plus-ext="builtin-prompt-enhance"]');

  function codexPlusPromptEnhanceEnabled() {
    return codexPlusBackendSettingsLoaded && codexPlusBackendSettings.enhancementsEnabled !== false
      && codexPlusBackendSettings.promptEnhance?.enabled === true;
  }

  function codexPlusPromptEnhanceLeafText(node) {
    const name = node.type?.name || "";
    if (name === "hard_break" || name === "hardBreak") return "\n";
    const attrs = node.attrs || {};
    return String(attrs.label || attrs.name || attrs.path || attrs.text || "");
  }

  // 只有段落、换行和纯文本时才允许整段替换；@文件、技能、附件这类节点替换后会丢。
  function codexPlusPromptEnhanceRich(view) {
    let rich = false;
    view.state.doc.descendants((node) => {
      if (rich) return false;
      if (node.isText || ["paragraph", "hard_break", "hardBreak"].includes(node.type?.name)) return true;
      rich = true;
      return false;
    });
    return rich;
  }

  function codexPlusPromptEnhanceRead(editor) {
    const view = codexPlusDictationView(editor);
    if (view) {
      const { doc } = view.state;
      return { text: doc.textBetween(0, doc.content.size, "\n", codexPlusPromptEnhanceLeafText), rich: codexPlusPromptEnhanceRich(view) };
    }
    if (editor instanceof HTMLTextAreaElement) return { text: editor.value, rich: false };
    // 认不出结构的编辑器不做整段替换，只允许插入到末尾和复制。
    return { text: editor.innerText || editor.textContent || "", rich: true };
  }

  function codexPlusPromptEnhanceParagraphs(schema, text) {
    const paragraph = schema?.nodes?.paragraph;
    if (!paragraph || typeof schema.text !== "function") return null;
    return text.split("\n").map((line) => paragraph.create(null, line ? schema.text(line) : null));
  }

  // replace 覆盖整段草稿，append 追加到末尾。ProseMirror 下都是一次事务，Ctrl+Z 能整体撤回。
  function codexPlusPromptEnhanceWrite(editor, text, mode) {
    const value = String(text || "").replace(/\r\n?/g, "\n").trim();
    if (!value || !editor?.isConnected || editor.matches?.(':disabled, [readonly], [contenteditable="false"]')
        || editor.closest?.('[inert], [aria-hidden="true"]')) return false;
    editor.focus();
    const view = codexPlusDictationView(editor);
    if (view) {
      const { state } = view;
      const end = state.doc.content.size;
      const SelectionType = state.selection?.constructor;
      const atEnd = (doc) => typeof SelectionType?.atEnd === "function" ? SelectionType.atEnd(doc) : null;
      const nodes = codexPlusPromptEnhanceParagraphs(state.schema, value);
      let tr = state.tr;
      if (nodes) tr = mode === "replace" ? tr.replaceWith(0, end, nodes) : tr.insert(end, nodes);
      else if (mode === "replace") tr = tr.insertText(value, 0, end);
      else {
        const position = atEnd(state.doc)?.from ?? end;
        tr = tr.insertText(`\n${value}`, position, position);
      }
      const selection = atEnd(tr.doc);
      if (selection) tr = tr.setSelection(selection);
      view.dispatch(tr.scrollIntoView());
      return true;
    }
    if (editor instanceof HTMLTextAreaElement) {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
      if (!setter) return false;
      const current = editor.value.replace(/\s+$/, "");
      const next = mode === "replace" ? value : `${current}${current ? "\n\n" : ""}${value}`;
      setter.call(editor, next);
      editor.setSelectionRange(next.length, next.length);
      editor.dispatchEvent(new Event("input", { bubbles: true }));
      return true;
    }
    if (mode === "replace") return false;
    const selection = window.getSelection?.();
    if (!selection || typeof document.createRange !== "function") return false;
    const range = document.createRange();
    range.selectNodeContents(editor);
    range.collapse(false);
    selection.removeAllRanges();
    selection.addRange(range);
    return document.execCommand("insertText", false, `\n\n${value}`) === true;
  }

  function codexPlusPromptEnhanceTargetAlive() {
    const target = codexPlusPromptEnhance.target;
    return !!target?.editor?.isConnected && window.location.href === target.href;
  }

  // 返回不能整段替换的原因，空串表示可以替换。
  function codexPlusPromptEnhanceReplaceBlocker() {
    const target = codexPlusPromptEnhance.target;
    if (!codexPlusPromptEnhanceTargetAlive()) return "输入框或会话已变化，请复制结果后手动粘贴";
    if (target.rich) return "草稿里有 @文件、技能或附件，整段替换会丢掉它们，可以插入到末尾或复制";
    if (codexPlusPromptEnhanceRead(target.editor).text !== target.original) return "生成期间草稿有改动，为免覆盖，请插入到末尾或复制";
    return "";
  }

  async function requestCodexPlusPromptEnhance() {
    const target = codexPlusPromptEnhance.target;
    if (!target) return;
    const revision = ++codexPlusPromptEnhance.revision;
    Object.assign(codexPlusPromptEnhance, { phase: "loading", text: "", error: "" });
    renderCodexPlusPromptEnhance();
    let result;
    try {
      // app:// 的 CSP 不允许直接请求模型服务，经特权桥接交给 Rust，API Key 不进页面。
      result = await postJson("/prompt-enhance/generate", { prompt: target.original });
    } catch (error) {
      result = { status: "failed", message: error?.message };
    }
    if (revision !== codexPlusPromptEnhance.revision) return;
    const text = result?.status === "ok" && typeof result.text === "string" ? result.text.trim() : "";
    if (text) Object.assign(codexPlusPromptEnhance, { phase: "result", text });
    else Object.assign(codexPlusPromptEnhance, { phase: "error", error: result?.message || "提示词增强失败" });
    renderCodexPlusPromptEnhance();
  }

  async function startCodexPlusPromptEnhance(footer) {
    if (!codexPlusPromptEnhanceEnabled() || codexPlusPromptEnhance.phase === "loading") return;
    const scope = codexPlusDictationScope(footer);
    const editor = scope && codexPlusDictationEditor(scope);
    if (!editor) { showToast("无法找到可编辑的 Codex 输入框", null); return; }
    const { text, rich } = codexPlusPromptEnhanceRead(editor);
    if (!text.trim()) { showToast("先在输入框写下需求，再点增强", null); return; }
    codexPlusPromptEnhance.target = { editor, href: window.location.href, original: text, rich };
    await requestCodexPlusPromptEnhance();
  }

  function closeCodexPlusPromptEnhance() {
    ++codexPlusPromptEnhance.revision;
    const editor = codexPlusPromptEnhance.target?.editor;
    Object.assign(codexPlusPromptEnhance, { phase: "idle", target: null, text: "", error: "" });
    renderCodexPlusPromptEnhance();
    if (editor?.isConnected) editor.focus?.();
  }

  function applyCodexPlusPromptEnhance(mode, text) {
    if (!String(text || "").trim()) { showToast("结果为空，没有可写入的内容", null); return false; }
    if (!codexPlusPromptEnhanceTargetAlive()) { showToast("输入框或会话已变化，请复制结果后手动粘贴", null); return false; }
    if (mode === "replace") {
      const blocker = codexPlusPromptEnhanceReplaceBlocker();
      if (blocker) { showToast(blocker, null); return false; }
    }
    if (!codexPlusPromptEnhanceWrite(codexPlusPromptEnhance.target.editor, text, mode)) {
      showToast("写入输入框失败，请复制结果后手动粘贴", null);
      return false;
    }
    closeCodexPlusPromptEnhance();
    return true;
  }

  function codexPlusPromptEnhanceButton(action, label, options = {}) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.dataset.promptEnhanceAction = action;
    if (options.primary) button.className = "is-primary";
    if (options.disabled) button.disabled = true;
    if (options.title) button.title = options.title;
    return button;
  }

  function positionCodexPlusPromptEnhancePanel(panel) {
    const editor = codexPlusPromptEnhance.target?.editor;
    const anchor = editor?.isConnected ? editor.closest?.("[data-codex-composer-root], [data-composer-surface-variant], form") || editor : null;
    const rect = anchor?.getBoundingClientRect?.();
    const width = Math.min(Math.max(rect?.width || 480, 360), 760, window.innerWidth - 32);
    if (!rect?.width) {
      Object.assign(panel.style, { left: `${Math.max(16, window.innerWidth - width - 24)}px`, width: `${width}px`, top: "", bottom: "24px", maxHeight: "" });
      return;
    }
    const left = Math.min(Math.max(16, rect.left), Math.max(16, window.innerWidth - width - 16));
    const above = rect.top - 16;
    if (above >= 220) {
      Object.assign(panel.style, { left: `${left}px`, width: `${width}px`, top: "", bottom: `${window.innerHeight - rect.top + 8}px`, maxHeight: `${above}px` });
    } else {
      Object.assign(panel.style, { left: `${left}px`, width: `${width}px`, bottom: "", top: `${rect.bottom + 8}px`, maxHeight: `${Math.max(200, window.innerHeight - rect.bottom - 24)}px` });
    }
  }

  function createCodexPlusPromptEnhancePanel() {
    const panel = document.createElement("div");
    panel.id = "codex-plus-prompt-enhance-panel";
    panel.dataset.codexPlusExt = "builtin-prompt-enhance";
    panel.setAttribute("role", "dialog");
    panel.setAttribute("aria-label", "提示词增强");
    panel.addEventListener("click", (event) => {
      const action = event.target.closest?.("[data-prompt-enhance-action]")?.dataset.promptEnhanceAction;
      if (!action) return;
      event.preventDefault();
      event.stopPropagation();
      const text = panel.querySelector("textarea")?.value ?? codexPlusPromptEnhance.text;
      if (action === "replace" || action === "append") applyCodexPlusPromptEnhance(action, text);
      else if (action === "retry") void requestCodexPlusPromptEnhance();
      else if (action === "copy") {
        if (!navigator.clipboard?.writeText) { showToast("请选中结果手动复制", null); return; }
        void navigator.clipboard.writeText(text).then(() => showToast("已复制增强结果", null))
          .catch(() => showToast("复制失败，请选中结果手动复制", null));
      } else closeCodexPlusPromptEnhance();
    });
    panel.addEventListener("keydown", (event) => {
      if (event.key !== "Enter" || event.isComposing || !(event.ctrlKey || event.metaKey) || event.target?.tagName !== "TEXTAREA") return;
      event.preventDefault();
      event.stopPropagation();
      applyCodexPlusPromptEnhance(codexPlusPromptEnhanceReplaceBlocker() ? "append" : "replace", event.target.value);
    });
    return panel;
  }

  function renderCodexPlusPromptEnhance() {
    const state = codexPlusPromptEnhance;
    document.querySelectorAll("[data-prompt-enhance-trigger]").forEach((button) => {
      button.disabled = state.phase === "loading";
      button.setAttribute("aria-busy", String(state.phase === "loading"));
    });
    let panel = document.getElementById("codex-plus-prompt-enhance-panel");
    if (state.phase === "idle") { panel?.remove(); return; }
    if (!panel) { panel = createCodexPlusPromptEnhancePanel(); document.body.append(panel); }
    positionCodexPlusPromptEnhancePanel(panel);
    const key = `${state.revision}:${state.phase}`;
    if (panel.dataset.renderKey === key) return;
    panel.dataset.renderKey = key;
    const title = document.createElement("strong");
    title.textContent = "提示词增强";
    const status = document.createElement("span");
    status.setAttribute("role", "status");
    const actions = document.createElement("div");
    actions.className = "codex-plus-prompt-enhance-actions";
    const children = [title, status];
    if (state.phase === "loading") {
      status.textContent = "正在优化…";
      actions.append(codexPlusPromptEnhanceButton("close", "取消"));
    } else if (state.phase === "error") {
      status.textContent = state.error;
      status.className = "is-error";
      actions.append(codexPlusPromptEnhanceButton("retry", "重试", { primary: true }), codexPlusPromptEnhanceButton("close", "关闭"));
    } else {
      const blocker = codexPlusPromptEnhanceReplaceBlocker();
      status.textContent = "可以先修改，确认后再写回输入框";
      const text = document.createElement("textarea");
      text.value = state.text;
      text.spellcheck = false;
      text.setAttribute("aria-label", "增强后的提示词");
      children.push(text);
      if (blocker) {
        const note = document.createElement("p");
        note.className = "codex-plus-prompt-enhance-note";
        note.textContent = blocker;
        children.push(note);
      }
      actions.append(
        codexPlusPromptEnhanceButton("replace", "替换", { primary: !blocker, disabled: !!blocker, title: blocker || "Ctrl+Enter" }),
        codexPlusPromptEnhanceButton("append", "插入到末尾", { primary: !!blocker }),
        codexPlusPromptEnhanceButton("copy", "复制"),
        codexPlusPromptEnhanceButton("retry", "重新生成"),
        codexPlusPromptEnhanceButton("close", "取消"),
      );
      requestAnimationFrame(() => { if (text.isConnected) text.focus(); });
    }
    children.push(actions);
    panel.replaceChildren(...children);
  }

  function codexPlusPromptEnhanceIcon() {
    const ns = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", "0 0 16 16");
    svg.setAttribute("width", "16"); svg.setAttribute("height", "16");
    svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "1.3"); svg.setAttribute("stroke-linejoin", "round");
    svg.setAttribute("aria-hidden", "true");
    ["M6.5 2.5 7.6 5.9 11 7 7.6 8.1 6.5 11.5 5.4 8.1 2 7 5.4 5.9Z", "M12 9.5l.6 1.4 1.4.6-1.4.6-.6 1.4-.6-1.4-1.4-.6 1.4-.6Z"].forEach((d) => {
      const path = document.createElementNS(ns, "path");
      path.setAttribute("d", d);
      svg.appendChild(path);
    });
    return svg;
  }

  function createCodexPlusPromptEnhanceTrigger() {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "codex-plus-prompt-enhance-trigger";
    button.dataset.codexPlusExt = "builtin-prompt-enhance";
    button.dataset.promptEnhanceTrigger = "true";
    button.setAttribute("aria-label", "增强提示词");
    button.title = "增强提示词（预览后再写回）";
    button.append(codexPlusPromptEnhanceIcon());
    // 保住输入框焦点和选区，读取草稿时才是用户正在编辑的那个输入框。
    button.addEventListener("pointerdown", (event) => event.preventDefault());
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      void startCodexPlusPromptEnhance(button.closest(".composer-footer, [data-composer-footer-responsive]"));
    });
    return button;
  }

  function installCodexPlusPromptEnhance() {
    if (!codexPlusPromptEnhanceEnabled()) {
      if (codexPlusPromptEnhance.phase !== "idle") closeCodexPlusPromptEnhance();
      document.querySelectorAll('[data-codex-plus-ext="builtin-prompt-enhance"]').forEach((node) => node.remove());
      return;
    }
    if (!document.getElementById("codex-plus-prompt-enhance-style")) {
      const style = document.createElement("style");
      style.id = "codex-plus-prompt-enhance-style";
      style.dataset.codexPlusExt = "builtin-prompt-enhance";
      style.textContent = `.codex-plus-prompt-enhance-trigger{display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;padding:0;margin-inline:2px;border:0;border-radius:8px;background:transparent;color:var(--color-token-text-primary,currentColor);opacity:.9;cursor:pointer}
        .codex-plus-prompt-enhance-trigger:hover{background:color-mix(in srgb,currentColor 12%,transparent);opacity:1}
        .codex-plus-prompt-enhance-trigger:disabled{opacity:.4;cursor:default;background:transparent}
        .codex-plus-prompt-enhance-trigger:focus-visible,#codex-plus-prompt-enhance-panel button:focus-visible{outline:2px solid currentColor;outline-offset:2px}
        #codex-plus-prompt-enhance-panel{position:fixed;z-index:2147483000;display:flex;flex-direction:column;gap:8px;box-sizing:border-box;padding:12px;overflow:auto;border:1px solid var(--color-token-border-default,#8884);border-radius:12px;background:var(--color-token-bg-primary,var(--background,#252525));color:var(--color-token-text-primary,var(--foreground,#eee));box-shadow:0 8px 28px #0005;font-size:13px}
        #codex-plus-prompt-enhance-panel [role="status"]{color:var(--color-token-text-secondary,inherit)}
        #codex-plus-prompt-enhance-panel [role="status"].is-error{color:#e5484d;white-space:pre-wrap;word-break:break-word}
        #codex-plus-prompt-enhance-panel textarea{box-sizing:border-box;width:100%;min-height:120px;max-height:50vh;resize:vertical;padding:8px;border:1px solid var(--color-token-border-light,#8883);border-radius:8px;background:var(--color-token-bg-secondary,transparent);color:inherit;font:inherit;line-height:1.5}
        .codex-plus-prompt-enhance-note{margin:0;color:var(--color-token-text-secondary,inherit);font-size:12px}
        .codex-plus-prompt-enhance-actions{display:flex;flex-wrap:wrap;gap:6px;justify-content:flex-end}
        .codex-plus-prompt-enhance-actions button{padding:5px 10px;border:1px solid color-mix(in srgb,currentColor 20%,transparent);border-radius:8px;background:transparent;color:inherit;cursor:pointer;white-space:nowrap}
        .codex-plus-prompt-enhance-actions button.is-primary{background:var(--color-token-text-primary,#eee);color:var(--color-token-bg-primary,#222);border-color:transparent}
        .codex-plus-prompt-enhance-actions button:disabled{opacity:.45;cursor:default}`;
      document.head.append(style);
    }
    // 与语音入口同样的去重：一个输入框只挂一个按钮，优先挂在包含输入框的 footer 上。
    const hosts = new Map();
    document.querySelectorAll(".composer-footer, [data-composer-footer-responsive]").forEach((footer) => {
      if (!visibleElement(footer) || isExtensionUiNode(footer)) return;
      const scope = codexPlusDictationScope(footer);
      const editor = scope && codexPlusDictationEditor(scope);
      if (!editor) return;
      const score = codexPlusDictationFooterScore(footer, editor);
      const current = hosts.get(editor);
      if (!current || score >= current.score) hosts.set(editor, { footer, score });
    });
    const footers = new Set([...hosts.values()].map((item) => item.footer));
    document.querySelectorAll("[data-prompt-enhance-trigger]").forEach((node) => {
      if (!footers.has(node.closest(".composer-footer, [data-composer-footer-responsive]"))) node.remove();
    });
    for (const footer of footers) {
      const existing = footer.querySelector("[data-prompt-enhance-trigger]");
      if (existing?.closest(".composer-footer, [data-composer-footer-responsive]") === footer) continue;
      const trigger = createCodexPlusPromptEnhanceTrigger();
      trigger.disabled = codexPlusPromptEnhance.phase === "loading";
      footer.append(trigger);
    }
  }

  const codexPlusPromptEnhanceKeyHandler = (event) => {
    if (event.key !== "Escape" || event.isComposing || codexPlusPromptEnhance.phase === "idle") return;
    event.preventDefault();
    event.stopImmediatePropagation();
    closeCodexPlusPromptEnhance();
  };
  const codexPlusPromptEnhanceResize = () => {
    const panel = document.getElementById("codex-plus-prompt-enhance-panel");
    if (panel) positionCodexPlusPromptEnhancePanel(panel);
  };
  document.addEventListener("keydown", codexPlusPromptEnhanceKeyHandler, true);
  window.addEventListener("resize", codexPlusPromptEnhanceResize);
  const codexPlusPromptEnhanceDispose = () => {
    ++codexPlusPromptEnhance.revision;
    document.removeEventListener("keydown", codexPlusPromptEnhanceKeyHandler, true);
    window.removeEventListener("resize", codexPlusPromptEnhanceResize);
    document.querySelectorAll('[data-codex-plus-ext="builtin-prompt-enhance"]').forEach((node) => node.remove());
  };
  window.addEventListener("pagehide", codexPlusPromptEnhanceDispose, { once: true });
  window.__codexPlusPromptEnhanceRuntime = { dispose: () => {
    window.removeEventListener("pagehide", codexPlusPromptEnhanceDispose);
    codexPlusPromptEnhanceDispose();
  } };
