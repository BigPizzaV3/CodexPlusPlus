  /**
   * 内置拓展「粘贴修复」：在 document 捕获阶段拦截 paste，text/plain 非空时阻止默认行为并用
   * execCommand('insertText') 插入纯文本，避免 Codex 把 Word 复制的内容识别为附件。
   *
   * 监听经 ctx.listen 挂上，拓展停用 / 被顶替时随卸载摘掉；热重注入时旧实例先卸载，不会重复挂。
   * 开关：读到后端设置前按启动时注入的 window.__CODEX_PLUS_PASTE_FIX__（settings.codexAppPasteFix），
   * 读到之后按设置项 pasteFix，所以改设置不必重启 Codex。
   */
  // 超过这个长度就不再拦截，交回 Codex 原生的「超长文本转附件」逻辑。
  // 否则整段长文本会被 insertText 直接塞进输入框，造成渲染层卡死（issue #1931）。
  const codexPlusPasteFixMaxInlineLength = 20000;

  function codexPlusPasteFixEnabled() {
    return codexPlusBackendSettingsLoaded
      ? codexPlusSettings().pasteFix === true
      : window.__CODEX_PLUS_PASTE_FIX__?.enabled === true;
  }

  function codexPlusPasteFixHandler(event) {
    if (!codexPlusPasteFixEnabled()) return;
    const text = event.clipboardData?.getData("text/plain");
    if (typeof text !== "string" || text.length === 0) return;
    // 长文本放行：让 Codex 自己决定是否转成附件
    if (text.length > codexPlusPasteFixMaxInlineLength) {
      console.log("[PasteFix]", `text too long (${text.length} chars); letting Codex handle it`);
      return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
    let ok = false;
    try {
      ok = document.execCommand("insertText", false, text);
    } catch (error) {
      console.warn("[PasteFix]", "execCommand threw:", error && error.message);
    }
    if (!ok) console.warn("[PasteFix]", "execCommand failed; please paste again");
  }

  function codexPlusPasteFixExtension(ctx) {
    ctx.listen(document, "paste", codexPlusPasteFixHandler, { capture: true });
  }
