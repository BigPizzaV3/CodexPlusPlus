  let codexThreadGoalInstallPending = false;
  let codexThreadGoalInstallAt = 0;
  function installCodexThreadGoalForwarding() {
    const runtime = window.__codexPlusNativeThreadGoal;
    if (!runtime || codexThreadGoalInstallPending || Date.now() < codexThreadGoalInstallAt) return;
    const ref = currentSessionRef();
    if (!ref.session_id) return;
    codexThreadGoalInstallPending = true;
    codexThreadGoalInstallAt = Date.now() + 2000;
    void runtime.install(ref.session_id, {
      hostId: ref.host_id || "local",
      loadModule: () => loadOptionalCodexAppModule("app-shared-"),
      enabled: () => codexPlusBackendSettings.enhancementsEnabled !== false,
      forward: request => postJson("/thread-goal/set-in-owner", request),
      report: code => sendCodexPlusDiagnostic("thread_goal_forward_failed", { code }),
    }).catch(() => {}).finally(() => { codexThreadGoalInstallPending = false; });
  }
