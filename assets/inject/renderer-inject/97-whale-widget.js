  // 完整版鲸鱼由上游原始引擎 + Codex 数据适配层组成；默认关闭。
  window.__codexPlusWhaleWidgetRuntime?.dispose?.();
  const codexPlusWhaleState = { disposed: false, runtime: null, profile: "", missingSource: false, failedUntil: 0, notice: null };
  function codexPlusWhaleEnabled() {
    return !codexPlusWhaleState.disposed && codexPlusBackendSettingsLoaded && codexPlusSettings().whaleWidget === true;
  }
  function codexPlusWhaleStop() {
    codexPlusWhaleState.runtime?.dispose();
    codexPlusWhaleState.runtime = null;
    codexPlusWhaleState.notice?.remove(); codexPlusWhaleState.notice = null;
  }
  function codexPlusWhaleInitializationFailed(runtime) {
    runtime?.dispose(); codexPlusWhaleState.runtime = null; codexPlusWhaleState.failedUntil = Date.now() + 10000;
    if (codexPlusWhaleState.notice) return;
    const notice = document.createElement("div"); notice.setAttribute("data-codex-plus-ext", "whale-widget"); notice.setAttribute("role", "status");
    notice.style.cssText = "position:fixed;right:20px;bottom:20px;z-index:2147483600;max-width:320px;padding:12px;border-radius:10px;background:#fff5ec;color:#7c3218;font:12px/1.6 system-ui;box-shadow:0 3px 16px #0002";
    notice.textContent = "鲸鱼挂件暂时无法加载，10 秒后自动重试。可在 Codex++ 增强设置关闭后重新开启。";
    document.body.appendChild(notice); codexPlusWhaleState.notice = notice;
  }
  function codexPlusWhaleProfileFingerprint() {
    const profile = codexRemoteSessionActiveProfile?.();
    const settings = codexPlusBackendSettings || {};
    return JSON.stringify([profile?.id || "", profile?.relayMode || "", settings.codexAppWhaleBalanceProtocol,
      settings.codexAppWhaleBalancePath, settings.codexAppWhaleBalanceField, settings.codexAppWhaleBalanceCurrency,
      settings.codexAppWhaleBalanceScale]);
  }
  function syncCodexPlusWhaleWidget() {
    if (!codexPlusWhaleEnabled()) { codexPlusWhaleStop(); codexPlusWhaleState.failedUntil = 0; return; }
    const profile = codexPlusWhaleProfileFingerprint();
    if (codexPlusWhaleState.profile !== profile) { codexPlusWhaleStop(); codexPlusWhaleState.failedUntil = 0; }
    codexPlusWhaleState.profile = profile;
    if (codexPlusWhaleState.runtime) {
      codexPlusWhaleState.runtime.context();
      codexPlusWhaleState.runtime.subscribe();
      codexPlusWhaleState.runtime.pause(document.hidden === true);
      return;
    }
    // 隐藏窗口等恢复可见后再挂载，不在后台构造音频或轮询会话日志。
    if (document.hidden) return;
    if (Date.now() < codexPlusWhaleState.failedUntil) return;
    if (typeof window.__CODEX_PLUS_WHALE_ENGINE__ !== "function") {
      if (!codexPlusWhaleState.missingSource) console.warn("[Codex++] 完整鲸鱼运行时未随启动器加载，请重新构建启动器。");
      codexPlusWhaleState.missingSource = true; codexPlusWhaleInitializationFailed(null); return;
    }
    const runtime = createCodexPlusWhaleFullRuntime();
    codexPlusWhaleState.runtime = runtime;
    registerCodexPlusExtensionSelector("[data-codex-plus-ext=whale-widget]");
    void runtime.initialize().then(() => {
      if (runtime.active() && codexPlusWhaleState.runtime === runtime && codexPlusWhaleEnabled()) {
        window.__CODEX_PLUS_WHALE_ENGINE__(...runtime.environment);
        if (runtime.state.initializationError || !runtime.state.controls) throw new Error("完整鲸鱼初始化未完成");
        codexPlusWhaleState.notice?.remove(); codexPlusWhaleState.notice = null;
      }
    }).catch((error) => {
      if (!runtime.active()) return;
      if (codexPlusWhaleState.runtime === runtime) codexPlusWhaleInitializationFailed(runtime);
    });
  }
  window.__codexPlusWhaleWidgetRuntime = {
    version: 3,
    sync: syncCodexPlusWhaleWidget,
    dispose() { codexPlusWhaleStop(); codexPlusWhaleState.disposed = true; },
    get state() { return codexPlusWhaleState.runtime?.state || null; },
  };
