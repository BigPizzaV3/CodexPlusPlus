  installCodexConfigReadDeadline();
  void installCodexForkReadinessFallback();
  installCodexPlusForceChineseLocale();
  // 打字特效、桌宠（含大肥鱼）由各自的内置拓展装上（syncCodexPlusBuiltinExtensions），这里不再直接调用。
  let codexPlusResizeRafId = 0;
  window.__codexPlusResizeHandler = () => {
    cancelAnimationFrame(codexPlusResizeRafId);
    codexPlusResizeRafId = requestAnimationFrame(() => {
      syncActionGroupsLayout(true);
    });
  };
  window.addEventListener("resize", window.__codexPlusResizeHandler);
  window.__codexSessionDeleteObserver?.disconnect();
  window.__codexSessionDeleteObserver = new MutationObserver((mutations) => {
    // 先同步藏起已删会话的重建行（首帧前生效，#2791），再按原规则排扫描。
    runScanStep(() => hideDeletedSessionRowsInMutations(mutations));
    scheduleScan(mutations);
  });
  window.__codexSessionDeleteObserver.observe(document.body || document.documentElement, {
    childList: true,
    subtree: true,
    // Codex may promote a newly-created row from a temporary client ID to its
    // persisted UUID without replacing the DOM node. Re-scan those rows so the
    // action button and its delete reference are rebuilt from the canonical ID.
    attributes: true,
    attributeFilter: ["data-app-action-sidebar-thread-id", "href"],
  });
  // 会话复制 / 自动命名两项已下线；同页重复注入时摘掉旧脚本留下的触发器监听。
  document.removeEventListener("pointerdown", window.__codexSessionActionTriggerHandler, true);
  document.removeEventListener("click", window.__codexSessionActionTriggerClickHandler, true);
  window.__codexSessionActionTriggerHandler = null;
  window.__codexSessionActionTriggerClickHandler = null;
  // 对外接口层在此刻挂载：此时所有分片都已执行完毕，闭包里的函数全部就绪。
  // 放在 99-tail 收尾之前，确保 IIFE 结束前 window.codexPlus 已经可用——
  // 用户脚本的注入晚于本脚本，不会撞上这个时间点。
  installCodexPlusBuiltinServices();
  window.codexPlus = buildCodexPlusExtensionApi();
  // 内置功能挂到拓展内核上（见 96-builtin-extensions.js）。旧版直接挂在 body 上的多开身份提示先摘掉，
  // 现在由内置拓展经 overlay 插槽画。用户脚本注入包下发停用 / 顶替状态后会再调一次。
  document.querySelector('body > #codex-plus-instance-identity[data-codex-plus-ext="instance-identity"]')?.remove();
  window.__codexPlusSyncBuiltinExtensions = syncCodexPlusBuiltinExtensions;
  syncCodexPlusBuiltinExtensions();
