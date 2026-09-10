/* Runtime settings synchronization and feature enablement. */

  async function syncSettings(patch = {}) {
    if (!isCurrentInstance()) return null;
    const normalizedPatch = {};
    if (patch && typeof patch === "object") {
      Object.entries(patch).forEach(([key, value]) => {
        if (value !== undefined) normalizedPatch[key] = value;
      });
    }
    if (Object.keys(normalizedPatch).length) {
      if (!state.settingsLoaded) {
        pendingSettingsPatch = { ...pendingSettingsPatch, ...normalizedPatch };
      }
      applyRuntimeSettings({ ...(state.settings || {}), ...normalizedPatch });
    }
    const hasRuntimePatch = typeof normalizedPatch.enabled === "boolean"
      || typeof normalizedPatch.answerOutlineEnabled === "boolean"
      || Object.prototype.hasOwnProperty.call(normalizedPatch, "generationMode");
    if (patch?.enabled === true) {
      pushDiagnostic("settings:enabled-sync", {});
    }
    if (patch?.answerOutlineEnabled === true) pushDiagnostic("settings:outline-enabled-sync", {});
    if (Object.prototype.hasOwnProperty.call(normalizedPatch, "generationMode")) {
      pushDiagnostic("settings:generation-mode-sync", {
        mode: stepwiseGenerationMode(),
      });
    }
    if (hasRuntimePatch) {
      const hasInFlightSettingsRequest = Boolean(settingsPromise);
      settingsSyncEpoch += 1;
      if (!state.settingsLoaded || hasInFlightSettingsRequest) {
        pendingSettingsPatch = { ...pendingSettingsPatch, ...normalizedPatch };
        settingsPromise = null;
        void reloadSettings();
      }
      if (!runtimeEnabled()) {
        pushDiagnostic("settings:disabled-sync", {});
        if (state.runtimeActive) stopRuntime();
        return state.settings;
      }
      activateRuntime();
      renderFloat();
      scheduleScan(0);
      return state.settings;
    }

    settingsPromise = null;
    startupPromise = null;
    const settings = await loadSettings();
    if (!isCurrentInstance()) return null;
    if (!runtimeEnabled(settings)) {
      pushDiagnostic("settings:disabled-sync", {});
      if (state.runtimeActive) stopRuntime();
      return settings;
    }
    pushDiagnostic("settings:enabled-sync", {});
    activateRuntime();
    renderFloat();
    scheduleScan(0);
    return settings;
  }

  function destroy() {
    state.destroyed = true;
    state.promptContext = null;
    state.latestTurnAnchor = null;
    state.pinnedPaneKey = "";
    state.pinnedSessionId = "";
    state.pinnedThreadRoot = null;
    if (state.settingsSyncTimer) window.clearTimeout(state.settingsSyncTimer);
    state.settingsSyncTimer = 0;
    cancelSourceCueAnimation();
    cancelViewAnimation();
    stopRuntime();
    if (window[API_KEY]?.instanceId === INSTANCE_ID) delete window[API_KEY];
  }

  function escapeHtml(value) {
    return String(value ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");
  }

  function escapeAttr(value) {
    return escapeHtml(value);
  }

  async function start() {
    scheduleSettingsSync();
    if (startupPromise) return startupPromise;
    const generation = state.runtimeGeneration;
    startupPromise = (async () => {
      const settings = await ensureSettings();
      if (!isCurrentInstance() || generation !== state.runtimeGeneration) return;
      if (!runtimeEnabled(settings)) {
        pushDiagnostic("startup:disabled", {});
        startupPromise = null;
        return;
      }
      activateRuntime();
    })();
    return startupPromise;
  }
