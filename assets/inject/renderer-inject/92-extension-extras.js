  /**
   * 拓展的输入触发、快捷键、主题与向用户提问。
   *
   * - 输入触发：registerInputTrigger({ trigger: "#", items(query) => [{ label, description?, insert }] })。
   *   @、$、/ 是 Codex 自己的提及与命令，不让拓展占用。
   *   输入框里光标前是「触发符 + 查询词」时弹出候选，选中后把这一段换成 insert。
   * - 快捷键：registerShortcut({ key: "Mod+Shift+K", run, allowInInput? })。Mod 在 macOS 是 ⌘，其余是 Ctrl。
   * - 主题：registerTheme({ id, name, variables: { "--x": "值" }, css })，同一时间只有一个生效，选择记在本机。
   * - 提问：codexPlusAsk({ title, message, options?, input? }) → 用户的回答，取消时为 null。后台经 CDP 也能调。
   */

  // ---------------------------------------------------------------- 输入触发
  const codexPlusTriggerPattern = /^[#%&~!^:+][\w-]{0,7}$/;
  /** Codex 自己占用的触发符：@ 提及文件 / 应用，$ 提及技能，/ 是斜杠命令。拓展抢这些只会和原生菜单打架。 */
  const codexPlusReservedTriggers = /^[@$/]/;
  let codexPlusTriggerState = null;

  // 「已装过」记在本份注入代码里（见 installCodexPlusCommandHandlers 的说明）。
  let codexPlusInputTriggerHandlersInstalled = false;
  let codexPlusShortcutHandlerInstalled = false;

  function codexPlusRegisterInputTrigger(definition, options = {}) {
    const trigger = String(definition?.trigger || "");
    if (codexPlusReservedTriggers.test(trigger)) throw new Error(`触发符 ${trigger[0]} 是 Codex 自己在用的（@ 提及、$ 技能、/ 命令），换一个，例如 "#"`);
    if (!codexPlusTriggerPattern.test(trigger)) throw new Error("trigger 是一个符号开头的短前缀，例如 \"#\"、\"%\"、\"#issue\"");
    if (typeof definition?.items !== "function") throw new Error(`输入触发 ${trigger} 需要 items(query) 返回候选`);
    const dispose = codexPlusRegisterExtensionItem("inputTrigger", codexPlusRegistry.inputTriggers, { ...definition, trigger }, options);
    installCodexPlusInputTriggerHandlers();
    return dispose;
  }

  /** 光标前的文字（只看光标所在的文本节点，够用且不受富文本结构影响）。 */
  function codexPlusTextBeforeCaret(editor) {
    const selection = window.getSelection();
    if (!selection || !selection.rangeCount || !selection.isCollapsed) return null;
    const node = selection.anchorNode;
    if (!node || !editor.contains(node) || node.nodeType !== Node.TEXT_NODE) return null;
    return node.textContent.slice(0, selection.anchorOffset);
  }

  function codexPlusMatchInputTrigger(before) {
    let best = null;
    for (const item of codexPlusExtensionItems(codexPlusRegistry.inputTriggers)) {
      const index = before.lastIndexOf(item.trigger);
      if (index < 0 || (index > 0 && !/\s/.test(before[index - 1]))) continue;
      const query = before.slice(index + item.trigger.length);
      if (/\s/.test(query) || query.length > 64) continue;
      if (!best || index > best.index || (index === best.index && item.trigger.length > best.item.trigger.length)) best = { item, index, query };
    }
    return best;
  }

  function hideCodexPlusTriggerHints() {
    codexPlusTriggerState = null;
    document.getElementById("codex-plus-input-trigger-hints")?.remove();
  }

  function codexPlusAcceptTrigger(choice) {
    const state = codexPlusTriggerState;
    if (!state || !choice) return;
    const { editor, match } = state;
    hideCodexPlusTriggerHints();
    editor.focus();
    const selection = window.getSelection();
    const length = match.item.trigger.length + match.query.length;
    for (let index = 0; index < length; index += 1) selection.modify("extend", "backward", "character");
    document.execCommand("insertText", false, String(choice.insert ?? choice.label ?? ""));
    if (typeof choice.onSelect === "function") runCodexPlusExtensionCallback(match.item.scriptKey, "inputTrigger.onSelect", () => choice.onSelect());
  }

  function renderCodexPlusTriggerHints() {
    const state = codexPlusTriggerState;
    if (!state) return;
    if (!state.items.length) {
      document.getElementById("codex-plus-input-trigger-hints")?.remove();
      return;
    }
    let panel = document.getElementById("codex-plus-input-trigger-hints");
    if (!panel) {
      panel = document.createElement("div");
      panel.id = "codex-plus-input-trigger-hints";
      panel.className = "codex-plus-hint-panel";
      panel.setAttribute("role", "listbox");
      markCodexPlusExtensionNode(panel, "builtin-input-triggers");
      document.body.appendChild(panel);
    }
    panel.textContent = "";
    state.items.forEach((choice, index) => {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "codex-plus-command-hint";
      row.setAttribute("role", "option");
      row.setAttribute("aria-selected", String(index === state.active));
      row.innerHTML = `<strong>${escapeHtml(String(choice.label ?? choice.insert ?? ""))}</strong><span>${escapeHtml(String(choice.description || ""))}</span>`;
      row.addEventListener("pointerdown", (event) => event.preventDefault());
      row.addEventListener("click", () => codexPlusAcceptTrigger(choice));
      panel.appendChild(row);
    });
    const rect = state.editor.getBoundingClientRect();
    panel.style.left = `${Math.round(rect.left)}px`;
    panel.style.bottom = `${Math.round(window.innerHeight - rect.top + 8)}px`;
    panel.style.minWidth = `${Math.round(Math.min(rect.width, 420))}px`;
  }

  async function updateCodexPlusTriggerHints(editor) {
    const before = codexPlusTextBeforeCaret(editor);
    const match = before === null ? null : codexPlusMatchInputTrigger(before);
    if (!match) {
      hideCodexPlusTriggerHints();
      return;
    }
    const token = Symbol("trigger");
    codexPlusTriggerState = { editor, match, items: codexPlusTriggerState?.items || [], active: 0, token };
    const result = runCodexPlusExtensionCallback(match.item.scriptKey, `inputTrigger.items:${match.item.trigger}`, () => match.item.items(match.query, { editor }));
    if (!result.ok) return hideCodexPlusTriggerHints();
    let items;
    try {
      items = await result.value;
    } catch (error) {
      codexPlusMarkExtensionFailure(match.item.scriptKey, `inputTrigger.items: ${String(error?.stack || error?.message || error)}`);
      return hideCodexPlusTriggerHints();
    }
    // 等候选期间用户又输入了：以最新一次为准。
    if (codexPlusTriggerState?.token !== token) return;
    codexPlusTriggerState.items = (Array.isArray(items) ? items : []).filter((choice) => choice && typeof choice === "object").slice(0, 12);
    renderCodexPlusTriggerHints();
  }

  function installCodexPlusInputTriggerHandlers() {
    // 共用命令提示的样式（命令提示的处理器同时装上也无妨：没有命令时它什么都不做）。
    installCodexPlusCommandHandlers();
    if (!document.getElementById("codex-plus-input-trigger-style")) {
      const style = document.createElement("style");
      style.id = "codex-plus-input-trigger-style";
      markCodexPlusExtensionNode(style, "builtin-input-triggers");
      style.textContent = `#codex-plus-input-trigger-hints{position:fixed;z-index:2147483645;display:flex;flex-direction:column;gap:2px;max-height:min(360px,50vh);overflow:auto;padding:6px;border-radius:12px;border:1px solid color-mix(in srgb,currentColor 14%,transparent);background:var(--color-token-main-surface-primary,var(--codex-plus-bg-elevated,#262626));color:var(--color-token-text-primary,var(--codex-plus-text,#eee));box-shadow:0 8px 24px rgba(0,0,0,.25);font-size:13px}
        #codex-plus-input-trigger-hints .codex-plus-command-hint[aria-selected="true"]{background:color-mix(in srgb,currentColor 12%,transparent)}`;
      document.head.append(style);
    }
    if (codexPlusInputTriggerHandlersInstalled) return;
    codexPlusInputTriggerHandlersInstalled = true;
    const onInput = (event) => {
      if (!codexPlusRegistry.inputTriggers.size) return;
      const editor = codexPlusComposerEditorFrom(event.target);
      if (editor) void updateCodexPlusTriggerHints(editor);
    };
    const onKeyDown = (event) => {
      const state = codexPlusTriggerState;
      if (!state || !state.items.length || event.isComposing) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        hideCodexPlusTriggerHints();
      } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        event.stopImmediatePropagation();
        const step = event.key === "ArrowDown" ? 1 : -1;
        state.active = (state.active + step + state.items.length) % state.items.length;
        renderCodexPlusTriggerHints();
      } else if ((event.key === "Enter" || event.key === "Tab") && !event.shiftKey) {
        event.preventDefault();
        event.stopImmediatePropagation();
        codexPlusAcceptTrigger(state.items[state.active]);
      }
    };
    const onFocusOut = (event) => {
      if (codexPlusTriggerState && event.target === codexPlusTriggerState.editor) setTimeout(() => {
        if (codexPlusTriggerState && !codexPlusTriggerState.editor.contains(document.activeElement)) hideCodexPlusTriggerHints();
      }, 150);
    };
    const previous = window.__codexPlusInputTriggerHandlers;
    if (previous) {
      document.removeEventListener("input", previous.onInput, true);
      document.removeEventListener("keydown", previous.onKeyDown, true);
      document.removeEventListener("focusout", previous.onFocusOut, true);
    }
    window.__codexPlusInputTriggerHandlers = { onInput, onKeyDown, onFocusOut };
    document.addEventListener("input", onInput, true);
    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("focusout", onFocusOut, true);
  }

  // ---------------------------------------------------------------- 快捷键
  const codexPlusIsMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent || "");

  /** "Mod+Shift+K" → { mod, ctrl, meta, shift, alt, key }；key 按 event.key 小写比较。 */
  function codexPlusParseShortcut(text) {
    const parts = String(text || "").split("+").map((part) => part.trim()).filter(Boolean);
    const spec = { ctrl: false, meta: false, shift: false, alt: false, key: "" };
    for (const part of parts) {
      const lower = part.toLowerCase();
      if (lower === "mod") spec[codexPlusIsMac ? "meta" : "ctrl"] = true;
      else if (lower === "ctrl" || lower === "control") spec.ctrl = true;
      else if (lower === "cmd" || lower === "meta" || lower === "command") spec.meta = true;
      else if (lower === "shift") spec.shift = true;
      else if (lower === "alt" || lower === "option") spec.alt = true;
      else if (spec.key) throw new Error(`快捷键 ${text} 只能有一个主键`);
      else spec.key = lower === "space" ? " " : lower;
    }
    if (!spec.key) throw new Error(`快捷键 ${text} 缺少主键`);
    if (!spec.ctrl && !spec.meta && !spec.alt && !/^f\d{1,2}$/.test(spec.key)) throw new Error(`快捷键 ${text} 需要带 Mod / Ctrl / Alt（或用 F1~F12），免得吞掉普通输入`);
    return spec;
  }

  function codexPlusShortcutMatches(spec, event) {
    const key = String(event.key || "").toLowerCase();
    const code = String(event.code || "").toLowerCase();
    const sameKey = key === spec.key || code === `key${spec.key}` || code === `digit${spec.key}`;
    return sameKey && event.ctrlKey === spec.ctrl && event.metaKey === spec.meta && event.shiftKey === spec.shift && event.altKey === spec.alt;
  }

  function codexPlusRegisterShortcut(definition, options = {}) {
    if (typeof definition?.run !== "function") throw new Error("快捷键需要 run()");
    const spec = codexPlusParseShortcut(definition.key);
    const dispose = codexPlusRegisterExtensionItem("shortcut", codexPlusRegistry.shortcuts, { ...definition, spec }, options);
    if (!codexPlusShortcutHandlerInstalled) {
      codexPlusShortcutHandlerInstalled = true;
      document.removeEventListener("keydown", window.__codexPlusShortcutHandler, true);
      window.__codexPlusShortcutHandler = (event) => {
        if (event.isComposing || !codexPlusRegistry.shortcuts.size) return;
        const inInput = !!event.target?.closest?.("input, textarea, [contenteditable=true]");
        const item = codexPlusExtensionItems(codexPlusRegistry.shortcuts)
          .find((candidate) => codexPlusShortcutMatches(candidate.spec, event) && (!inInput || candidate.allowInInput !== false));
        if (!item) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        const result = runCodexPlusExtensionCallback(item.scriptKey, `shortcut:${item.key}`, () => item.run({ event }));
        if (result.ok && codexPlusIsThenable(result.value)) {
          result.value.catch((error) => codexPlusMarkExtensionFailure(item.scriptKey, `shortcut:${item.key}: ${String(error?.stack || error?.message || error)}`));
        }
      };
      document.addEventListener("keydown", window.__codexPlusShortcutHandler, true);
    }
    return dispose;
  }

  // ---------------------------------------------------------------- 主题
  const codexPlusThemeStorageKey = "codexPlusExtensionTheme";

  function codexPlusCurrentThemeId() {
    try {
      return localStorage.getItem(codexPlusThemeStorageKey) || "";
    } catch {
      return "";
    }
  }

  function codexPlusThemeCss(theme) {
    const variables = Object.entries(theme.variables || {})
      .filter(([name]) => /^--[\w-]+$/.test(name))
      .map(([name, value]) => `${name}:${String(value).replace(/[;{}]/g, "")}`)
      .join(";");
    return `${variables ? `:root{${variables}}\n` : ""}${typeof theme.css === "string" ? theme.css : ""}`;
  }

  /** 把记住的主题画上去；主题没注册（拓展停用）时撤掉样式但保留选择，拓展回来后自动恢复。 */
  function applyCodexPlusTheme() {
    const id = codexPlusCurrentThemeId();
    const theme = id ? codexPlusExtensionItems(codexPlusRegistry.themes).find((item) => item.themeId === id) : null;
    let style = document.getElementById("codex-plus-extension-theme");
    if (!theme) {
      style?.remove();
      document.documentElement.removeAttribute("data-codex-plus-theme");
      return;
    }
    if (!style) {
      style = document.createElement("style");
      style.id = "codex-plus-extension-theme";
      markCodexPlusExtensionNode(style, theme.scriptKey);
      document.head.append(style);
    }
    const css = codexPlusThemeCss(theme);
    if (style.textContent !== css) style.textContent = css;
    document.documentElement.setAttribute("data-codex-plus-theme", id);
  }

  function codexPlusRegisterTheme(definition, options = {}) {
    const themeId = String(definition?.id || "").trim();
    if (!/^[\w-]{1,48}$/.test(themeId)) throw new Error("主题 id 只能是字母、数字、_、-");
    if (!definition.variables && typeof definition.css !== "string") throw new Error(`主题 ${themeId} 需要 variables 或 css`);
    const dispose = codexPlusRegisterExtensionItem("theme", codexPlusRegistry.themes, { ...definition, themeId, name: definition.name || themeId }, { ...options, id: options.id || `theme-${themeId}` });
    applyCodexPlusTheme();
    return () => {
      dispose();
      applyCodexPlusTheme();
    };
  }

  /** 内置的主题服务（inject: ["themes"]，ctx.themes 同一套）：列出、切换、当前。 */
  function codexPlusThemesService() {
    return Object.freeze({
      list: () => codexPlusExtensionItems(codexPlusRegistry.themes).map((item) => ({ id: item.themeId, name: codexPlusResolveText(item.name), extension: item.scriptKey })),
      get current() {
        return codexPlusCurrentThemeId() || null;
      },
      apply(id) {
        const next = id ? String(id) : "";
        if (next && !codexPlusExtensionItems(codexPlusRegistry.themes).some((item) => item.themeId === next)) throw new Error(`没有主题 ${next}`);
        try {
          if (next) localStorage.setItem(codexPlusThemeStorageKey, next);
          else localStorage.removeItem(codexPlusThemeStorageKey);
        } catch {}
        applyCodexPlusTheme();
        codexPlusEmitExtensionEvent("theme/change", [{ id: next || null }]);
        return next || null;
      },
    });
  }

  // ---------------------------------------------------------------- 向用户提问
  /**
   * 弹一个对话框问用户：options 给按钮（字符串或 { value, label }），input 给输入框（{ placeholder, default, multiline }）。
   * 返回 { value, text }；用户关掉或超时返回 null。同一时间只显示一个，后来的排队。
   */
  let codexPlusAskQueue = Promise.resolve();

  function codexPlusAsk(question = {}) {
    const run = () => new Promise((resolve) => {
      const options = (Array.isArray(question.options) && question.options.length ? question.options : [{ value: "ok", label: "确定" }])
        .slice(0, 6)
        .map((option) => (typeof option === "object" && option ? { value: option.value ?? option.label, label: String(option.label ?? option.value) } : { value: option, label: String(option) }));
      const overlay = document.createElement("div");
      overlay.className = "codex-plus-ask-overlay";
      markCodexPlusExtensionNode(overlay, "builtin-ask");
      const input = question.input
        ? (question.input.multiline ? `<textarea class="codex-plus-ask-input" rows="4"></textarea>` : `<input class="codex-plus-ask-input" type="text">`)
        : "";
      overlay.innerHTML = `<div class="codex-plus-ask" role="dialog" aria-modal="true">
        <div class="codex-plus-ask-title">${escapeHtml(String(codexPlusResolveText(question.title) || "拓展想问你"))}</div>
        ${question.message ? `<div class="codex-plus-ask-message">${escapeHtml(String(codexPlusResolveText(question.message)))}</div>` : ""}
        ${input}
        <div class="codex-plus-ask-actions">
          <button type="button" class="codex-plus-ask-cancel">取消</button>
          ${options.map((option, index) => `<button type="button" class="codex-plus-ask-option" data-index="${index}">${escapeHtml(option.label)}</button>`).join("")}
        </div></div>`;
      const field = overlay.querySelector(".codex-plus-ask-input");
      if (field) {
        field.value = String(question.input.default ?? "");
        field.placeholder = String(question.input.placeholder ?? "");
      }
      let timer = null;
      const finish = (answer) => {
        clearTimeout(timer);
        overlay.remove();
        document.removeEventListener("keydown", onKey, true);
        resolve(answer);
      };
      const onKey = (event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopImmediatePropagation();
          finish(null);
        } else if (event.key === "Enter" && !event.shiftKey && (!field || field.tagName === "INPUT")) {
          event.preventDefault();
          event.stopImmediatePropagation();
          finish({ value: options[options.length - 1].value, text: field ? field.value : null });
        }
      };
      overlay.addEventListener("click", (event) => {
        if (event.target === overlay || event.target.closest(".codex-plus-ask-cancel")) return finish(null);
        const button = event.target.closest(".codex-plus-ask-option");
        if (button) finish({ value: options[Number(button.dataset.index)].value, text: field ? field.value : null });
      });
      document.addEventListener("keydown", onKey, true);
      installCodexPlusAskStyle();
      document.body.appendChild(overlay);
      (field || overlay.querySelector(".codex-plus-ask-option:last-child"))?.focus();
      const timeoutMs = Number(question.timeoutMs);
      if (Number.isFinite(timeoutMs) && timeoutMs > 0) timer = setTimeout(() => finish(null), timeoutMs);
    });
    const answer = codexPlusAskQueue.then(run, run);
    codexPlusAskQueue = answer.catch(() => null);
    return answer;
  }

  function installCodexPlusAskStyle() {
    if (document.getElementById("codex-plus-ask-style")) return;
    const style = document.createElement("style");
    style.id = "codex-plus-ask-style";
    markCodexPlusExtensionNode(style, "builtin-ask");
    style.textContent = `.codex-plus-ask-overlay{position:fixed;inset:0;z-index:2147483646;display:flex;align-items:center;justify-content:center;background:rgba(0,0,0,.35)}
      .codex-plus-ask{width:min(440px,calc(100vw - 48px));display:flex;flex-direction:column;gap:12px;padding:18px;border-radius:14px;border:1px solid color-mix(in srgb,currentColor 14%,transparent);background:var(--color-token-main-surface-primary,var(--codex-plus-bg-elevated,#262626));color:var(--color-token-text-primary,var(--codex-plus-text,#eee));box-shadow:0 16px 48px rgba(0,0,0,.35);font-size:13px}
      .codex-plus-ask-title{font-size:15px;font-weight:600}
      .codex-plus-ask-message{white-space:pre-wrap;opacity:.8;line-height:1.5}
      .codex-plus-ask-input{width:100%;box-sizing:border-box;padding:8px 10px;border-radius:8px;border:1px solid color-mix(in srgb,currentColor 18%,transparent);background:transparent;color:inherit;font:inherit;resize:vertical}
      .codex-plus-ask-actions{display:flex;justify-content:flex-end;flex-wrap:wrap;gap:8px}
      .codex-plus-ask-actions button{padding:6px 14px;border-radius:8px;border:1px solid color-mix(in srgb,currentColor 16%,transparent);background:transparent;color:inherit;cursor:pointer;font:inherit}
      .codex-plus-ask-actions .codex-plus-ask-option:last-child{background:var(--color-token-text-primary,#eee);color:var(--color-token-main-surface-primary,#111);border-color:transparent}`;
    document.head.append(style);
  }

  // ---------------------------------------------------------------- 拓展变更事件
  /** 拓展装上 / 卸下后合并成一次 `extensions/changed`（热重载时一批拓展会连续变动）。 */
  let codexPlusExtensionsChangedTimer = null;

  function scheduleCodexPlusExtensionsChanged() {
    clearTimeout(codexPlusExtensionsChangedTimer);
    codexPlusExtensionsChangedTimer = setTimeout(() => {
      const extensions = [...codexPlusExtensionInstances].map((instance) => ({ id: instance.id, scriptKey: instance.scriptKey, version: instance.manifest?.version ?? null }));
      codexPlusEmitExtensionEvent("extensions/changed", [{ extensions }]);
    }, 300);
  }
