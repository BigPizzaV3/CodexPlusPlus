(() => {
  const runtime = window.__codexPlusUserScripts ||= { scripts: {} };
  runtime.registerCleanup = (cleanup) => {
    const script = runtime.scripts[runtime.currentKey];
    if (!script || typeof cleanup !== "function") throw new Error("Register cleanup during script initialization");
    (script.cleanups ||= []).push(cleanup);
  };
  runtime.prepareReload = () => {
    const refreshPage = () => {
      if (!window.__codexPlusUserScriptsBootstrap) {
        throw new Error("请先重启 Codex++，以启用旧脚本的安全重载支持。");
      }
      // 旧脚本没有完整的清理协议，交给浏览器释放整个页面的资源。
      if (!runtime.refreshPending) {
        runtime.refreshPending = true;
        window.setTimeout(() => window.location.reload(), 0);
      }
      return "page";
    };
    if (runtime.refreshPending) return "page";
    const scripts = Object.values(runtime.scripts);
    if (scripts.some((script) => !script.cleanups?.length)) return refreshPage();
    for (const script of scripts.reverse()) {
      for (const cleanup of script.cleanups.splice(0).reverse()) {
        try {
          const result = cleanup();
          // 异步清理无法在同一轮注入前完成，安全回退到页面刷新。
          if (result && typeof result.then === "function") {
            Promise.resolve(result).catch(() => {});
            return refreshPage();
          }
        } catch {
          return refreshPage();
        }
      }
    }
    runtime.scripts = {};
    return "scripts";
  };
})();
