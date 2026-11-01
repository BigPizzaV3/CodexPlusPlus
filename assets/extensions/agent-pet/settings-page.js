  // 管理器里的设置页（settings.html）：把沙箱页面的 codexPlusManager 接成 settings.js 要的 host。
  // 打包时和 settings.js 一起内联进 settings.html（见 crates/codex-plus-core/src/bundled_extensions.rs）。
  (async () => {
    const root = document.getElementById("root");
    const manager = window.codexPlusManager;
    try {
      const config = await manager.config.get();
      createCodexPlusAgentPetSettings(root, {
        config,
        save: (values) => manager.config.set(values),
        call: (route, payload) => manager.call(route, payload),
        asset: (path) => manager.asset(path),
      });
    } catch (error) {
      root.textContent = `设置页加载失败：${error?.message || error}`;
    }
  })();
