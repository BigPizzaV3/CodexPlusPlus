// 控制开关：window.__CODEX_PLUS_PASTE_FIX__ = { enabled: <bool> }
// 由 CodexPlusPlus 在启动时根据 settings.codexAppPasteFix 注入。
// 关闭时不进入 if 体，行为与原 Codex 完全一致；开启时在 document 捕获阶段
// 拦截 paste，若 text/plain 非空则阻止默认行为并调用 execCommand('insertText')
// 插入纯文本，避免 Codex 把 Word 复制的内容识别为附件。
// SENTINEL 保证多次执行（页面刷新、脚本重注入）只装一次 handler。
if (window.__CODEX_PLUS_PASTE_FIX__ && window.__CODEX_PLUS_PASTE_FIX__.enabled === true) {
  (() => {
    const SENTINEL = '__codexPasteFixInstalled__';
    if (window[SENTINEL]) return;
    window[SENTINEL] = true;

    const TAG = '[PasteFix]';

    const handler = (e) => {
      const cd = e.clipboardData;
      if (!cd) return;

      const text = cd.getData('text/plain');
      if (typeof text !== 'string' || text.length === 0) return;

      e.preventDefault();
      e.stopImmediatePropagation();

      let ok = false;
      try {
        ok = document.execCommand('insertText', false, text);
      } catch (err) {
        console.warn(TAG, 'execCommand threw:', err && err.message);
      }
      if (!ok) {
        console.warn(TAG, 'execCommand failed; please paste again');
      }
    };

    document.addEventListener('paste', handler, { capture: true });
    console.log(TAG, 'paste handler installed (capture phase)');
  })();
}
