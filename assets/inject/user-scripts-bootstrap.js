(() => {
  if (window.top !== window || window.self !== window || !window.electronBridge || !/^app:\/\/\-\//i.test(window.location.href)) return;
  if (window.__codexPlusUserScriptsBootstrap) return;
  window.__codexPlusUserScriptsBootstrap = true;
  const failed = () => {
    const runtime = window.__codexPlusUserScripts ||= { scripts: {} };
    runtime.initialError = "initialization_failed";
  };
  const load = () => {
    // 每次页面加载都读取当前文件与开关，不保留启动时的旧脚本副本。
    window.__codexSessionDeleteBridge("/user-scripts/load", {}).then((result) => {
      if (result?.status === "failed") {
        failed();
        console.warn("[Codex++] user scripts:", result.message);
      } else if (window.__codexPlusUserScripts) {
        window.__codexPlusUserScripts.initialError = null;
      }
    }).catch((error) => {
      failed();
      console.warn("[Codex++] user scripts:", error);
    });
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", load, { once: true });
  else load();
})();
