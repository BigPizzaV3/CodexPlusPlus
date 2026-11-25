(() => {
  if (window.top !== window || window.self !== window || !window.electronBridge || !/^app:\/\/\-\//i.test(window.location.href)) return;
  if (window.__codexPlusUserScriptsBootstrap) return;
  window.__codexPlusUserScriptsBootstrap = true;
  const load = () => {
    // 每次页面加载都读取当前文件与开关，不保留启动时的旧脚本副本。
    window.__codexSessionDeleteBridge("/user-scripts/load", {}).then((result) => {
      if (result?.status === "failed") console.warn("[Codex++] user scripts:", result.message);
    }).catch((error) => console.warn("[Codex++] user scripts:", error));
  };
  const loadWhenApiReady = () => {
    if (window.codexPlus) load();
    else window.addEventListener("codex-plus-extension-api-ready", load, { once: true });
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", loadWhenApiReady, { once: true });
  } else loadWhenApiReady();
})();
