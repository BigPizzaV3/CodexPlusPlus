/* Public floating-panel injection entry: publish the runtime and start it. */

  window[API_KEY] = {
    version: SCRIPT_VERSION,
    instanceId: INSTANCE_ID,
    state,
    scan,
    start,
    destroy,
    loadSettings,
    syncSettings,
    renderFloat,
    diagnostics: () => state.diagnostics.slice(),
  };

  void start();
