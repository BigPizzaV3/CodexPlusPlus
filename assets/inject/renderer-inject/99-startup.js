  /**
   * 多开实例身份提示。
   *
   * 启动器会在独立 Codex 实例的 renderer 注入前设置
   * `window.__CODEX_PLUS_INSTANCE_LABEL__`（或兼容用的
   * `window.__CODEX_PLUS_INSTANCE_ID__`）。普通主实例没有这些值，因此不会
   * 渲染提示。节点带有扩展归属标记，避免被扫描调度器当成 Codex 原生节点
   * 触发自喂循环；也不占用公开的 window.codexPlus API 名称。
   */
  function installCodexPlusInstanceIdentity() {
    const raw = window.__CODEX_PLUS_INSTANCE_LABEL__ || window.__CODEX_PLUS_INSTANCE_ID__;
    const label = String(raw || "").replace(/\s+/g, " ").trim();
    if (!label) return;
    const mount = () => {
      if (!document.body) return false;
      let node = document.getElementById("codex-plus-instance-identity");
      if (!node) {
        node = document.createElement("div");
        node.id = "codex-plus-instance-identity";
        node.setAttribute("data-codex-plus-ext", "instance-identity");
        node.setAttribute("role", "status");
        node.style.cssText = [
          "position:fixed",
          "top:8px",
          "left:50%",
          "transform:translateX(-50%)",
          "z-index:2147483600",
          "pointer-events:none",
          "user-select:none",
          "box-sizing:border-box",
          "max-width:min(80vw,640px)",
          "overflow:hidden",
          "text-overflow:ellipsis",
          "white-space:nowrap",
          "padding:3px 10px",
          "border:1px solid color-mix(in srgb,currentColor 22%,transparent)",
          "border-radius:999px",
          "background:color-mix(in srgb,var(--color-token-bg-primary,#202020) 88%,transparent)",
          "color:var(--color-token-text-secondary,rgba(210,210,220,.9))",
          "font:11px/16px ui-monospace,SFMono-Regular,Menlo,Monaco,Consolas,monospace",
          "letter-spacing:.02em",
          "opacity:.86",
        ].join(";");
        document.body.appendChild(node);
      }
      node.textContent = `Codex++ 多开 · ${label}`;
      node.setAttribute("aria-label", `当前 Codex++ 多开身份：${label}`);
      return true;
    };
    if (mount()) return;
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", mount, { once: true });
    } else {
      const observer = new MutationObserver(() => {
        if (mount()) observer.disconnect();
      });
      observer.observe(document.documentElement, { childList: true, subtree: true });
      setTimeout(() => observer.disconnect(), 10000);
    }
  }

  installCodexConfigReadDeadline();
  installCodexPlusForceChineseLocale();
  installCodexPlusInstanceIdentity();
  runScanStep(installCodexPlusTypingEffects);
  runScanStep(syncCodexPlusAgentPet);
  let codexPlusResizeRafId = 0;
  window.__codexPlusResizeHandler = () => {
    cancelAnimationFrame(codexPlusResizeRafId);
    codexPlusResizeRafId = requestAnimationFrame(() => {
      syncActionGroupsLayout(true);
      runScanStep(refreshConversationView);
    });
  };
  window.addEventListener("resize", window.__codexPlusResizeHandler);
  window.__codexSessionDeleteObserver?.disconnect();
  window.__codexSessionDeleteObserver = new MutationObserver(scheduleScan);
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
  window.codexPlus = buildCodexPlusExtensionApi();
