(() => {
  const runtime = window.__codexPlusUserScripts ||= { scripts: {} };
  runtime.documentId ||= `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  runtime.reloadOperations ||= {};
  runtime.nativePageReadiness = () => {
    if (document.readyState === "loading") return { ready: false, reason: "document_loading" };
    const root = document.getElementById("root");
    if (!root) return { ready: false, reason: "native_root_missing" };
    const native = (selector, scope = root) => [...scope.querySelectorAll(selector)].some((element) =>
      !element.closest("[data-codex-plus-ext], [id^='codex-plus-']") &&
      !element.closest("[hidden], [aria-hidden='true']"));
    if (native("main .ProseMirror[contenteditable='true'], [role='main'] .ProseMirror[contenteditable='true']")) {
      return { ready: true, view: "conversation" };
    }
    const hasNavigation = native("#app-shell-sidebar, .app-shell-left-panel, nav[data-app-navigation-rail], .sidebar-navigation");
    const hasContent = native("main h1, main h2, main button, main [role='tabpanel'], [role='main'] h1, [role='main'] button");
    if (hasNavigation && hasContent) return { ready: true, view: "native_shell" };
    return { ready: false, reason: "native_view_pending" };
  };
  runtime.revokeBackendSource = () => {
    window.__codexPlusBackendSource = null;
    window.__codexPlusBackendServices = [];
    try { window.__codexPlusSyncBackendServices?.(); } catch {}
  };
  runtime.registerCleanup = (cleanup) => {
    const script = runtime.scripts[runtime.currentKey];
    if (!script || typeof cleanup !== "function") throw new Error("Register cleanup during script initialization");
    (script.cleanups ||= []).push(cleanup);
  };
  runtime.prepareReload = (options = {}) => {
    const refreshPage = (reason, degraded = false) => {
      runtime.lastReloadReason = reason;
      if (options.allowPage === false) {
        if (degraded) {
          runtime.reloadDegraded = true;
          runtime.revokeBackendSource();
        }
        return degraded ? "degraded" : "blocked";
      }
      if (!window.__codexPlusUserScriptsBootstrap) {
        throw new Error("请先重启 Codex++，以启用旧脚本的安全重载支持。");
      }
      // 旧脚本没有完整的清理协议，交给浏览器释放整个页面的资源。
      runtime.revokeBackendSource();
      if (!runtime.refreshPending) {
        runtime.refreshPending = true;
        window.setTimeout(() => window.location.reload(), 0);
      }
      return "page";
    };
    if (runtime.refreshPending) return "page";
    if (runtime.reloadDegraded && options.allowPage === false) {
      runtime.lastReloadReason = "previous_cleanup_incomplete";
      return "degraded";
    }
    if (runtime.reloadDegraded) return refreshPage("previous_cleanup_incomplete");
    const scripts = Object.values(runtime.scripts);
    if (scripts.some((script) => !script.cleanups?.length)) return refreshPage("cleanup_missing");
    runtime.revokeBackendSource();
    for (const script of scripts.reverse()) {
      for (const cleanup of script.cleanups.splice(0).reverse()) {
        try {
          const result = cleanup();
          // 异步清理无法在同一轮注入前完成，安全回退到页面刷新。
          if (result && typeof result.then === "function") {
            Promise.resolve(result).catch(() => {});
            return refreshPage("cleanup_async", true);
          }
        } catch {
          return refreshPage("cleanup_failed", true);
        }
      }
    }
    runtime.scripts = {};
    runtime.appliedRevision = null;
    runtime.lastReloadReason = "";
    return "scripts";
  };
  // Preparation and application are separate so the owner can stop the old
  // backend after cleanup without injecting a second copy on an uncertain ACK.
  runtime.prepareOperation = (id, documentId, revision, allowPage, sequence) => {
    if (!id || documentId !== runtime.documentId) return { phase: "rejected", reason: "document_changed" };
    const existing = runtime.reloadOperations[id];
    if (existing) return existing.revision === revision && existing.allowPage === allowPage && existing.sequence === sequence
      ? existing : { phase: "rejected", reason: "operation_identity_conflict" };
    if (!Number.isSafeInteger(sequence) || sequence <= (runtime.reloadSequence || 0)) {
      return { phase: "rejected", reason: "stale_operation" };
    }
    const current = runtime.reloadOperations[runtime.currentReloadOperation];
    if (current && ["quiescing", "applying"].includes(current.phase)) {
      return { phase: "rejected", reason: "operation_in_progress" };
    }
    if (current && current.phase === "stopped") current.phase = "superseded";
    runtime.reloadSequence = sequence;
    const operation = { id, documentId, revision, allowPage, sequence, phase: "quiescing" };
    runtime.reloadOperations[id] = operation;
    runtime.currentReloadOperation = id;
    const ids = Object.keys(runtime.reloadOperations);
    if (ids.length > 256) {
      for (const old of ids) {
        if (Object.keys(runtime.reloadOperations).length <= 256) break;
        const entry = runtime.reloadOperations[old];
        if (old !== id && !["quiescing", "stopped", "applying"].includes(entry?.phase)) delete runtime.reloadOperations[old];
      }
    }
    let mode;
    try {
      mode = runtime.prepareReload({ allowPage });
    } catch (error) {
      operation.phase = "failed";
      operation.reason = String(error && (error.message || error) || error);
      return operation;
    }
    operation.mode = mode;
    operation.reason = runtime.lastReloadReason || "";
    operation.phase = mode === "scripts" ? "stopped"
      : mode === "page" ? "pageRequested"
      : mode === "blocked" ? "blocked" : "degraded";
    return operation;
  };
  runtime.applyOperation = (id, documentId, revision, apply) => {
    const operation = runtime.reloadOperations[id];
    if (!operation || documentId !== runtime.documentId || operation.documentId !== documentId ||
        operation.revision !== revision || runtime.currentReloadOperation !== id ||
        operation.sequence !== runtime.reloadSequence) {
      return { phase: "rejected", reason: "operation_or_document_changed" };
    }
    if (operation.phase !== "stopped") return operation;
    operation.phase = "applying";
    try {
      apply();
      operation.phase = "verifying";
      runtime.initialized = true;
      runtime.appliedRevision = revision;
    } catch (error) {
      operation.phase = "failed";
      operation.reason = String(error && (error.message || error) || error);
    }
    return operation;
  };
})();
