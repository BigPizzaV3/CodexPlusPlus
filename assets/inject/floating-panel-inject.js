/* Public floating-panel injection entry: lifecycle methods and debug surface. */

  window[API_KEY] = {
    version: SCRIPT_VERSION,
    instanceId: INSTANCE_ID,
    state,
    scan,
    start,
    destroy,
    loadSettings,
    syncSettings,
    setOpen,
    setMaterial: writeMaterial,
    toggleMaterial,
    dockRight: dockRightKeepHeight,
    getFabExpression: () => resolveFabExpression(),
    renderFloat,
    diagnostics: () => state.diagnostics.slice(),
  };

  void start();
