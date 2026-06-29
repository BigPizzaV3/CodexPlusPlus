(() => {
  function installCodexPlusForceChineseLocale() {
    const config = window.__CODEX_PLUS_FORCE_CHINESE_LOCALE__;
    if (!config || config.enabled !== true) return;
    if (window.__codexPlusForceChineseLocaleInstalled === "1") return;
    window.__codexPlusForceChineseLocaleInstalled = "1";
    const locale = typeof config.locale === "string" && config.locale ? config.locale : "zh-CN";
    const languages = [locale, "zh", "en-US", "en"];

    const defineNavigatorGetter = (name, value) => {
      try {
        Object.defineProperty(Navigator.prototype, name, {
          configurable: true,
          get: () => value,
        });
      } catch {
        try {
          Object.defineProperty(navigator, name, {
            configurable: true,
            get: () => value,
          });
        } catch {
        }
      }
    };

    defineNavigatorGetter("language", locale);
    defineNavigatorGetter("languages", languages);

    const patchI18nConfig = (dynamicConfig) => {
      if (!dynamicConfig || typeof dynamicConfig !== "object") return dynamicConfig;
      const value = dynamicConfig.value && typeof dynamicConfig.value === "object" ? dynamicConfig.value : {};
      const nextValue = {
        ...value,
        enable_i18n: true,
        locale_source: "SYSTEM",
      };
      try {
        dynamicConfig.value = nextValue;
      } catch {
      }
      if (typeof dynamicConfig.get === "function" && !dynamicConfig.__codexPlusForceChineseLocaleGetPatched) {
        const originalGet = dynamicConfig.get.bind(dynamicConfig);
        dynamicConfig.get = (key, fallback) => {
          if (key === "enable_i18n") return true;
          if (key === "locale_source") return "SYSTEM";
          return originalGet(key, fallback);
        };
        dynamicConfig.__codexPlusForceChineseLocaleGetPatched = true;
      }
      return dynamicConfig;
    };

    const statsigClients = () => {
      const root = window.__STATSIG__ || globalThis.__STATSIG__;
      if (!root || typeof root !== "object") return [];
      const clients = [root.firstInstance, typeof root.instance === "function" ? root.instance() : null];
      if (root.instances && typeof root.instances === "object") clients.push(...Object.values(root.instances));
      return clients.filter((client, index, array) => client && typeof client === "object" && array.indexOf(client) === index);
    };

    const patchStatsigClient = (client) => {
      if (!client || typeof client !== "object") return;
      if (typeof client.getDynamicConfig !== "function") return;
      if (!client.__codexPlusForceChineseLocalePatched) {
        const originalGetDynamicConfig = client.getDynamicConfig.bind(client);
        client.getDynamicConfig = (name, options) => {
          const result = originalGetDynamicConfig(name, options);
          return name === "72216192" ? patchI18nConfig(result) : result;
        };
        client.__codexPlusForceChineseLocalePatched = true;
      }
      try {
        patchI18nConfig(client.getDynamicConfig("72216192", { disableExposureLog: true }));
      } catch {
      }
    };

    const patchStatsigRoot = (root) => {
      if (!root || typeof root !== "object" || root.__codexPlusForceChineseLocaleRootPatched) return;
      root.__codexPlusForceChineseLocaleRootPatched = true;
      ["firstInstance", "instance"].forEach((key) => {
        let current;
        try {
          current = root[key];
        } catch {
          return;
        }
        patchStatsigClient(typeof current === "function" && key === "instance" ? current.call(root) : current);
        try {
          Object.defineProperty(root, key, {
            configurable: true,
            get: () => current,
            set: (next) => {
              current = next;
              patchStatsigClient(typeof next === "function" && key === "instance" ? next.call(root) : next);
            },
          });
        } catch {
        }
      });
    };

    const installStatsigRootSetter = () => {
      const descriptor = Object.getOwnPropertyDescriptor(window, "__STATSIG__");
      if (descriptor && descriptor.configurable === false) return;
      let currentRoot = window.__STATSIG__;
      patchStatsigRoot(currentRoot);
      try {
        Object.defineProperty(window, "__STATSIG__", {
          configurable: true,
          get: () => currentRoot,
          set: (next) => {
            currentRoot = next;
            patchStatsigRoot(next);
            statsigClients().forEach(patchStatsigClient);
          },
        });
      } catch {
      }
    };

    const patchStatsigI18nConfig = () => {
      installStatsigRootSetter();
      const root = window.__STATSIG__ || globalThis.__STATSIG__;
      patchStatsigRoot(root);
      statsigClients().forEach((client) => {
        if (typeof client.getDynamicConfig !== "function") return;
        patchStatsigClient(client);
      });
    };

    patchStatsigI18nConfig();
    const startedAt = Date.now();
    const timer = window.setInterval(() => {
      patchStatsigI18nConfig();
      if (Date.now() - startedAt > 5000) window.clearInterval(timer);
    }, 50);
  }

  installCodexPlusForceChineseLocale();

  const helperBase = window.__CODEX_SESSION_DELETE_HELPER__ || "http://127.0.0.1:57321";
  const buttonClass = "codex-delete-button";
  const exportButtonClass = "codex-export-button";
  const projectMoveButtonClass = "codex-project-move-button";
  const projectMoveOverlayClass = "codex-project-move-overlay";
  const actionButtonClass = "codex-session-action-button";
  const actionGroupClass = "codex-session-actions";
  const moreButtonClass = "codex-session-more-button";
  const moreMenuClass = "codex-session-more-menu";
  const actionTooltipClass = "codex-session-action-tooltip";
  const threadIdBadgeClass = "codex-thread-id-badge";
  const conversationViewMinWidth = 320;
  const conversationViewMaxAllowedWidth = 4000;
  const conversationViewDefaultWidth = 900;
  const conversationViewLegacyWidthKey = "codexPlus.threadCenter.maxWidth";
  const zedRemoteButtonClass = "codex-zed-remote-button";
  const zedRemoteOpenInMenuItemClass = "codex-zed-open-in-menu-item";
  const zedRemoteToastClass = "codex-zed-remote-toast";
  const upstreamWorktreeDialogClass = "codex-upstream-worktree-dialog";
  const upstreamBranchOptionAttribute = "data-codex-upstream-branch-option";
  const upstreamBranchSelectionKey = "codexUpstreamBranchSelection";
  const upstreamProjectContextKey = "codexUpstreamProjectContext";
  const zedRemoteOpenVersion = "1";
  const zedRemoteOpenInMenuVersion = "1";
  const zedRemoteOpenInMenuActivationWindowMs = 600;
  const projectMoveProjectionKey = "codexProjectMoveProjection";
  const legacyProjectMoveOverridesKey = "codexProjectMoveOverrides";
  const projectMoveProjectionTtlMs = 24 * 60 * 60 * 1000;
  const projectMoveProjectionSettleMs = 5 * 60 * 1000;
  const projectMoveRefreshDelaysMs = [50, 250, 750, 1500];
  const chatsSortRefreshIntervalMs = 1500;
  const chatsSortDbRefreshIntervalMs = 5000;
  const styleId = "codex-delete-style";
  const codexDeleteStyleVersion = "14";
  const codexPlusMenuId = "codex-plus-menu";
  const codexPlusMenuFloatingClass = "codex-plus-menu-floating";
  const codexPlusBackendHeartbeatVersion = "5";
  const codexDeleteVersion = "8";
  const codexExportVersion = "1";
  const codexProjectMoveVersion = "1";
  const codexActionGroupVersion = "6";
  const codexArchiveRowActionsVersion = "1";
  const codexArchiveDeleteAllVersion = "2";
  const codexConversationViewVersion = "1";
  const codexThreadScrollVersion = "1";
  const codexThreadIdBadgeVersion = "1";
  const codexThreadServiceTierVersion = "1";
  const codexServiceTierBadgeClass = "codex-service-tier-badge";
  const codexServiceTierBadgeVersion = "3";
  const codexMenuLocalizationVersion = "1";
  const codexMenuLocalizationMap = new Map([
    ["Toggle Sidebar", "切换侧边栏"],
    ["Toggle Bottom Panel", "切换底部面板"],
    ["Toggle Pinned Summary", "切换置顶摘要"],
    ["Open Terminal", "打开终端"],
    ["Toggle File Tree", "切换文件树"],
    ["Open Browser Tab", "打开浏览器标签页"],
    ["Focus Browser Address Bar", "聚焦浏览器地址栏"],
    ["Reload Browser Page", "重新加载浏览器页面"],
    ["Force Reload Browser Page", "强制重新加载浏览器页面"],
    ["Toggle Browser Panel", "切换浏览器面板"],
    ["Toggle Side Panel", "切换侧边面板"],
    ["Find", "查找"],
    ["Previous Chat", "上一个对话"],
    ["Next Chat", "下一个对话"],
    ["Back", "后退"],
    ["Forward", "前进"],
    ["Zoom In", "放大"],
    ["Zoom Out", "缩小"],
    ["Actual Size", "实际大小"],
    ["Toggle Full Screen", "切换全屏"],
    ["Keyboard Shortcuts", "键盘快捷键"],
    ["Open command menu", "打开命令菜单"],
    ["Search Chats…", "搜索对话…"],
    ["Search Files…", "搜索文件…"],
    ["New Chat", "新建对话"],
    ["Quick Chat", "快速对话"],
    ["Open in New Window", "在新窗口打开"],
    ["Archive chat", "归档对话"],
    ["Pin/unpin chat", "置顶/取消置顶对话"],
    ["Settings…", "设置…"],
    ["Open Folder…", "打开文件夹…"],
    ["Close Tab", "关闭标签页"],
    ["Close", "关闭"],
    ["New Window", "新建窗口"],
    ["Copy conversation path", "复制对话路径"],
    ["Copy deeplink", "复制深层链接"],
    ["Copy session id", "复制会话 ID"],
    ["Copy working directory", "复制工作目录"],
  ]);
  let codexPlusVersion = window.__CODEX_PLUS_VERSION__ || "unknown";
  const codexPlusBuild = window.__CODEX_PLUS_BUILD__ || "unknown";
  const codexPlusSettingsKey = "codexPlusSettings";
  const codexThreadScrollKey = "codexThreadScroll";
  const codexThreadServiceTierKey = "codexThreadServiceTierOverrides";
  const codexThreadServiceTierMaxEntries = 120;
  const codexThreadServiceTierDraftBindWindowMs = 60 * 1000;
  const codexServiceTierRequestOverrideVersion = "3";
  const codexAppServerModelRequestPatchVersion = "1";
  const codexPluginMarketplaceUnlockVersion = "12";
  const codexPluginAutoExpandVersion = "1";
  const codexPluginAutoExpandMaxClicks = 80;
  const codexPluginAutoExpandClickDelayMs = 90;
  const codexThreadScrollMaxEntries = 120;
  const codexThreadScrollSaveThrottleMs = 120;
  const codexThreadScrollRestoreWindowMs = 3200;
  const codexThreadScrollRestoreDelaysMs = [0, 80, 220, 500, 1000, 1800, 2800];
  const codexThreadScrollUserIntentWindowMs = 1200;
  const codexThreadScrollProgrammaticGuardVersion = "dispatcher:2";
  const codexThreadScrollRouteHooksVersion = "dispatcher:2";
  const codexThreadScrollListenerVersion = "4";
  const codexThreadScrollUserIntentVersion = "dispatcher:2";
  const codexForcePluginInstallRefreshIntervalMs = 1000;
  const codexPlusImageOverlayId = "codex-plus-image-overlay";
  window.__codexProjectMoveRuntimeId = (window.__codexProjectMoveRuntimeId || 0) + 1;
  const codexProjectMoveRuntimeId = window.__codexProjectMoveRuntimeId;
  clearTimeout(window.__codexProjectMoveProjectionTimer);
  clearTimeout(window.__codexProjectMoveChatsSortTimer);
  window.__codexProjectMoveProjectionTimer = null;
  window.__codexProjectMoveChatsSortTimer = null;
  clearTimeout(window.__codexThreadScrollSaveTimer);
  window.__codexThreadScrollSaveTimer = null;
  (window.__codexThreadScrollRestoreTimers || []).forEach((timer) => clearTimeout(timer));
  window.__codexThreadScrollRestoreTimers = [];
  (window.__codexThreadScrollSyncTimers || []).forEach((timer) => clearTimeout(timer));
  window.__codexThreadScrollSyncTimers = [];
  window.__codexThreadScrollRestoreRevision = (window.__codexThreadScrollRestoreRevision || 0) + 1;

  function installCodexPlusImageOverlay() {
    const config = window.__CODEX_PLUS_IMAGE_OVERLAY__ || {};
    const canQueryById = typeof document?.getElementById === "function";
    const existing = canQueryById ? document.getElementById(codexPlusImageOverlayId) : null;
    const source = config.dataUrl || "";
    if (!config.enabled || !source) {
      if (window.__codexPlusImageOverlayBlobUrl) {
        URL.revokeObjectURL(window.__codexPlusImageOverlayBlobUrl);
        window.__codexPlusImageOverlayBlobUrl = "";
      }
      if (existing) existing.remove();
      return;
    }
    const root = document?.documentElement;
    if (!root || typeof document?.createElement !== "function") {
      return;
    }
    const opacity = Math.min(1, Math.max(0.01, Number(config.opacity) || 0.35));
    const image = existing || document.createElement("img");
    image.id = codexPlusImageOverlayId;
    image.src = source;
    image.alt = "";
    image.setAttribute("aria-hidden", "true");
    Object.assign(image.style, {
      position: "fixed",
      inset: "0",
      width: "100vw",
      height: "100vh",
      objectFit: "contain",
      objectPosition: "center center",
      opacity: String(opacity),
      pointerEvents: "none",
      zIndex: "2147483646",
      userSelect: "none",
    });
    if (!existing) root.appendChild(image);
    sendCodexPlusDiagnostic("image_overlay_installed", {
      opacity,
      sourceKind: source.startsWith("data:") ? "data-uri" : "unknown",
    });
  }

  function scheduleCodexPlusImageOverlay() {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", installCodexPlusImageOverlay, { once: true });
      return;
    }
    installCodexPlusImageOverlay();
    setTimeout(installCodexPlusImageOverlay, 250);
  }

  scheduleCodexPlusImageOverlay();
  window.__codexThreadScrollSyncRevision = (window.__codexThreadScrollSyncRevision || 0) + 1;
  let upstreamBranchDefaultsCache = new Map();
  const upstreamBranchDefaultsCacheTtlMs = 5000;
  const upstreamRemoteBranchDefaultsCacheTtlMs = 30000;
  let upstreamBranchDefaultsInflight = new Map();
  const upstreamProjectContextTtlMs = 10 * 60 * 1000;
  const branchWorktreePathAttribute = "data-codex-branch-worktree-path";
  ["__codexPlusHtmlCenteredThreadWidth", "__codexPlusViewportCenteredThreadWidth", "__codexPlusBoundedThreadCenter"].forEach((key) => {
    try {
      window[key]?.cleanup?.();
    } catch (_) {}
  });
  try {
    window.__codexPlusConversationViewCleanup?.();
  } catch (_) {}
  window.__codexPlusConversationViewCleanup = null;
  const selectors = {
    sidebarThread: "[data-app-action-sidebar-thread-id]",
    threadTitle: "[data-thread-title]",
    appHeader: ".app-header-tint",
    nativeMenuBar: "[class*=\"ms-auto\"][class*=\"flex\"][class*=\"items-center\"]",
    headerContextMenuSurface: '[data-testid="app-shell-header-context-menu-surface"]',
    archiveNav: 'button[aria-label="已归档对话"], button[aria-label="Archived conversations"]',
    disabledInstallButton: 'button:disabled, button[aria-disabled="true"], [role="button"][aria-disabled="true"], button[data-disabled], [role="button"][data-disabled], button.cursor-not-allowed, [role="button"].cursor-not-allowed, button.pointer-events-none, [role="button"].pointer-events-none',
    pluginNavButton: 'nav[role="navigation"] button.h-token-nav-row.w-full',
    pluginSvgPath: 'svg path[d^="M7.94562 14.0277"]',
  };
  const headerContextButtonClass = "border-token-border user-select-none no-drag cursor-interaction flex items-center gap-1 border whitespace-nowrap focus:outline-none disabled:cursor-not-allowed disabled:opacity-40 rounded-lg border-token-border text-token-button-tertiary-foreground bg-token-bg-fog enabled:hover:bg-token-list-hover-background data-[state=open]:bg-token-list-hover-background border h-token-button-composer px-2 py-0 text-base leading-[18px]";
  const headerIconTextButtonClass = "border-token-border no-drag cursor-interaction flex items-center gap-1 border whitespace-nowrap select-none focus:outline-none disabled:cursor-not-allowed disabled:opacity-40 rounded-lg text-token-text-tertiary enabled:hover:bg-token-list-hover-background data-[state=open]:bg-token-list-hover-background border-transparent h-token-button-composer px-2 py-0 text-base leading-[18px]";

  function isCodexOverlayWindow() {
    return location.href.includes("initialRoute=%2Favatar-overlay") ||
      location.pathname.includes("/avatar-overlay") ||
      document.documentElement.classList.contains("compact-window");
  }

  function elementHasVisibleBox(element) {
    if (!element) return false;
    const rect = element.getBoundingClientRect?.();
    if (rect && (!(rect.width > 0) || !(rect.height > 0))) return false;
    const style = typeof getComputedStyle === "function" ? getComputedStyle(element) : null;
    if (style && (style.display === "none" || style.visibility === "hidden")) return false;
    return true;
  }

  function visibleElementText(element) {
    if (!elementHasVisibleBox(element)) return "";
    return String(element.textContent || element.getAttribute?.("aria-label") || "").replace(/\s+/g, " ").trim();
  }

  function hasVisibleButtonText(labels) {
    const expected = new Set(labels);
    return Array.from(document.querySelectorAll("button, [role='link'], a")).some((element) => {
      const text = visibleElementText(element);
      const aria = String(element.getAttribute?.("aria-label") || "").replace(/\s+/g, " ").trim();
      return expected.has(text) || expected.has(aria);
    });
  }

  function hasVisibleSettingsNavText(labels) {
    const expected = new Set(labels);
    return Array.from(document.querySelectorAll("nav button, [role='navigation'] button, aside button, button[aria-label]")).some((element) => {
      const text = visibleElementText(element);
      const aria = String(element.getAttribute?.("aria-label") || "").replace(/\s+/g, " ").trim();
      return expected.has(text) || expected.has(aria);
    });
  }

  function isCodexSettingsPage() {
    if (location.href.includes("initialRoute=%2Favatar-overlay")) return false;
    const hasBackToApp = hasVisibleButtonText(["Back to app", "返回应用"]);
    if (!hasBackToApp) return false;
    const primaryNavHits = [
      ["General", "常规"],
      ["Appearance", "外观"],
      ["Keyboard shortcuts", "键盘快捷键"],
    ].filter((labels) => hasVisibleSettingsNavText(labels)).length;
    return primaryNavHits >= 2;
  }

  function cleanupCodexOverlayWindowArtifacts() {
    document.querySelectorAll(`#${codexPlusMenuId}, [data-codex-plus-menu="true"], #${styleId}`).forEach((node) => node.remove());
  }

  if (isCodexOverlayWindow()) {
    cleanupCodexOverlayWindowArtifacts();
    return;
  }

  try {
    localStorage.removeItem("codexPlusLanguage");
  } catch (_) {}

  const codexPlusMessages = {
    en: {
      close: "Close",
      relaunchNotNeeded: "Not needed",
      backendChecking: "Checking backend...",
      backendConnected: "connected",
      backendDisconnected: "Not connected",
      backendTimeout: "Backend check timed out",
      backendRepairing: "Repairing backend...",
      backendRepairFailed: "Backend repair failed",
      backendIndicatorChecking: "Checking backend",
      backendIndicatorDisconnected: "Not connected",
      bridgeUnavailable: "Bridge unavailable. Please restart the launcher.",
      managerOpened: "Manager opened",
      managerOpenFailed: "Failed to open manager",
      userScriptLoaded: "Loaded",
      userScriptFailed: "Failed",
      userScriptDisabled: "Disabled",
      userScriptNotLoaded: "Not loaded",
      userScriptLoading: "Loading",
      userScriptUnknown: "Unknown",
      userScriptDirs: "Built-in: {builtin}  User: {user}",
      userScriptDirMissing: "Not found",
      userScriptEmpty: "No user scripts found.",
      userScriptBuiltin: "Built-in",
      userScriptUser: "User",
      visitHost: "Visit {host}",
      adsLoading: "Recommendations are loading...",
      adsEmpty: "No recommendations yet.",
      sponsorAdsTitle: "Sponsor recommendations",
      sponsorAdsEmpty: "No sponsor recommendations yet.",
      normalAdsTitle: "General recommendations",
      normalAdsEmpty: "No general recommendations yet.",
      tabHome: "Home",
      tabUserScripts: "User Scripts",
      tabSponsor: "Recommendations",
      tabSupport: "Support",
      backendTitle: "Backend connection",
      backendDescription: "Checks launcher backend status every 5 seconds. If disconnected, Codex++ can try to repair the backend.",
      backendRepair: "Repair backend",
      enhancementsTitle: "Page enhancements",
      enhancementsDescription: "When off, delete, export, move, Timeline, plugin enhancements, and menu positioning are disabled.",
      marketplaceTitle: "Unlock plugin marketplace",
      marketplaceRelayDescription: "Not needed in compatibility mode. Your ChatGPT login keeps the official plugin marketplace available.",
      marketplacePatchDescription: "In API Key mode, expands plugin marketplace requests to show as many plugins as possible.",
      pluginEntryTitle: "Force plugin entry",
      pluginEntryRelayDescription: "Not needed in compatibility mode. The official login state keeps the plugin entry available.",
      pluginEntryPatchDescription: "Restores the 1.1.9 entry unlock behavior to force the plugin entry visible and enabled.",
      forcePluginInstallTitle: "Force install special plugins",
      forcePluginInstallRelayDescription: "Not needed in compatibility mode. Plugin install entry points are left unchanged.",
      forcePluginInstallPatchDescription: "Removes front-end install blocking caused by App unavailable states.",
      modelWhitelistTitle: "Unlock model allowlist",
      modelWhitelistDescription: "Fetches models from the relay /v1/models endpoint in environment variables and Codex config.toml, then adds them to the model selector.",
      fastButtonTitle: "Fast button",
      fastButtonDescription: "Shows the service tier switch. Fast only supports {models}; other models are sent as Standard.",
      serviceTierTitle: "Service tier",
      serviceTierDescription: "Inherit uses config.toml service tier. Global mode applies to all threads. Custom mode allows per-thread overrides.",
      serviceTierInherit: "Inherit",
      serviceTierGlobalStandard: "Global Standard",
      serviceTierGlobalFast: "Global Fast",
      serviceTierCustom: "Custom",
      serviceTierThreadOverride: "Current thread override",
      serviceTierThreadInheritTitle: "Do not override this thread; inherit config.toml",
      serviceTierThreadStandardTitle: "Use Standard for this thread only and switch to Custom mode",
      serviceTierThreadFastTitle: "Use Fast for this thread only and switch to Custom mode",
      sessionDeleteTitle: "Session delete",
      sessionDeleteDescription: "Shows a delete button on hovered sessions and supports undo.",
      markdownExportTitle: "Markdown export",
      markdownExportDescription: "Shows an export button on sessions and exports timestamped Markdown using the local rollout.",
      pasteFixTitle: "Paste fix",
      pasteFixDescription: "Keeps only plain text when pasting from rich text sources such as Word into the Codex composer, preventing accidental image or file attachments. Restart Codex to apply.",
      projectMoveTitle: "Move sessions between projects",
      projectMoveDescription: "Shows a move button on hovered sessions. Move a session to Chats or another local project.",
      timelineTitle: "Conversation Timeline",
      timelineDescription: "Shows a question timeline beside the conversation. Hover for summaries, click to jump.",
      threadIdBadgeTitle: "Session ID badge",
      threadIdBadgeDescription: "Shows a short ID and UUIDv7 creation time before sidebar session titles to make historical sessions easier to identify.",
      conversationViewTitle: "Centered conversation width",
      conversationViewDescription: "Limits the main conversation and composer to a fixed maximum width for easier reading on large screens.",
      threadScrollTitle: "Restore scroll when switching threads",
      threadScrollDescription: "Restores each thread to the last read position instead of always jumping to the bottom.",
      zedRemoteTitle: "Zed Remote open",
      zedRemoteDescription: "Open supported remote SSH file references in Zed without patching Codex.app.",
      upstreamWorktreeTitle: "Upstream worktree",
      upstreamWorktreeDescription: "Create a Git worktree from a fresh upstream branch, equivalent to git worktree add -b branch path upstream/base.",
      upstreamWorktreeDialogLabel: "Create upstream worktree",
      upstreamWorktreeDialogTitle: "Create from upstream",
      upstreamWorktreeDialogDescription: "Equivalent to git worktree add -b branch path upstream/base. Fetches the remote branch first.",
      upstreamRepoPathLabel: "Repository path",
      upstreamBranchNameLabel: "New branch name",
      upstreamWorktreePathLabel: "Worktree path",
      upstreamRemoteLabel: "Remote",
      upstreamBaseBranchLabel: "Base branch",
      upstreamDefaultsHint: "Enter a repository path to read the remote and current branch automatically.",
      upstreamReadDefaults: "Read defaults",
      upstreamDefaultsReading: "Reading repository defaults...",
      upstreamDefaultsFailed: "Failed to read repository defaults",
      upstreamDefaultsReady: "Worktree will be created from {remote}/{baseBranch}.",
      upstreamCreateMissingFields: "Repository path, branch name, worktree path, remote, and base branch are required.",
      upstreamCreating: "Fetching and creating worktree...",
      upstreamCreatedMessage: "Created from {sourceRef}: {worktreePath}",
      upstreamCreatedToast: "Created upstream worktree: {branchName}",
      upstreamCreatedNativeToast: "Created worktree from {sourceRef}",
      upstreamCreateFailed: "Failed to create upstream worktree",
      upstreamNativeFormUnknown: "Cannot safely read the native Codex worktree form. Use the Codex++ menu instead.",
      upstreamBranchInUse: "This branch is already used by another worktree: {path}",
      upstreamBranchWillCreate: "New worktree will be created from {label}",
      create: "Create",
      providerSyncTitle: "Historical session repair",
      providerSyncDescription: "After switching official login, hybrid API, or pure API modes, makes old conversations visible in the current mode again.",
      pageModeTitle: "Page enhancement mode",
      pageModeRelayDescription: "Compatibility mode keeps session delete, export, project move, Timeline, and user scripts, while disabling plugin-entry enhancements.",
      pageModePatchDescription: "Full mode loads all page capabilities, including plugin entry, forced install, and project path moving.",
      openManager: "Open manager",
      nativeMenuTitle: "Native menu bar position",
      nativeMenuDescription: "Insert the Codex++ menu into the native top menu bar. Enabled by default; can be turned off if page re-renders conflict.",
      devtoolsTitle: "Open DevTools",
      devtoolsDescription: "Open DevTools for the current Codex page to inspect user script errors.",
      openDevtools: "Open DevTools",
      aboutTitle: "About Codex++",
      aboutDescription: "Codex++ is an enhancement menu injected by an external launcher. It does not modify the original Codex App installation.",
      discordTitle: "Discord community",
      discordDescription: "Join Discord for updates, issue feedback, and usage discussions.",
      openDiscord: "Open Discord",
      telegramTitle: "Telegram channel",
      telegramDescription: "Join Telegram for updates and usage discussions.",
      openTelegram: "Open Telegram",
      issueTitle: "Report an issue",
      issueDescription: "Open GitHub Issues to report bugs or suggestions.",
      openIssue: "Report issue",
      userScriptsTitle: "User Scripts",
      userScriptsDescription: "Enable user scripts. Codex++ automatically loads .js files from the built-in directory and user config directory.",
      userScriptsWarning: "After disabling, reload the page or restart Codex++ to fully remove effects from scripts that already ran.",
      userScriptsReadingDirs: "Reading script directories...",
      userScriptsReading: "Reading user scripts...",
      reloadUserScripts: "Reload user scripts",
      sponsorDescription: "Recommendations include sponsor recommendations and general recommendations. Sponsor recommendations come from partners supporting Codex++ maintenance. General recommendations highlight services and information useful to Codex users.",
      supportDescription: "If Codex++ has helped you, you can buy me a coffee or send a small tip to support continued maintenance.",
      alipay: "Alipay",
      wechat: "WeChat",
      alipayQrAlt: "Alipay tip QR code",
      wechatQrAlt: "WeChat tip QR code",
      serviceTierDisabled: "Disabled",
      serviceTierReading: "Reading...",
      serviceTierReadFailed: "Read failed",
      serviceTierNotRead: "Not read",
      serviceTierGlobalFastOn: "Fast is on",
      serviceTierGlobalDefault: "Default service tier",
      serviceTierCurrentValue: "Current: {value}",
      serviceTierInheritStatus: "Inherit config.toml: {mode}",
      serviceTierCustomDefault: "Custom: default {mode}",
      serviceTierCustomThread: "Custom: current thread {mode}",
      serviceTierBackendDisconnectedToast: "Backend is not connected; service tier cannot be changed",
      serviceTierToast: "Service tier: {mode}",
      serviceTierModeInherit: "Inherit config.toml",
      serviceTierModeGlobalStandard: "Global Standard",
      serviceTierModeGlobalFast: "Global Fast",
      serviceTierModeCustom: "Custom",
      serviceTierThreadTargetCurrent: "Current thread",
      serviceTierThreadTargetDraft: "New thread draft",
      serviceTierThreadToast: "{target} service tier: {mode}",
      serviceTierLoadingTitle: "Service tier: checking backend connection",
      serviceTierFailedTitle: "Service tier: backend is not connected, cannot switch",
      serviceTierReadingTitle: "Service tier: reading",
      serviceTierReadFailedTitle: "Service tier: read failed",
      serviceTierBadgeTitlePrefix: "Service tier: {scope}",
      serviceTierStandardHelp: "Standard: uses standard processing and does not set priority on requests.",
      serviceTierFastHelp: "Fast: only supports {models}; uses service_tier=\"priority\" for supported models. Official docs say latency is lower and more consistent, with higher pricing. Rate limits are shared with Standard, and traffic spikes may fall back to Standard.",
      serviceTierUnsupportedBadge: "unsupported",
      serviceTierUnsupportedSuffix: "{message}; current requests will be sent as Standard.",
      serviceTierFastUnsupported: "Fast only supports {models}; {modelText}",
      serviceTierFastUnsupportedModel: "current model {model} is not supported",
      serviceTierFastUnsupportedUnknown: "current model was not read",
      serviceTierFastTitle: "Fast: use service_tier=\"priority\"",
      serviceTierThreadInheritDynamicTitle: "Do not override this thread; inherit custom default {mode}",
      upstreamWorktreeDisabled: "Upstream worktree enhancement is disabled",
      cancel: "Cancel",
      delete: "Delete",
      deleteSessionDialogLabel: "Delete session",
      deleteSessionTitle: "Delete session",
      deleteSessionMessage: "Delete \"{title}\"?",
      deleteSuccess: "Deleted",
      deleteFailed: "Delete failed",
      exportSuccess: "Exported",
      exportFailed: "Export failed",
      exportCancelled: "Export cancelled",
      exportArchivedMissingId: "Export failed: archived session ID was not found",
      moveLabel: "Move",
      movingLabel: "Moving",
      moveToChatsSuccess: "Moved to Chats: \"{title}\"",
      moveToProjectSuccess: "Moved to \"{project}\": \"{title}\"",
      moveFailed: "Move failed: {message}",
      moveDialogLabel: "Move conversation",
      moveDialogTitle: "Move \"{title}\"",
      moveTargetsLoading: "Loading projects...",
      moveTargetsEmpty: "No targets available",
      moveTargetsLoadFailed: "Failed to load projects: {message}",
      sidebarMoreActions: "More actions",
      sidebarDelete: "Delete",
      sidebarExport: "Export",
      sidebarMove: "Move",
    },
    zh: {
      close: "关闭",
      relaunchNotNeeded: "无需开启",
      backendChecking: "正在检查后端…",
      backendConnected: "已连接",
      backendDisconnected: "未连接",
      backendTimeout: "后端检查超时",
      backendRepairing: "正在修复后端…",
      backendRepairFailed: "后端修复失败",
      backendIndicatorChecking: "正在检查后端",
      backendIndicatorDisconnected: "未连接",
      bridgeUnavailable: "桥接不可用，请重启启动器",
      managerOpened: "管理工具已打开",
      managerOpenFailed: "打开管理工具失败",
      userScriptLoaded: "已加载",
      userScriptFailed: "失败",
      userScriptDisabled: "已禁用",
      userScriptNotLoaded: "未加载",
      userScriptLoading: "加载中",
      userScriptUnknown: "未知",
      userScriptDirs: "内置：{builtin}  用户：{user}",
      userScriptDirMissing: "未找到",
      userScriptEmpty: "未发现用户脚本。",
      userScriptBuiltin: "内置",
      userScriptUser: "用户",
      visitHost: "访问 {host}",
      adsLoading: "推荐内容加载中…",
      adsEmpty: "暂无推荐内容。",
      sponsorAdsTitle: "赞助商推荐",
      sponsorAdsEmpty: "暂无赞助商推荐。",
      normalAdsTitle: "普通推荐",
      normalAdsEmpty: "暂无普通推荐。",
      tabHome: "主页",
      tabUserScripts: "用户脚本",
      tabSponsor: "推荐内容",
      tabSupport: "请作者喝咖啡",
      backendTitle: "后端连接",
      backendDescription: "每 5 秒检查一次 launcher 后端状态；断开时可尝试修复后端运行。",
      backendRepair: "修复后端运行",
      enhancementsTitle: "页面功能增强",
      enhancementsDescription: "关闭后停用删除、导出、移动、Timeline、插件相关和菜单位置增强。",
      marketplaceTitle: "插件市场解锁",
      marketplaceRelayDescription: "兼容增强模式下无需开启；ChatGPT 登录态会保留官方插件市场。",
      marketplacePatchDescription: "API Key 模式下扩展插件市场请求，尽量显示完整插件列表。",
      pluginEntryTitle: "强制解锁入口",
      pluginEntryRelayDescription: "兼容增强模式下无需开启；官方登录态会保留插件入口。",
      pluginEntryPatchDescription: "恢复 1.1.9 的入口解锁方式，强制显示并启用插件入口。",
      forcePluginInstallTitle: "特殊插件强制安装",
      forcePluginInstallRelayDescription: "兼容增强模式下无需开启；不会改插件安装入口。",
      forcePluginInstallPatchDescription: "解除 App unavailable / 应用不可用导致的前端安装禁用。",
      modelWhitelistTitle: "模型白名单解锁",
      modelWhitelistDescription: "从环境变量和 Codex config.toml 中的中转站 /v1/models 拉取模型，并补进模型选择列表。",
      fastButtonTitle: "Fast 按钮",
      fastButtonDescription: "显示服务模式切换按钮；Fast 仅支持 {models}，其他模型按 Standard 发送。",
      serviceTierTitle: "服务模式",
      serviceTierDescription: "继承使用 config.toml 的 service tier；全局模式覆盖全部 thread；自定义允许按 thread 覆盖。",
      serviceTierInherit: "继承",
      serviceTierGlobalStandard: "全局 Standard",
      serviceTierGlobalFast: "全局 Fast",
      serviceTierCustom: "自定义",
      serviceTierThreadOverride: "当前 thread 覆盖",
      serviceTierThreadInheritTitle: "当前 thread 不单独覆盖，继承 config.toml",
      serviceTierThreadStandardTitle: "仅当前 thread 使用 Standard，并切到自定义模式",
      serviceTierThreadFastTitle: "仅当前 thread 使用 Fast，并切到自定义模式",
      sessionDeleteTitle: "会话删除",
      sessionDeleteDescription: "在会话列表悬停显示删除按钮，并支持撤销。",
      markdownExportTitle: "Markdown 导出",
      markdownExportDescription: "在会话列表显示导出按钮，按本地 rollout 导出带时间戳的 Markdown。",
      pasteFixTitle: "粘贴修复",
      pasteFixDescription: "从 Word 等富文本来源粘贴到 Codex composer 时只保留纯文本，避免被识别为图片/文件附件。需重启 Codex 才生效。",
      projectMoveTitle: "会话项目移动",
      projectMoveDescription: "在会话列表悬停显示移动按钮，可移动到普通对话或其他本地项目。",
      timelineTitle: "对话 Timeline",
      timelineDescription: "在对话右侧显示用户提问时间线，悬停查看摘要，点击跳转。",
      threadIdBadgeTitle: "会话 ID 标识",
      threadIdBadgeDescription: "在侧边栏会话标题前显示短 ID 和 UUIDv7 创建时间，方便定位历史会话。",
      conversationViewTitle: "对话居中宽度",
      conversationViewDescription: "开启后把主对话和输入框限制到固定最大宽度，适合大屏阅读。",
      threadScrollTitle: "切换对话保留位置",
      threadScrollDescription: "开启后在不同 thread 之间切换时恢复到上一次浏览位置，不再自动跳到底部。",
      zedRemoteTitle: "Zed Remote 打开",
      zedRemoteDescription: "不修改 Codex.app，直接用 Zed 打开支持的远程 SSH 文件引用。",
      upstreamWorktreeTitle: "上游 Worktree",
      upstreamWorktreeDescription: "基于最新上游分支创建 Git worktree，等价于 git worktree add -b branch path upstream/base。",
      upstreamWorktreeDialogLabel: "创建上游 worktree",
      upstreamWorktreeDialogTitle: "从上游创建",
      upstreamWorktreeDialogDescription: "等价于 git worktree add -b branch path upstream/base。创建前会先 fetch 远端分支。",
      upstreamRepoPathLabel: "仓库路径",
      upstreamBranchNameLabel: "新分支名",
      upstreamWorktreePathLabel: "Worktree 路径",
      upstreamRemoteLabel: "Remote",
      upstreamBaseBranchLabel: "Base branch",
      upstreamDefaultsHint: "填写仓库路径后会自动读取 remote 和当前分支。",
      upstreamReadDefaults: "读取默认值",
      upstreamDefaultsReading: "正在读取仓库默认值…",
      upstreamDefaultsFailed: "读取仓库默认值失败",
      upstreamDefaultsReady: "将从 {remote}/{baseBranch} 创建 worktree。",
      upstreamCreateMissingFields: "仓库路径、分支名、worktree 路径、remote 和 base branch 都必须填写。",
      upstreamCreating: "正在 fetch 并创建 worktree…",
      upstreamCreatedMessage: "已从 {sourceRef} 创建：{worktreePath}",
      upstreamCreatedToast: "已创建 upstream worktree：{branchName}",
      upstreamCreatedNativeToast: "已从 {sourceRef} 创建 worktree",
      upstreamCreateFailed: "创建 upstream worktree 失败",
      upstreamNativeFormUnknown: "无法安全识别 Codex 原生 worktree 表单，请使用 Codex++ 菜单创建。",
      upstreamBranchInUse: "该分支已在另一个 worktree 使用：{path}",
      upstreamBranchWillCreate: "将从 {label} 创建新 worktree",
      create: "创建",
      providerSyncTitle: "历史会话修复",
      providerSyncDescription: "切换官方登录、混合 API 或纯 API 后，让旧对话重新显示在当前模式下。",
      pageModeTitle: "页面增强模式",
      pageModeRelayDescription: "兼容增强：保留会话删除、导出、项目移动、Timeline 和用户脚本，仅关闭插件入口相关增强。",
      pageModePatchDescription: "完整增强：加载插件入口、强制安装、项目路径移动等全部页面能力。",
      openManager: "打开管理工具",
      nativeMenuTitle: "原生菜单栏位置",
      nativeMenuDescription: "把 Codex++ 菜单插入顶部原生菜单栏；默认开启；如页面重渲染冲突可关闭。",
      devtoolsTitle: "打开 DevTools",
      devtoolsDescription: "打开当前 Codex 页面开发者工具，方便查看用户脚本报错。",
      openDevtools: "打开 DevTools",
      aboutTitle: "关于 Codex++",
      aboutDescription: "Codex++ 是通过外部 launcher 注入的增强菜单，不修改 Codex App 原始安装文件。",
      discordTitle: "Discord 社区",
      discordDescription: "加入 Discord 获取更新消息、反馈问题或交流使用体验。",
      openDiscord: "打开 Discord",
      telegramTitle: "Telegram 频道",
      telegramDescription: "加入 Telegram 获取更新消息和交流使用体验。",
      openTelegram: "打开 Telegram",
      issueTitle: "提出问题",
      issueDescription: "打开 GitHub Issues 反馈问题或建议。",
      openIssue: "提出问题",
      userScriptsTitle: "用户脚本",
      userScriptsDescription: "启用用户脚本：自动加载内置目录和用户配置目录中的 .js 文件。",
      userScriptsWarning: "禁用后需重载页面或重启 Codex++ 才能完全移除已执行效果。",
      userScriptsReadingDirs: "正在读取脚本目录…",
      userScriptsReading: "正在读取用户脚本…",
      reloadUserScripts: "重新加载用户脚本",
      sponsorDescription: "推荐内容分为赞助商推荐和普通推荐。赞助商推荐来自支持 Codex++ 继续维护的合作方；普通推荐用于展示适合 Codex 用户的服务与信息。",
      supportDescription: "如果 Codex++ 帮到了你，可以请我喝杯咖啡，或者随手赞赏支持一下继续维护。",
      alipay: "支付宝",
      wechat: "微信",
      alipayQrAlt: "支付宝赞赏码",
      wechatQrAlt: "微信赞赏码",
      serviceTierDisabled: "未启用",
      serviceTierReading: "正在读取…",
      serviceTierReadFailed: "读取失败",
      serviceTierNotRead: "未读取",
      serviceTierGlobalFastOn: "Fast 已开启",
      serviceTierGlobalDefault: "默认服务模式",
      serviceTierCurrentValue: "当前：{value}",
      serviceTierInheritStatus: "继承 config.toml：{mode}",
      serviceTierCustomDefault: "自定义：默认 {mode}",
      serviceTierCustomThread: "自定义：当前 thread {mode}",
      serviceTierBackendDisconnectedToast: "后端未连接，无法切换服务模式",
      serviceTierToast: "服务模式：{mode}",
      serviceTierModeInherit: "继承 config.toml",
      serviceTierModeGlobalStandard: "全局 Standard",
      serviceTierModeGlobalFast: "全局 Fast",
      serviceTierModeCustom: "自定义",
      serviceTierThreadTargetCurrent: "当前 thread",
      serviceTierThreadTargetDraft: "新 thread 草稿",
      serviceTierThreadToast: "{target}服务模式：{mode}",
      serviceTierLoadingTitle: "服务模式：正在检查后端连接",
      serviceTierFailedTitle: "服务模式：后端未连接，无法切换",
      serviceTierReadingTitle: "服务模式：正在读取",
      serviceTierReadFailedTitle: "服务模式：读取失败",
      serviceTierBadgeTitlePrefix: "服务模式：{scope}",
      serviceTierStandardHelp: "Standard：使用标准处理；不在请求上设置 priority。",
      serviceTierFastHelp: "Fast：仅支持 {models}；对支持模型使用 service_tier=\"priority\"，官方说明其延迟更低且更一致，但会按更高价格计费；rate limit 与 Standard 共享，流量快速上涨时可能回落到 Standard。",
      serviceTierUnsupportedBadge: "不支持",
      serviceTierUnsupportedSuffix: "{message}；当前请求会按 Standard 发送。",
      serviceTierFastUnsupported: "Fast 仅支持 {models}，{modelText}",
      serviceTierFastUnsupportedModel: "当前模型 {model} 不支持",
      serviceTierFastUnsupportedUnknown: "当前模型未读取",
      serviceTierFastTitle: "Fast：使用 service_tier=\"priority\"",
      serviceTierThreadInheritDynamicTitle: "当前 thread 不单独覆盖，继承自定义默认 {mode}",
      upstreamWorktreeDisabled: "上游 worktree 增强已关闭",
      cancel: "取消",
      delete: "删除",
      deleteSessionDialogLabel: "删除会话",
      deleteSessionTitle: "删除会话",
      deleteSessionMessage: "删除“{title}”？",
      deleteSuccess: "删除成功",
      deleteFailed: "删除失败",
      exportSuccess: "导出成功",
      exportFailed: "导出失败",
      exportCancelled: "导出已取消",
      exportArchivedMissingId: "导出失败：未找到归档会话 ID",
      moveLabel: "移动",
      movingLabel: "移动中",
      moveToChatsSuccess: "已移动到普通对话：“{title}”",
      moveToProjectSuccess: "已移动到“{project}”：“{title}”",
      moveFailed: "移动失败：{message}",
      moveDialogLabel: "移动对话",
      moveDialogTitle: "移动“{title}”",
      moveTargetsLoading: "加载项目中...",
      moveTargetsEmpty: "没有可用目标",
      moveTargetsLoadFailed: "加载项目失败：{message}",
      sidebarMoreActions: "更多操作",
      sidebarDelete: "删除",
      sidebarExport: "导出",
      sidebarMove: "移动",
    },
  };

  const codexPlusLanguageStorageKeyPattern = /language|locale|lng|i18n/i;
  let codexPlusLanguageCache = { at: 0, value: "" };

  function codexPlusLanguageCodeFromValue(value, depth = 0) {
    if (value == null || depth > 4) return "";
    if (typeof value === "string") {
      const normalized = value.trim().toLowerCase();
      if (!normalized) return "";
      if (/^(zh|zh[-_](cn|hans|hant|tw|hk)|cn|chinese|中文|简体中文|繁體中文)$/.test(normalized)) return "zh";
      if (/^(en|en[-_](us|gb)|english)$/.test(normalized)) return "en";
      if (/^zh[-_]/.test(normalized)) return "zh";
      if (/^en[-_]/.test(normalized)) return "en";
      const sample = normalized.slice(0, 4096);
      if (/"(?:language|locale|lng|i18nextlng)"\s*:\s*"(zh|zh[-_][^"]*)"/i.test(sample)) return "zh";
      if (/"(?:language|locale|lng|i18nextlng)"\s*:\s*"(en|en[-_][^"]*)"/i.test(sample)) return "en";
      if ((sample.startsWith("{") && sample.endsWith("}")) || (sample.startsWith("[") && sample.endsWith("]"))) {
        try {
          return codexPlusLanguageCodeFromValue(JSON.parse(value), depth + 1);
        } catch (_) {}
      }
      return "";
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        const language = codexPlusLanguageCodeFromValue(item, depth + 1);
        if (language) return language;
      }
      return "";
    }
    if (typeof value === "object") {
      for (const [key, nested] of Object.entries(value)) {
        const keyMatches = codexPlusLanguageStorageKeyPattern.test(String(key));
        const language = keyMatches
          ? codexPlusLanguageCodeFromValue(nested, depth + 1)
          : (depth < 2 ? codexPlusLanguageCodeFromValue(nested, depth + 1) : "");
        if (language) return language;
      }
    }
    return "";
  }

  function codexPlusRuntimeLanguageHint() {
    const candidates = [
      window.i18next?.language,
      window.i18n?.language,
      window.__i18n?.language,
      window.__locale,
      window.__NEXT_DATA__?.locale,
      window.__NEXT_DATA__?.props?.pageProps?.locale,
    ];
    for (const candidate of candidates) {
      const language = codexPlusLanguageCodeFromValue(candidate);
      if (language) return language;
    }
    return "";
  }

  function codexPlusStoredLanguageHint() {
    for (const storage of [window.localStorage, window.sessionStorage]) {
      if (!storage) continue;
      try {
        for (let index = 0; index < storage.length; index += 1) {
          const key = storage.key(index) || "";
          const value = storage.getItem(key) || "";
          if (!codexPlusLanguageStorageKeyPattern.test(key) && !codexPlusLanguageStorageKeyPattern.test(value.slice(0, 1024))) continue;
          const language = codexPlusLanguageCodeFromValue(value) || codexPlusLanguageCodeFromValue(key);
          if (language) return language;
        }
      } catch (_) {}
    }
    return "";
  }

  function codexPlusOwnsLanguageNode(element) {
    return !!element?.closest?.(`#${codexPlusMenuId}, .codex-plus-modal-overlay, .codex-delete-confirm-overlay, .${projectMoveOverlayClass}, .${moreMenuClass}, .${actionButtonClass}, .${actionGroupClass}, .${actionTooltipClass}, .${timelineClass}, .codex-conversation-timeline, .${codexServiceTierBadgeClass}, .${zedRemoteButtonClass}, .${zedRemoteToastClass}, .codex-delete-toast`);
  }

  const codexPlusNativeLanguageSelectors = [
    "button[aria-label]",
    "[role='button'][aria-label]",
    "a[aria-label]",
    "[role='link'][aria-label]",
    "input[placeholder]",
    "textarea[placeholder]",
    "header button",
    ".app-header-tint button",
    "nav button",
    "[role='navigation'] button",
    "[data-testid='app-shell-header-context-menu-surface'] [role='menuitem']",
  ].join(",");

  const codexPlusNativeLanguageSignals = {
    en: {
      strong: ["Back to app", "Default permissions", "New chat", "Archived conversations", "Unarchive"],
      weak: ["General", "Appearance", "Keyboard shortcuts", "Settings", "Language", "Search", "Plugins", "Automations", "Projects", "Pinned"],
    },
    zh: {
      strong: ["返回应用", "默认权限", "新对话", "新建对话", "已归档对话", "取消归档"],
      weak: ["常规", "外观", "键盘快捷键", "设置", "语言", "搜索", "插件", "自动化", "项目", "置顶"],
    },
  };

  function codexPlusNormalizeNativeLanguageText(value) {
    return String(value || "").replace(/\s+/g, " ").trim();
  }

  function codexPlusVisibleNativeLanguageValues(element) {
    if (!element || codexPlusOwnsLanguageNode(element) || !elementHasVisibleBox(element)) return [];
    const values = [];
    for (const attribute of ["aria-label", "title", "placeholder"]) {
      const value = codexPlusNormalizeNativeLanguageText(element.getAttribute?.(attribute));
      if (value) values.push(value);
    }
    const text = codexPlusNormalizeNativeLanguageText(element.textContent);
    if (text) values.push(text);
    return values;
  }

  function codexPlusNativeLanguageScore(language, values) {
    const signals = codexPlusNativeLanguageSignals[language];
    if (!signals) return 0;
    let score = 0;
    const matchesSignal = (value, signal) => {
      const text = codexPlusNormalizeNativeLanguageText(value).toLowerCase();
      const needle = codexPlusNormalizeNativeLanguageText(signal).toLowerCase();
      return !!needle && (text === needle || text.includes(needle));
    };
    for (const value of new Set(values)) {
      if (signals.strong.some((signal) => matchesSignal(value, signal))) score += 2;
      else if (signals.weak.some((signal) => matchesSignal(value, signal))) score += 1;
    }
    return score;
  }

  function codexPlusNativeTextLanguageHint() {
    const values = [];
    document.querySelectorAll?.(codexPlusNativeLanguageSelectors).forEach((element) => {
      values.push(...codexPlusVisibleNativeLanguageValues(element));
    });
    const zhScore = codexPlusNativeLanguageScore("zh", values);
    const enScore = codexPlusNativeLanguageScore("en", values);
    if (zhScore > enScore) return "zh";
    if (enScore > zhScore) return "en";
    return "";
  }

  function detectCodexPlusLanguage() {
    const rootLang = String(document.documentElement.lang || "").trim().toLowerCase();
    return codexPlusRuntimeLanguageHint()
      || codexPlusNativeTextLanguageHint()
      || codexPlusStoredLanguageHint()
      || (rootLang.startsWith("zh") ? "zh" : "")
      || "en";
  }

  function codexPlusLanguage() {
    try {
      const now = Date.now();
      if (codexPlusLanguageCache.value && now - codexPlusLanguageCache.at < 500) return codexPlusLanguageCache.value;
      const language = detectCodexPlusLanguage();
      codexPlusLanguageCache = { at: now, value: language };
      return language;
    } catch (_) {
      return "en";
    }
  }

  function codexPlusText(key, vars = {}) {
    const table = codexPlusMessages[codexPlusLanguage()] || codexPlusMessages.en;
    const fallback = codexPlusMessages.en[key] || key;
    return String(table[key] || fallback).replace(/\{([a-zA-Z0-9_]+)\}/g, (_, name) => {
      return Object.prototype.hasOwnProperty.call(vars, name) ? String(vars[name]) : "";
    });
  }

  function t(key, vars = {}) {
    return codexPlusText(key, vars);
  }

  function codexPlusEscapedText(key, vars = {}) {
    return escapeHtml(t(key, vars));
  }

  function installStyle() {
    const existingStyle = document.getElementById(styleId);
    if (existingStyle?.dataset.codexDeleteStyleVersion === codexDeleteStyleVersion) return;
    existingStyle?.remove();
    const style = document.createElement("style");
    style.id = styleId;
    style.dataset.codexDeleteStyleVersion = codexDeleteStyleVersion;
    style.textContent = `
      :root {
        --codex-plus-font-sans: var(--font-sans, var(--font-sans-default, var(--vscode-font-family, var(--default-font-family, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif))));
        --codex-plus-font-mono: var(--font-mono, var(--font-mono-default, var(--vscode-editor-font-family, var(--default-mono-font-family, ui-monospace, "SFMono-Regular", "SF Mono", Menlo, Consolas, "Liberation Mono", monospace))));
        --codex-plus-font-size: var(--vscode-font-size, var(--text-base, 14px));
        --codex-plus-font-size-xs: calc(var(--codex-plus-font-size) - 3px);
        --codex-plus-font-size-sm: calc(var(--codex-plus-font-size) - 2px);
        --codex-plus-font-size-md: var(--codex-plus-font-size);
        --codex-plus-font-size-lg: calc(var(--codex-plus-font-size) + 4px);
        --codex-plus-modal-overlay-bg: transparent;
        --codex-plus-modal-bg: var(--color-background-elevated-primary-opaque, var(--color-background-elevated-primary, var(--color-token-menu-background, var(--vscode-menu-background, Canvas))));
        --codex-plus-modal-fg: var(--color-text-foreground, var(--color-token-foreground, var(--vscode-foreground, CanvasText)));
        --codex-plus-modal-border: var(--color-border, var(--color-token-menu-border, var(--vscode-menu-border, color-mix(in srgb, currentColor 12%, transparent))));
        --codex-plus-modal-shadow: var(--shadow-2xl, 0 16px 32px -8px color-mix(in srgb, CanvasText 18%, transparent));
        --codex-plus-radius-control: var(--radius-md, 7px);
        --codex-plus-radius-card: var(--radius-lg, 12px);
        --codex-plus-radius-modal: var(--radius-2xl, 18px);
        --codex-plus-muted: var(--color-text-foreground-tertiary, var(--color-token-description-foreground, var(--vscode-descriptionForeground, color-mix(in srgb, var(--codex-plus-modal-fg) 58%, transparent))));
        --codex-plus-row-border: var(--color-border, var(--color-token-border, var(--vscode-widget-border, color-mix(in srgb, var(--codex-plus-modal-fg) 10%, transparent))));
        --codex-plus-control-border: var(--color-token-button-border, var(--color-border, var(--vscode-button-border, var(--codex-plus-row-border))));
        --codex-plus-control-bg: var(--color-background-button-secondary, var(--color-token-button-secondary-background, var(--vscode-button-secondaryBackground, ButtonFace)));
        --codex-plus-control-hover-bg: var(--color-background-button-secondary-hover, var(--color-token-button-secondary-hover-background, var(--vscode-button-secondaryHoverBackground, color-mix(in srgb, var(--codex-plus-modal-fg) 8%, var(--codex-plus-control-bg)))));
        --codex-plus-control-fg: var(--color-text-button-secondary, var(--color-token-button-secondary-foreground, var(--vscode-button-secondaryForeground, ButtonText)));
        --codex-plus-action-button-bg: color-mix(in srgb, var(--color-token-foreground, var(--codex-plus-modal-fg)) 5%, transparent);
        --codex-plus-action-button-hover-bg: color-mix(in srgb, var(--color-token-foreground, var(--codex-plus-modal-fg)) 10%, transparent);
        --codex-plus-action-button-fg: var(--color-token-foreground, var(--codex-plus-modal-fg));
        --codex-plus-action-button-border: transparent;
        --codex-plus-select-bg: var(--color-token-dropdown-background, var(--color-background-elevated-primary-opaque, var(--codex-plus-modal-bg)));
        --codex-plus-select-hover-bg: var(--color-token-list-hover-background, var(--codex-plus-control-hover-bg));
        --codex-plus-select-active-bg: var(--color-token-list-active-selection-background, var(--color-background-tertiary, var(--codex-plus-select-hover-bg)));
        --codex-plus-select-active-fg: var(--color-token-list-active-selection-foreground, var(--codex-plus-modal-fg));
        --codex-plus-focus-border: var(--color-token-focus-border, var(--color-border-focus, var(--vscode-focusBorder, var(--codex-plus-accent))));
        --codex-plus-checkbox-bg: var(--color-token-checkbox-background, var(--codex-plus-select-bg));
        --codex-plus-checkbox-border: var(--color-token-checkbox-border, var(--codex-plus-control-border));
        --codex-plus-primary-bg: var(--color-background-button-primary, var(--color-token-button-background, var(--vscode-button-background, var(--color-token-primary, var(--color-text-accent, Highlight)))));
        --codex-plus-primary-fg: var(--color-text-button-primary, var(--color-token-button-foreground, var(--vscode-button-foreground, HighlightText)));
        --codex-plus-input-bg: var(--color-token-input-background, var(--vscode-input-background, var(--color-background-control, var(--codex-plus-modal-bg))));
        --codex-plus-input-fg: var(--color-token-input-foreground, var(--vscode-input-foreground, var(--codex-plus-modal-fg)));
        --codex-plus-input-border: var(--color-token-input-border, var(--vscode-input-border, var(--codex-plus-control-border)));
        --codex-plus-toggle-bg: color-mix(in srgb, var(--color-token-foreground, var(--codex-plus-modal-fg)) 10%, transparent);
        --codex-plus-toggle-border: transparent;
        --codex-plus-toggle-knob-bg: var(--gray-0, var(--color-white, var(--white, #fff)));
        --codex-plus-toggle-knob-border: var(--gray-0, var(--color-white, var(--white, #fff)));
        --codex-plus-toggle-active-bg: var(--color-token-charts-blue, var(--color-text-accent, var(--color-token-primary, var(--codex-plus-primary-bg))));
        --codex-plus-toggle-active-border: var(--codex-plus-toggle-active-bg);
        --codex-plus-toggle-active-knob-bg: var(--codex-plus-toggle-knob-bg);
        --codex-plus-toggle-active-knob-border: var(--codex-plus-toggle-knob-border);
        --codex-plus-toggle-muted-bg: var(--color-background-button-secondary, var(--color-token-button-secondary-background, var(--codex-plus-control-bg)));
        --codex-plus-toggle-muted-fg: var(--color-text-foreground-tertiary, var(--color-token-description-foreground, var(--codex-plus-muted)));
        --codex-plus-tab-fg: var(--color-text-foreground-secondary, var(--color-token-text-secondary, var(--vscode-foreground, var(--codex-plus-modal-fg))));
        --codex-plus-scrollbar-thumb: var(--color-token-scrollbar-slider-background, color-mix(in srgb, var(--codex-plus-modal-fg) 18%, transparent));
        --codex-plus-scrollbar-thumb-hover: var(--color-token-scrollbar-slider-hover-background, color-mix(in srgb, var(--codex-plus-modal-fg) 26%, transparent));
        --codex-plus-card-bg: var(--color-background-elevated-secondary-opaque, var(--color-background-elevated-secondary, var(--color-token-menu-background, var(--vscode-menu-background, var(--codex-plus-modal-bg)))));
        --codex-plus-success: var(--color-text-success, var(--color-token-git-decoration-added-resource-foreground, var(--vscode-testing-coveredGutterBackground, var(--codex-plus-modal-fg))));
        --codex-plus-error: var(--color-text-error, var(--color-token-error-foreground, var(--vscode-errorForeground, var(--codex-plus-modal-fg))));
        --codex-plus-warning: var(--color-text-warning, var(--color-token-editor-warning-foreground, var(--vscode-editorWarning-foreground, var(--codex-plus-modal-fg))));
        --codex-plus-accent: var(--color-text-accent, var(--color-token-primary, var(--vscode-textLink-foreground, var(--codex-plus-primary-bg))));
        --codex-plus-accent-bg: var(--color-background-accent, var(--vscode-inputOption-activeBackground, var(--color-background-button-secondary, var(--codex-plus-control-bg))));
        --codex-plus-secondary: var(--color-text-foreground-secondary, var(--color-token-text-secondary, var(--vscode-foreground, var(--codex-plus-modal-fg))));
        --codex-plus-danger-bg: var(--color-background-button-danger, var(--color-background-danger-active, var(--color-background-status-error, var(--vscode-inputValidation-errorBackground, color-mix(in srgb, var(--codex-plus-error) 16%, var(--codex-plus-modal-bg))))));
        --codex-plus-danger-fg: var(--color-text-button-danger, var(--color-text-on-danger, var(--vscode-button-foreground, var(--codex-plus-primary-fg))));
        --codex-plus-danger-border: var(--color-border-danger, var(--vscode-inputValidation-errorBorder, color-mix(in srgb, var(--codex-plus-error) 42%, var(--codex-plus-modal-border))));
        --codex-plus-ad-card-bg: var(--color-background-elevated-secondary-opaque, var(--color-background-elevated-secondary, var(--color-token-diff-surface, var(--codex-plus-card-bg))));
        --codex-plus-ad-card-border: var(--codex-plus-row-border);
        --codex-plus-ad-card-shadow: var(--shadow-lg, var(--codex-plus-modal-shadow));
        --codex-plus-ad-chip-bg: var(--codex-plus-control-bg);
        --codex-plus-ad-chip-border: var(--codex-plus-control-border);
        --codex-plus-ad-chip-fg: var(--codex-plus-control-fg);
        --codex-plus-ad-empty-border: var(--codex-plus-row-border);
        --codex-plus-ad-empty-fg: var(--codex-plus-muted);
      }
      .${actionGroupClass} {
        position: absolute;
        right: var(--codex-session-actions-right, 28px);
        top: 50%;
        transform: translateY(-50%);
        z-index: 20;
        opacity: 0;
        pointer-events: none;
        display: inline-flex;
        align-items: center;
        gap: 4px;
        background: transparent;
      }
      .${actionButtonClass} {
        width: 20px;
        height: 20px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border: 1px solid transparent;
        border-radius: 999px;
        background: transparent;
        color: var(--color-text-foreground, var(--color-token-foreground, var(--codex-plus-modal-fg)));
        font: 430 var(--codex-plus-font-size-sm)/1 var(--codex-plus-font-sans);
        opacity: .5;
        padding: 0;
        cursor: default;
        text-align: center;
      }
      .${actionButtonClass} svg {
        display: block;
        width: 16px;
        height: 16px;
      }
      .${actionButtonClass}:hover,
      .${actionButtonClass}:focus-visible {
        background: transparent;
        color: var(--color-text-foreground, var(--color-token-foreground, var(--codex-plus-modal-fg)));
        opacity: 1;
        outline: none;
      }
      .${moreButtonClass}[aria-expanded="true"],
      [data-codex-delete-row="true"].codex-session-more-open .${moreButtonClass} {
        color: var(--color-text-foreground, var(--color-token-foreground, var(--codex-plus-modal-fg)));
        opacity: 1;
      }
      .${moreMenuClass} {
        position: fixed;
        z-index: 2147483201;
        min-width: 140px;
        border: 0;
        border-radius: var(--radius-xl, 15px);
        background: var(--color-token-dropdown-background, var(--codex-plus-modal-bg));
        color: var(--color-token-foreground, var(--codex-plus-modal-fg));
        box-shadow:
          0 0 0 .5px var(--color-token-border, var(--color-border, color-mix(in srgb, var(--codex-plus-modal-fg) 8%, transparent))),
          0 8px 16px -4px color-mix(in srgb, CanvasText 12%, transparent);
        display: flex;
        flex-direction: column;
        padding: 4px;
      }
      .${moreMenuClass}[hidden] { display: none !important; }
      .${moreMenuClass}.codex-session-more-menu-open-up {
        transform: translateY(calc(-100% - 34px));
      }
      .codex-session-more-menu-item {
        width: 100%;
        border: 0;
        border-radius: 12.5px;
        background: transparent;
        color: inherit;
        cursor: default;
        display: flex;
        align-items: center;
        gap: 8px;
        font: 430 var(--codex-plus-font-size-sm)/18.5714px var(--codex-plus-font-sans);
        min-height: 28.5703px;
        padding: 5px 8px;
        text-align: left;
      }
      .codex-session-more-menu-item:hover,
      .codex-session-more-menu-item:focus-visible {
        background: var(--color-token-list-hover-background, var(--codex-plus-control-hover-bg));
        outline: none;
      }
      .codex-session-more-menu-icon {
        width: 16px;
        height: 16px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        flex: 0 0 auto;
      }
      .${threadIdBadgeClass} {
        flex: 0 0 auto;
        display: inline-flex;
        align-items: center;
        max-width: 152px;
        margin-right: 8px;
        color: var(--text-secondary, var(--token-text-secondary, rgba(142,142,160,.95)));
        font: 11px/1.1 ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace;
        letter-spacing: .01em;
        opacity: .9;
        white-space: nowrap;
        user-select: text;
      }
      ${selectors.sidebarThread} [data-codex-thread-id-badge-wrap="true"] {
        display: inline-flex;
        align-items: center;
        min-width: 0;
        max-width: 100%;
      }
      ${selectors.sidebarThread} [data-codex-thread-id-badge-wrap="true"] ${selectors.threadTitle},
      ${selectors.sidebarThread} [data-codex-thread-id-badge-wrap="true"] .truncate.select-none,
      ${selectors.sidebarThread} [data-codex-thread-id-badge-wrap="true"] .truncate.text-base {
        min-width: 0;
      }
      .codex-archive-row-button {
        border: 1px solid #ef4444;
        border-radius: 7px;
        background: #f3f4f6;
        color: #374151;
        font: 12px system-ui, sans-serif;
        line-height: 16px;
        padding: 3px 8px;
        cursor: pointer;
      }
      .codex-archive-row-button.${buttonClass} {
        border-color: #ef4444;
        background: #fee2e2;
        color: #991b1b;
      }
      .codex-archive-row-button.${exportButtonClass} {
        border-color: #93c5fd;
        background: #dbeafe;
        color: #1d4ed8;
      }
      .codex-force-install-unlocked {
        border-color: #ef4444 !important;
        background: #fee2e2 !important;
        color: #991b1b !important;
        opacity: 1 !important;
      }
      .${zedRemoteButtonClass} {
        border: 1px solid #10a37f;
        border-radius: 7px;
        background: #d1fae5;
        color: #065f46;
        font: 12px system-ui, sans-serif;
        line-height: 16px;
        margin-left: 6px;
        padding: 2px 7px;
        cursor: pointer;
      }
      .${zedRemoteButtonClass}:hover,
      .${zedRemoteButtonClass}:focus-visible {
        background: #a7f3d0;
        outline: none;
      }
      .${zedRemoteOpenInMenuItemClass} {
        cursor: pointer;
      }
      .codex-zed-open-in-menu-icon {
        width: 18px;
        height: 18px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        object-fit: contain;
      }
      .${zedRemoteToastClass} {
        position: fixed;
        right: 18px;
        bottom: 58px;
        z-index: 2147483000;
        max-width: min(420px, calc(100vw - 36px));
        border-radius: 8px;
        background: #111827;
        color: #ffffff;
        font: 13px system-ui, sans-serif;
        line-height: 18px;
        padding: 10px 12px;
        box-shadow: 0 8px 30px rgba(0,0,0,.25);
        pointer-events: none;
      }
      [data-codex-delete-row="true"]:hover .${actionGroupClass} {
        opacity: 1;
        pointer-events: auto;
      }
      [data-codex-delete-row="true"].codex-session-more-open .${actionGroupClass} {
        opacity: 1;
        pointer-events: auto;
        z-index: 2147483201;
      }
      [data-codex-delete-row="true"].codex-archive-confirm-visible .${actionGroupClass} {
        right: max(66px, var(--codex-session-actions-right, 28px));
      }
      .${actionTooltipClass} {
        position: fixed;
        z-index: 2147483201;
        max-width: min(220px, calc(100vw - 32px));
        border: 1px solid var(--color-token-border, var(--color-border, var(--codex-plus-row-border)));
        border-radius: var(--radius-lg, 12px);
        background: var(--color-token-dropdown-background, var(--codex-plus-modal-bg));
        color: var(--color-token-foreground, var(--codex-plus-modal-fg));
        font: 430 var(--codex-plus-font-size-sm)/18px var(--codex-plus-font-sans);
        padding: 4px 8px;
        box-shadow: none;
        pointer-events: none;
        white-space: nowrap;
      }
      .${projectMoveOverlayClass} {
        position: fixed;
        inset: 0;
        z-index: 2147483200;
        background: var(--codex-plus-modal-overlay-bg);
      }
      .codex-project-move-panel {
        position: fixed;
        width: min(360px, calc(100vw - 32px));
        max-height: min(520px, calc(100vh - 32px));
        overflow: hidden;
        border: 1px solid var(--codex-plus-modal-border);
        border-radius: var(--codex-plus-radius-card);
        background: var(--codex-plus-modal-bg);
        color: var(--codex-plus-modal-fg);
        font: var(--codex-plus-font-size-sm)/1.35 var(--codex-plus-font-sans);
        box-shadow: var(--codex-plus-modal-shadow);
      }
      .codex-project-move-header { border-bottom: 1px solid var(--codex-plus-row-border); padding: 10px 12px; }
      .codex-project-move-title { font-weight: var(--font-weight-semibold, 600); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-project-move-list { max-height: min(440px, calc(100vh - 110px)); overflow-y: auto; padding: 6px; }
      .codex-project-move-item {
        display: block;
        width: 100%;
        border: 0;
        border-radius: var(--codex-plus-radius-control);
        background: transparent;
        color: var(--codex-plus-modal-fg);
        font: inherit;
        padding: 8px 9px;
        text-align: left;
        cursor: pointer;
      }
      .codex-project-move-item:hover,
      .codex-project-move-item:focus-visible { background: var(--codex-plus-control-hover-bg); outline: none; }
      .codex-project-move-item-title { font-weight: 550; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-project-move-item-path { margin-top: 2px; color: var(--codex-plus-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .codex-project-move-empty { padding: 18px 12px; color: var(--codex-plus-muted); text-align: center; }
      .codex-project-move-hidden { display: none !important; }
      [data-codex-project-move-injected-list="true"] { display: flex; flex-direction: column; }
      .codex-archive-delete-all {
        border: 1px solid #ef4444;
        border-radius: 7px;
        background: #fee2e2;
        color: #991b1b;
        font: 12px system-ui, sans-serif;
        line-height: 16px;
        padding: 3px 8px;
        cursor: pointer;
      }
      .codex-archive-action-bar {
        position: fixed;
        right: 28px;
        top: 86px;
        z-index: 2147482999;
        box-shadow: 0 8px 24px rgba(0,0,0,.18);
      }
      .codex-delete-toast {
        position: fixed;
        right: 18px;
        bottom: 18px;
        z-index: 2147483000;
        padding: 10px 12px;
        border-radius: 8px;
        background: #111827;
        color: white;
        font: 13px system-ui, sans-serif;
        box-shadow: 0 8px 30px rgba(0,0,0,.25);
        pointer-events: none;
      }
      .codex-delete-toast button { margin-left: 10px; pointer-events: auto; }
      .codex-delete-confirm-overlay {
        position: fixed;
        inset: 0;
        z-index: 2147483200;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--codex-plus-modal-overlay-bg);
      }
      .codex-delete-confirm-overlay.${upstreamWorktreeDialogClass} {
        z-index: 2147483647;
      }
      .codex-delete-confirm-content {
        width: min(420px, calc(100vw - 48px));
        border: 1px solid var(--codex-plus-modal-border);
        border-radius: var(--codex-plus-radius-card);
        background: var(--codex-plus-modal-bg);
        color: var(--codex-plus-modal-fg);
        font-family: var(--codex-plus-font-sans);
        font-size: var(--codex-plus-font-size-md);
        line-height: 1.4;
        box-shadow: var(--codex-plus-modal-shadow);
        padding: 18px;
      }
      .codex-delete-confirm-title { font-size: calc(var(--codex-plus-font-size-md) + 2px); font-weight: var(--font-weight-semibold, 600); }
      .codex-delete-confirm-message { margin-top: 8px; color: var(--codex-plus-muted); line-height: 1.45; }
      .codex-delete-confirm-actions {
        display: flex;
        justify-content: flex-end;
        gap: 10px;
        margin-top: 18px;
      }
      .codex-delete-confirm-actions button {
        border: 1px solid var(--codex-plus-control-border);
        border-radius: var(--codex-plus-radius-control);
        padding: 6px 12px;
        background: var(--codex-plus-control-bg);
        color: var(--codex-plus-control-fg);
        font: var(--codex-plus-font-size-sm) var(--codex-plus-font-sans);
        cursor: pointer;
      }
      .codex-delete-confirm-actions button:hover,
      .codex-delete-confirm-actions button:focus-visible { background: var(--codex-plus-control-hover-bg); outline: none; }
      .codex-delete-confirm-actions [data-codex-delete-confirm="true"] {
        border-color: var(--codex-plus-danger-border);
        background: var(--codex-plus-danger-bg);
        color: var(--codex-plus-danger-fg);
      }
      #${codexPlusMenuId}.${codexPlusMenuFloatingClass} {
        position: fixed;
        top: var(--codex-plus-menu-top, 0);
        right: var(--codex-plus-menu-right, 140px);
        left: auto;
        z-index: 2147483645;
        height: var(--codex-plus-menu-height, 30px);
        color: var(--codex-plus-secondary);
        font: var(--codex-plus-font-size-sm)/1 var(--codex-plus-font-sans);
        text-align: right;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      #${codexPlusMenuId} {
        display: inline-flex;
        align-items: center;
        height: 100%;
        flex: 0 0 auto;
        min-width: max-content;
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      #${codexPlusMenuId}[data-codex-plus-native-menu="true"] {
        margin-inline: 2px;
      }
      .codex-plus-trigger {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        gap: 4px;
        box-sizing: border-box;
        border: 0;
        background: transparent;
        color: inherit;
        font: inherit;
        height: 100%;
        line-height: 1;
        padding: 0 8px;
        cursor: pointer;
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-trigger:hover,
      .codex-plus-trigger:focus-visible { outline: none; }
      .codex-plus-native-trigger {
        aspect-ratio: auto !important;
        border: 1px solid var(--color-token-button-border, var(--color-token-border, var(--color-border, transparent))) !important;
        background: transparent !important;
        flex: 0 0 auto !important;
        gap: 8px !important;
        width: max-content !important;
        min-width: max-content !important;
        min-inline-size: max-content !important;
        max-width: none !important;
        overflow: visible !important;
        padding: 0 12px 0 11px !important;
        white-space: nowrap !important;
      }
      .codex-plus-native-trigger:hover,
      .codex-plus-native-trigger:focus-visible {
        background: var(--color-token-list-hover-background, var(--color-background-button-secondary-hover, transparent)) !important;
      }
      .codex-plus-native-trigger [data-codex-backend-indicator],
      .codex-plus-native-trigger [data-codex-plus-trigger-label] {
        flex: 0 0 auto;
      }
      .codex-plus-native-trigger [data-codex-plus-trigger-label] {
        min-width: max-content;
        overflow: visible;
        text-overflow: clip;
        white-space: nowrap;
      }
      .codex-plus-modal-overlay {
        position: fixed;
        inset: 0;
        z-index: 2147483646;
        display: flex;
        align-items: center;
        justify-content: center;
        background: transparent;
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-content {
        width: min(520px, calc(100vw - 48px));
        max-height: min(680px, calc(100vh - 40px));
        display: flex;
        flex-direction: column;
        overflow: hidden;
        border: 1px solid var(--codex-plus-modal-border);
        border-radius: var(--codex-plus-radius-modal);
        background: var(--codex-plus-modal-bg);
        color: var(--codex-plus-modal-fg);
        font-family: var(--codex-plus-font-sans);
        font-size: var(--codex-plus-font-size-md);
        line-height: 1.4;
        box-shadow: var(--codex-plus-modal-shadow);
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-content[data-codex-plus-active-tab="support"] { width: min(820px, calc(100vw - 48px)); }
      .codex-plus-modal-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        padding: 16px 20px 8px;
        flex: 0 0 auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-header-actions {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        flex: 0 0 auto;
      }
      .codex-plus-modal-title { display: flex; align-items: center; gap: 8px; font-size: var(--codex-plus-font-size-lg); font-weight: var(--font-weight-semibold, 600); }
      .codex-plus-backend-indicator { width: 9px; height: 9px; border-radius: 999px; background: var(--codex-plus-muted); display: inline-block; }
      .codex-plus-backend-indicator[data-status="ok"] { background: var(--codex-plus-success); box-shadow: 0 0 8px color-mix(in srgb, var(--codex-plus-success) 75%, transparent); }
      .codex-plus-backend-indicator[data-status="failed"] { background: var(--codex-plus-error); box-shadow: 0 0 8px color-mix(in srgb, var(--codex-plus-error) 75%, transparent); }
      .codex-plus-backend-indicator[data-status="checking"] { background: var(--codex-plus-warning); }
      .codex-plus-width-input:focus-visible,
      .codex-plus-form-field input:focus-visible {
        border-color: var(--codex-plus-focus-border);
        box-shadow: 0 0 0 1px var(--codex-plus-focus-border);
        outline: none;
      }
      .codex-plus-modal-close {
        width: 28px;
        height: 28px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border: 1px solid transparent;
        border-radius: var(--codex-plus-radius-control);
        background: transparent;
        color: var(--codex-plus-tab-fg);
        font: var(--font-weight-normal, 400) calc(var(--codex-plus-font-size-md) + 6px)/1 var(--codex-plus-font-sans);
        padding: 0;
        cursor: pointer;
        pointer-events: auto;
        -webkit-app-region: no-drag;
      }
      .codex-plus-modal-close:hover,
      .codex-plus-modal-close:focus-visible {
        background: var(--codex-plus-select-hover-bg);
        border-color: var(--codex-plus-input-border);
        outline: none;
      }
      .codex-plus-modal-body {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
        overscroll-behavior: contain;
        scrollbar-gutter: stable;
        padding: 4px 20px 16px;
        scrollbar-width: thin;
        scrollbar-color: var(--codex-plus-scrollbar-thumb) transparent;
      }
      .codex-plus-modal-body::-webkit-scrollbar { width: 10px; }
      .codex-plus-modal-body::-webkit-scrollbar-track { background: transparent; }
      .codex-plus-modal-body::-webkit-scrollbar-thumb {
        border: 2px solid transparent;
        border-radius: 999px;
        background: var(--codex-plus-scrollbar-thumb);
        background-clip: padding-box;
      }
      .codex-plus-modal-body::-webkit-scrollbar-thumb:hover { background: var(--codex-plus-scrollbar-thumb-hover); background-clip: padding-box; }
      .codex-plus-row {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 12px;
        padding: 10px 0;
        border-top: 1px solid var(--codex-plus-row-border);
      }
      .codex-plus-row:first-child { border-top: 0; }
      .codex-plus-row-title { font-weight: 550; line-height: 1.35; }
      .codex-plus-row-description { margin-top: 2px; color: var(--codex-plus-muted); font-size: var(--codex-plus-font-size-sm); line-height: 1.4; }
      .codex-plus-model-compat-warning { margin-top: 6px; color: var(--codex-plus-warning); font-size: var(--codex-plus-font-size-sm); line-height: 1.45; }
      .codex-plus-toggle {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        color: var(--codex-plus-modal-fg);
        font-size: var(--codex-plus-font-size-sm);
        line-height: 1;
        box-sizing: border-box;
        border: 0;
        background: transparent;
        padding: 0;
        cursor: pointer;
      }
      .codex-plus-toggle:focus-visible {
        outline: 2px solid var(--codex-plus-focus-border);
        outline-offset: 2px;
        border-radius: 999px;
      }
      .codex-plus-toggle-track {
        position: relative;
        display: inline-flex;
        flex-shrink: 0;
        align-items: center;
        width: 32px;
        height: 20px;
        box-sizing: border-box;
        border: 0;
        border-radius: 999px;
        background: var(--codex-plus-toggle-bg);
        transition: background-color .2s ease-out;
      }
      .codex-plus-toggle-knob {
        display: block;
        width: 16px;
        height: 16px;
        box-sizing: border-box;
        border: 1px solid var(--codex-plus-toggle-knob-border);
        border-radius: 999px;
        background: var(--codex-plus-toggle-knob-bg);
        box-shadow: var(--shadow-sm, 0 1px 2px -1px rgba(0, 0, 0, .18));
        transform: translateX(2px);
        transition: transform .2s ease-out, background-color .2s ease-out, border-color .2s ease-out;
      }
      .codex-plus-toggle,
      .codex-plus-action-button,
      .codex-plus-issue-button,
      .codex-plus-backend-status {
        flex-shrink: 0;
        align-self: center;
      }
      .codex-plus-toggle[data-enabled="true"] .codex-plus-toggle-track { background: var(--codex-plus-toggle-active-bg); opacity: 1; }
      .codex-plus-toggle[data-enabled="true"] .codex-plus-toggle-knob { transform: translateX(14px); border-color: var(--codex-plus-toggle-active-knob-border); background: var(--codex-plus-toggle-active-knob-bg); opacity: 1; }
      .codex-plus-toggle[data-relay-unneeded="true"] {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: auto;
        min-width: 92px;
        height: 26px;
        border: 1px solid var(--codex-plus-control-border);
        border-radius: var(--codex-plus-radius-control);
        background: var(--codex-plus-toggle-muted-bg);
        color: var(--codex-plus-toggle-muted-fg);
        cursor: default;
        padding: 0 10px;
        white-space: nowrap;
      }
      .codex-plus-toggle[data-relay-unneeded="true"] .codex-plus-toggle-track { display: none; }
      .codex-plus-toggle[data-relay-unneeded="true"]::after { content: attr(data-codex-plus-unneeded-label); font-size: var(--codex-plus-font-size-sm); font-weight: var(--font-weight-normal, 400); line-height: 1; white-space: nowrap; }
      .codex-plus-width-control { display: flex; align-items: center; justify-content: flex-end; gap: 8px; min-width: 176px; align-self: center; }
      .codex-plus-width-input {
        width: 78px;
        height: 26px;
        box-sizing: border-box;
        border: 1px solid var(--codex-plus-input-border);
        border-radius: var(--codex-plus-radius-control);
        background: var(--codex-plus-input-bg);
        color: var(--codex-plus-input-fg);
        font: var(--codex-plus-font-size-sm) var(--codex-plus-font-sans);
        padding: 0 8px;
      }
      .codex-plus-width-input:disabled { opacity: .55; cursor: not-allowed; }
      .codex-plus-service-tier-control { display: grid; gap: 6px; min-width: 316px; justify-items: end; align-self: center; }
      .codex-plus-service-tier-status { color: var(--codex-plus-muted); font-size: var(--codex-plus-font-size-sm); line-height: 1.3; text-align: right; }
      .codex-plus-service-tier-status[data-status="ok"] { color: var(--codex-plus-success); }
      .codex-plus-service-tier-status[data-status="failed"] { color: var(--codex-plus-error); }
      .codex-plus-service-tier-status[data-status="unsupported"] { color: var(--codex-plus-warning); }
      .codex-plus-service-tier-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
      .codex-plus-service-tier-thread-actions { opacity: .88; align-items: center; }
      .codex-plus-service-tier-thread-label { color: var(--codex-plus-muted); font: var(--codex-plus-font-size-sm)/1.2 var(--codex-plus-font-sans); white-space: nowrap; }
      .codex-plus-service-tier-button { border: 1px solid var(--codex-plus-input-border); border-radius: var(--codex-plus-radius-control); background: var(--codex-plus-select-bg); color: var(--codex-plus-control-fg); font: var(--codex-plus-font-size-sm) var(--codex-plus-font-sans); padding: 5px 8px; white-space: nowrap; }
      .codex-plus-service-tier-button:hover,
      .codex-plus-service-tier-button:focus-visible { background: var(--codex-plus-select-hover-bg); outline: none; }
      .codex-plus-service-tier-button:focus-visible { border-color: var(--codex-plus-focus-border); box-shadow: 0 0 0 1px var(--codex-plus-focus-border); }
      .codex-plus-service-tier-button[data-active="true"] { border-color: var(--codex-plus-input-border); background: var(--codex-plus-select-active-bg); color: var(--codex-plus-select-active-fg); font-weight: var(--font-weight-semibold, 600); }
      .codex-plus-service-tier-button:disabled { opacity: .55; cursor: not-allowed; }
      .${codexServiceTierBadgeClass} {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        flex: 0 0 auto;
        height: 24px;
        min-width: 54px;
        box-sizing: border-box;
        border: 1px solid var(--codex-plus-control-border);
        border-radius: 999px;
        background: var(--codex-plus-control-bg);
        color: var(--codex-plus-control-fg);
        font: var(--font-weight-semibold, 600) var(--codex-plus-font-size-sm)/1 var(--codex-plus-font-sans);
        padding: 0 8px;
        white-space: nowrap;
        cursor: pointer;
      }
      .${codexServiceTierBadgeClass}:hover { background: var(--codex-plus-control-hover-bg); }
      .${codexServiceTierBadgeClass}[data-tier="fast"] { border-color: var(--codex-plus-primary-bg); background: var(--codex-plus-primary-bg); color: var(--codex-plus-primary-fg); }
      .${codexServiceTierBadgeClass}[data-tier="loading"] { color: var(--codex-plus-muted); }
      .${codexServiceTierBadgeClass}[data-tier="failed"] { border-color: var(--codex-plus-danger-border); background: var(--codex-plus-danger-bg); color: var(--codex-plus-danger-fg); }
      .${codexServiceTierBadgeClass}[data-tier="unsupported"] { border-color: var(--codex-plus-warning); color: var(--codex-plus-warning); }
      .${codexServiceTierBadgeClass}[data-disabled="true"] { cursor: not-allowed; opacity: .78; }
      .codex-plus-about { color: var(--codex-plus-muted); line-height: 1.5; }
      .codex-plus-tabs { display: flex; gap: 6px; padding: 0 20px 6px; flex: 0 0 auto; }
      .codex-plus-tab-button { border: 1px solid var(--codex-plus-input-border); border-radius: var(--codex-plus-radius-control); background: var(--codex-plus-select-bg); color: var(--codex-plus-tab-fg); font: var(--codex-plus-font-size-sm) var(--codex-plus-font-sans); padding: 5px 10px; }
      .codex-plus-tab-button:hover,
      .codex-plus-tab-button:focus-visible { background: var(--codex-plus-select-hover-bg); outline: none; }
      .codex-plus-tab-button:focus-visible { border-color: var(--codex-plus-focus-border); box-shadow: 0 0 0 1px var(--codex-plus-focus-border); }
      .codex-plus-tab-button[data-active="true"] { background: var(--codex-plus-select-active-bg); color: var(--codex-plus-select-active-fg); border-color: var(--codex-plus-input-border); font-weight: var(--font-weight-semibold, 600); }
      .codex-plus-panel[hidden] { display: none; }
      .codex-plus-action-button,
      .codex-plus-issue-button { box-sizing: border-box; display: inline-flex; align-items: center; justify-content: center; gap: 4px; border: 1px solid var(--codex-plus-action-button-border); border-radius: var(--codex-plus-radius-control); background: var(--codex-plus-action-button-bg); color: var(--codex-plus-action-button-fg); cursor: pointer; font: var(--codex-plus-font-size-sm) var(--codex-plus-font-sans); line-height: 1.2; padding: 6px 8px; white-space: nowrap; user-select: none; }
      .codex-plus-action-button:hover,
      .codex-plus-action-button:focus-visible,
      .codex-plus-issue-button:hover,
      .codex-plus-issue-button:focus-visible { border-color: var(--codex-plus-action-button-border); background: var(--codex-plus-action-button-hover-bg); outline: none; }
      .codex-plus-worktree-actions {
        display: inline-flex;
        align-items: center;
        gap: 8px;
      }
      .codex-plus-form-field {
        display: grid;
        gap: 4px;
        margin-top: 10px;
        color: var(--codex-plus-modal-fg);
        font: var(--codex-plus-font-size-sm) var(--codex-plus-font-sans);
        text-align: left;
      }
      .codex-plus-form-field input {
        width: min(520px, 100%);
        box-sizing: border-box;
        border: 1px solid var(--codex-plus-input-border);
        border-radius: var(--codex-plus-radius-control);
        background: var(--codex-plus-input-bg);
        color: var(--codex-plus-input-fg);
        padding: 8px 10px;
        font: var(--codex-plus-font-size-sm) var(--codex-plus-font-mono);
      }
      .codex-plus-form-message {
        min-height: 18px;
        margin-top: 10px;
        color: var(--codex-plus-muted);
        font: var(--codex-plus-font-size-sm) var(--codex-plus-font-sans);
        text-align: left;
      }
      .codex-plus-form-message[data-status="ok"] { color: var(--codex-plus-success); }
      .codex-plus-form-message[data-status="failed"] { color: var(--codex-plus-error); }
      .codex-plus-form-message[data-status="loading"] { color: var(--codex-plus-warning); }
      .codex-plus-backend-status { display: grid; gap: 4px; min-width: 132px; justify-items: end; }
      .codex-plus-backend-label { color: var(--codex-plus-muted); font-size: var(--codex-plus-font-size-sm); }
      .codex-plus-backend-label[data-status="ok"] { color: var(--codex-plus-success); }
      .codex-plus-backend-label[data-status="failed"] { color: var(--codex-plus-error); }
      .codex-plus-backend-repair { border: 1px solid var(--codex-plus-control-border); border-radius: var(--codex-plus-radius-control); background: var(--codex-plus-control-bg); color: var(--codex-plus-control-fg); font: var(--codex-plus-font-size-sm) var(--codex-plus-font-sans); padding: 6px 8px; }
      .codex-plus-backend-repair:hover,
      .codex-plus-backend-repair:focus-visible { background: var(--codex-plus-control-hover-bg); outline: none; }
      .codex-plus-backend-repair[hidden] { display: none; }
      .codex-plus-user-script-warning { margin-top: 4px; color: var(--codex-plus-warning); font-size: var(--codex-plus-font-size-sm); }
      .codex-plus-user-script-dirs { margin-top: 6px; color: var(--codex-plus-muted); font-size: var(--codex-plus-font-size-xs); line-height: 1.4; word-break: break-all; }
      .codex-plus-user-script-list { margin-top: 8px; display: grid; gap: 6px; }
      .codex-plus-user-script-item { display: flex; align-items: center; justify-content: space-between; gap: 8px; border: 1px solid var(--codex-plus-row-border); border-radius: var(--codex-plus-radius-control); padding: 6px 8px; }
      .codex-plus-user-script-name { font-size: var(--codex-plus-font-size-sm); }
      .codex-plus-user-script-meta { margin-top: 2px; color: var(--codex-plus-muted); font-size: var(--codex-plus-font-size-xs); }
      .codex-plus-user-script-error { margin-top: 2px; color: var(--codex-plus-error); font-size: var(--codex-plus-font-size-xs); word-break: break-all; }
      .codex-plus-user-script-actions { display: grid; justify-items: end; gap: 8px; min-width: 120px; }
      .codex-plus-user-script-reload { border: 1px solid var(--codex-plus-control-border); border-radius: var(--codex-plus-radius-control); background: var(--codex-plus-control-bg); color: var(--codex-plus-control-fg); font: var(--codex-plus-font-size-sm) var(--codex-plus-font-sans); padding: 6px 8px; }
      .codex-plus-user-script-reload:hover,
      .codex-plus-user-script-reload:focus-visible { background: var(--codex-plus-control-hover-bg); outline: none; }
      .codex-plus-sponsor-text { color: var(--codex-plus-secondary); font-size: var(--codex-plus-font-size-md); line-height: 1.55; margin: 4px 0 12px; }
      .codex-plus-ad-section { display: grid; gap: 10px; margin-top: 12px; }
      .codex-plus-ad-section:first-of-type { margin-top: 0; }
      .codex-plus-ad-section-title { color: var(--codex-plus-modal-fg); font-size: calc(var(--codex-plus-font-size-md) + 1px); margin: 0; }
      .codex-plus-ad-list { display: grid; gap: 14px; }
      .codex-plus-ad-card { border: 1px solid var(--codex-plus-ad-card-border); border-radius: var(--codex-plus-radius-card); background: var(--codex-plus-ad-card-bg); box-shadow: var(--codex-plus-ad-card-shadow); }
      .codex-plus-ad-content { padding: 14px; }
      .codex-plus-ad-title { margin: 0; color: var(--codex-plus-modal-fg); font-size: calc(var(--codex-plus-font-size-md) + 3px); line-height: 1.35; }
      .codex-plus-ad-description { margin: 6px 0 10px; color: var(--codex-plus-secondary); font-size: var(--codex-plus-font-size-md); line-height: 1.55; }
      .codex-plus-ad-highlights { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px; }
      .codex-plus-ad-highlights span { border: 1px solid var(--codex-plus-ad-chip-border); border-radius: 999px; background: var(--codex-plus-ad-chip-bg); color: var(--codex-plus-ad-chip-fg); font-size: var(--codex-plus-font-size-sm); padding: 4px 8px; }
      .codex-plus-ad-link { display: inline-flex; align-items: center; justify-content: center; border-radius: var(--codex-plus-radius-control); background: var(--codex-plus-primary-bg); color: var(--codex-plus-primary-fg); font-size: var(--codex-plus-font-size-md); font-weight: var(--font-weight-semibold, 600); text-decoration: none; padding: 8px 12px; }
      .codex-plus-ad-empty { border: 1px dashed var(--codex-plus-ad-empty-border); border-radius: var(--codex-plus-radius-card); color: var(--codex-plus-ad-empty-fg); font-size: var(--codex-plus-font-size-md); padding: 12px; text-align: center; }
      .codex-plus-sponsor-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
      .codex-plus-sponsor-card { border: 1px solid var(--codex-plus-row-border); border-radius: var(--codex-plus-radius-card); padding: 10px; background: var(--codex-plus-card-bg); text-align: center; }
      .codex-plus-sponsor-card-title { color: var(--codex-plus-modal-fg); font-size: var(--codex-plus-font-size-md); margin-bottom: 8px; }
      .codex-plus-sponsor-qr { display: block; width: 100%; max-width: 340px; border-radius: 8px; margin: 0 auto; background: white; }
    `;
    document.documentElement.appendChild(style);
  }

  function defaultCodexPlusSettings() {
    return { pluginMarketplaceUnlock: true, forcePluginInstall: true, pluginAutoExpand: true, modelWhitelistUnlock: true, sessionDelete: true, markdownExport: true, pasteFix: false, projectMove: true, threadIdBadge: false, conversationView: false, conversationViewMaxWidth: conversationViewDefaultWidth, threadScrollRestore: true, zedRemoteOpen: true, upstreamWorktreeCreate: true, nativeMenuPlacement: true, serviceTierControls: false };
  }

  const codexPlusBackendSettingMap = {
    pluginMarketplaceUnlock: "codexAppPluginMarketplaceUnlock",
    forcePluginInstall: "codexAppForcePluginInstall",
    pluginAutoExpand: "codexAppPluginAutoExpand",
    modelWhitelistUnlock: "codexAppModelWhitelistUnlock",
    sessionDelete: "codexAppSessionDelete",
    markdownExport: "codexAppMarkdownExport",
    projectMove: "codexAppProjectMove",
    threadIdBadge: "codexAppThreadIdBadge",
    conversationView: "codexAppConversationView",
    threadScrollRestore: "codexAppThreadScrollRestore",
    zedRemoteOpen: "codexAppZedRemoteOpen",
    upstreamWorktreeCreate: "codexAppUpstreamWorktreeCreate",
    nativeMenuPlacement: "codexAppNativeMenuPlacement",
    serviceTierControls: "codexAppServiceTierControls",
    pasteFix: "codexAppPasteFix",
  };

  function backendCodexPlusSettings() {
    const settings = {};
    Object.entries(codexPlusBackendSettingMap).forEach(([localKey, backendKey]) => {
      if (typeof codexPlusBackendSettings[backendKey] === "boolean") {
        settings[localKey] = codexPlusBackendSettings[backendKey];
      }
    });
    return settings;
  }

  function codexPlusSettings() {
    const relayPatchDisabled = codexPlusBackendSettings.launchMode === "relay";
    if (codexPlusBackendSettings.enhancementsEnabled === false) {
      return {
        pluginMarketplaceUnlock: false,
        forcePluginInstall: false,
        pluginAutoExpand: false,
        modelWhitelistUnlock: false,
        sessionDelete: false,
        markdownExport: false,
        pasteFix: false,
        projectMove: false,
        threadIdBadge: false,
        conversationView: false,
        conversationViewMaxWidth: conversationViewDefaultWidth,
        threadScrollRestore: false,
        zedRemoteOpen: false,
        upstreamWorktreeCreate: false,
        nativeMenuPlacement: false,
        serviceTierControls: false,
      };
    }
    try {
      const settings = { ...defaultCodexPlusSettings(), ...JSON.parse(localStorage.getItem(codexPlusSettingsKey) || "{}"), ...backendCodexPlusSettings() };
      if (relayPatchDisabled) {
        settings.pluginMarketplaceUnlock = false;
        settings.forcePluginInstall = false;
      }
      return settings;
    } catch {
      const settings = { ...defaultCodexPlusSettings(), ...backendCodexPlusSettings() };
      if (relayPatchDisabled) {
        settings.pluginMarketplaceUnlock = false;
        settings.forcePluginInstall = false;
      }
      return settings;
    }
  }

  function setCodexPlusSetting(key, value) {
    const backendKey = codexPlusBackendSettingMap[key];
    if (backendKey) {
      setBackendSetting(backendKey, value);
      return;
    }
    let stored = {};
    try {
      stored = JSON.parse(localStorage.getItem(codexPlusSettingsKey) || "{}");
    } catch {
      stored = {};
    }
    const next = { ...stored, [key]: value };
    localStorage.setItem(codexPlusSettingsKey, JSON.stringify(next));
    if (key === "threadScrollRestore" && !value) {
      clearTimeout(window.__codexThreadScrollSaveTimer);
      window.__codexThreadScrollSaveTimer = null;
      window.__codexThreadScrollRestoreRevision = (window.__codexThreadScrollRestoreRevision || 0) + 1;
      window.__codexThreadScrollSyncRevision = (window.__codexThreadScrollSyncRevision || 0) + 1;
      (window.__codexThreadScrollRestoreTimers || []).forEach((timer) => clearTimeout(timer));
      window.__codexThreadScrollRestoreTimers = [];
      (window.__codexThreadScrollSyncTimers || []).forEach((timer) => clearTimeout(timer));
      window.__codexThreadScrollSyncTimers = [];
      window.__codexThreadScrollRuntime = null;
    }
    if (key === "serviceTierControls") {
      if (value) {
        void loadCodexServiceTierState();
      } else {
        removeCodexServiceTierBadges();
        refreshCodexServiceTierControls();
      }
    }
    if (key === "pluginAutoExpand" && !value) {
      clearTimeout(window.__codexPluginAutoExpandTimer);
      window.__codexPluginAutoExpandTimer = null;
      window.__codexPluginAutoExpandRunning = false;
      window.__codexPluginAutoExpandLastSignature = "";
    }
    renderCodexPlusMenu();
    scan();
  }

  function normalizeConversationViewWidth(value) {
    if (value === null || value === undefined || String(value).trim() === "") return null;
    const number = Number(value);
    if (!Number.isFinite(number)) return null;
    return Math.max(conversationViewMinWidth, Math.min(conversationViewMaxAllowedWidth, Math.round(number)));
  }

  function conversationViewWidth() {
    const settingsWidth = normalizeConversationViewWidth(codexPlusSettings().conversationViewMaxWidth);
    if (settingsWidth) return settingsWidth;
    const legacyWidth = normalizeConversationViewWidth(localStorage.getItem(conversationViewLegacyWidthKey));
    return legacyWidth || conversationViewDefaultWidth;
  }

  function refreshConversationViewControls() {
    const enabled = !!codexPlusSettings().conversationView;
    const width = conversationViewWidth();
    document.querySelectorAll("[data-codex-plus-conversation-view-width]").forEach((input) => {
      input.value = String(width);
      input.disabled = !enabled;
    });
  }

  function setConversationViewWidth(value) {
    const width = normalizeConversationViewWidth(value);
    if (!width) return;
    setCodexPlusSetting("conversationViewMaxWidth", width);
  }

  function codexPlusToggleMarkup(state = "unchecked") {
    return `<span class="codex-plus-toggle-track" data-state="${state}"><span class="codex-plus-toggle-knob" data-state="${state}"></span></span>`;
  }

  function codexPlusToggleAriaLabel(button) {
    const rowTitle = button.closest?.(".codex-plus-row")?.querySelector?.(".codex-plus-row-title")?.textContent;
    const scriptTitle = button.closest?.(".codex-plus-user-script-item")?.querySelector?.(".codex-plus-user-script-title")?.textContent;
    return String(rowTitle || scriptTitle || button.getAttribute("data-codex-plus-setting") || button.getAttribute("data-codex-backend-setting") || "Codex++").trim();
  }

  function setCodexPlusToggleState(button, enabled) {
    const isEnabled = !!enabled;
    const state = isEnabled ? "checked" : "unchecked";
    button.dataset.enabled = String(isEnabled);
    button.dataset.state = state;
    button.setAttribute("role", "switch");
    button.setAttribute("aria-checked", String(isEnabled));
    if (!button.getAttribute("aria-label")) {
      button.setAttribute("aria-label", codexPlusToggleAriaLabel(button));
    }
    if (!button.querySelector(".codex-plus-toggle-track")) {
      button.innerHTML = codexPlusToggleMarkup(state);
    }
    button.querySelector(".codex-plus-toggle-track")?.setAttribute("data-state", state);
    button.querySelector(".codex-plus-toggle-knob")?.setAttribute("data-state", state);
  }

  function renderCodexPlusMenu() {
    document.querySelectorAll(".codex-plus-toggle[data-codex-plus-setting]").forEach((button) => {
      const key = button.getAttribute("data-codex-plus-setting");
      setCodexPlusToggleState(button, !!codexPlusSettings()[key]);
    });
    refreshConversationViewControls();
    refreshCodexServiceTierControls();
  }

  let codexPlusBackendSettings = { providerSyncEnabled: false, enhancementsEnabled: true, launchMode: "patch", codexAppVersion: "" };
  const codexPluginLegacyEntryUnlockBeforeVersion = "26.601.2237";
  const codexPluginBridgeRequestUnlockFromVersion = "26.616.0";

  function parseCodexVersionParts(version) {
    const raw = String(version || "").trim();
    if (!raw) return null;
    const match = raw.match(/\d+(?:\.\d+)*/);
    if (!match) return null;
    const parts = match[0].split(".").map((part) => Number(part));
    if (!parts.length || parts.some((part) => !Number.isInteger(part) || part < 0)) return null;
    return parts;
  }

  function compareCodexVersions(left, right) {
    const leftParts = parseCodexVersionParts(left);
    const rightParts = parseCodexVersionParts(right);
    if (!leftParts || !rightParts) return null;
    const length = Math.max(leftParts.length, rightParts.length);
    for (let index = 0; index < length; index += 1) {
      const leftPart = leftParts[index] || 0;
      const rightPart = rightParts[index] || 0;
      if (leftPart !== rightPart) return leftPart < rightPart ? -1 : 1;
    }
    return 0;
  }

  function codexPluginUnlockStrategy() {
    const version = String(codexPlusBackendSettings.codexAppVersion || "").trim();
    const comparison = compareCodexVersions(version, codexPluginLegacyEntryUnlockBeforeVersion);
    if (comparison == null) return "unknown";
    return comparison < 0 ? "legacy" : "modern";
  }

  function logCodexPluginUnlockStrategy(strategy) {
    const codexAppVersion = String(codexPlusBackendSettings.codexAppVersion || "").trim();
    const signature = `${strategy}:${codexAppVersion || "unknown"}`;
    if (window.__codexPluginUnlockStrategyLogged === signature) return;
    window.__codexPluginUnlockStrategyLogged = signature;
    sendCodexPlusDiagnostic("plugin_unlock_strategy_selected", {
      strategy,
      codexAppVersion,
      cutoff: codexPluginLegacyEntryUnlockBeforeVersion,
    });
  }

  function codexPluginMarketplaceRequestPatchStrategy() {
    const pluginStrategy = codexPluginUnlockStrategy();
    if (pluginStrategy === "legacy") return "none";
    const version = String(codexPlusBackendSettings.codexAppVersion || "").trim();
    const comparison = compareCodexVersions(version, codexPluginBridgeRequestUnlockFromVersion);
    if (comparison == null) return "unknown";
    return comparison >= 0 ? "bridge" : "client";
  }

  let codexPlusBackendSettingsLoaded = false;
  let codexServiceTierState = {
    status: "loading",
    serviceTier: null,
    message: t("serviceTierReading"),
    fastTierValue: "priority",
    controlMode: "inherit",
    defaultMode: "inherit",
    activeThreadId: "",
    threadMode: "inherit",
    effectiveServiceTier: null,
    effectiveMode: "standard",
    fastModelName: "",
    fastSupported: false,
  };
  const codexDefaultServiceTierSetting = { key: "default-service-tier", default: null };
  const codexServiceTierFallbackFastValue = "priority";
  const codexServiceTierModulePromises = new Map();
  const codexServiceTierSupportedFastModels = new Set(["gpt-5.4", "gpt-5.5"]);
  const codexThreadServiceTierModes = new Set(["inherit", "standard", "fast"]);
  const codexServiceTierControlModes = new Set(["inherit", "global-standard", "global-fast", "custom"]);

  function codexAppAssetUrl(namePart) {
    const urls = [
      ...Array.from(document.scripts || []).map((script) => script.src),
      ...Array.from(document.querySelectorAll("link[href]") || []).map((link) => link.href),
      ...performance.getEntriesByType("resource").map((entry) => entry.name),
    ].filter(Boolean);
    return urls.find((url) => url.includes("/assets/") && url.includes(namePart) && url.split("?")[0].endsWith(".js")) || "";
  }

  async function loadCodexAppModule(namePart) {
    if (!codexServiceTierModulePromises.has(namePart)) {
      const promise = Promise.resolve().then(async () => {
        const url = codexAppAssetUrl(namePart);
        if (!url) throw new Error(`未找到 Codex App asset: ${namePart}`);
        return await import(url);
      }).catch((error) => {
        codexServiceTierModulePromises.delete(namePart);
        throw error;
      });
      codexServiceTierModulePromises.set(namePart, promise);
    }
    return await codexServiceTierModulePromises.get(namePart);
  }

  async function codexSettingStorageModule() {
    const module = await loadCodexAppModule("setting-storage-");
    if (typeof module.n !== "function" || typeof module.s !== "function") {
      throw new Error("Codex setting-storage 接口不可用");
    }
    return module;
  }

  async function getCodexServiceTierSetting() {
    try {
      const settingStorage = await codexSettingStorageModule();
      return await settingStorage.n(codexDefaultServiceTierSetting);
    } catch (error) {
      if (typeof codexStateCall === "function") {
        const result = await codexStateCall("get-setting", { params: { key: codexDefaultServiceTierSetting.key } });
        return result && Object.prototype.hasOwnProperty.call(result, "value") ? result.value : codexDefaultServiceTierSetting.default;
      }
      throw error;
    }
  }

  function isFastServiceTierValue(value) {
    const normalized = String(value || "").trim().toLowerCase();
    return normalized === "fast" || normalized === "priority";
  }

  function codexFastServiceTierValue() {
    return codexServiceTierState.fastTierValue || codexServiceTierFallbackFastValue;
  }

  function codexServiceTierFastModelListLabel() {
    return Array.from(codexServiceTierSupportedFastModels).join(" / ");
  }

  function normalizeCodexServiceTierModelName(model) {
    return String(model || "").trim().toLowerCase();
  }

  function codexServiceTierModelFromValue(value, visited = new WeakSet(), depth = 0) {
    if (typeof value === "string") return value.trim();
    if (!value || typeof value !== "object" || visited.has(value) || depth > 3) return "";
    visited.add(value);
    for (const key of ["model", "modelId", "model_id", "selectedModel", "selected_model", "defaultModel", "default_model"]) {
      const model = codexServiceTierModelFromValue(value[key], visited, depth + 1);
      if (model) return model;
    }
    for (const key of ["params", "request", "payload", "body", "config", "options"]) {
      const model = codexServiceTierModelFromValue(value[key], visited, depth + 1);
      if (model) return model;
    }
    return "";
  }

  function codexServiceTierCurrentModelName() {
    return codexServiceTierModelFromValue(codexModelCatalog.model) || codexServiceTierModelFromValue(codexModelCatalog.default_model);
  }

  function codexServiceTierModelForRequest(params, modelHint = "") {
    return codexServiceTierModelFromValue(params) || codexServiceTierModelFromValue(modelHint) || codexServiceTierCurrentModelName();
  }

  function codexServiceTierFastSupportedForModel(modelName) {
    return codexServiceTierSupportedFastModels.has(normalizeCodexServiceTierModelName(modelName));
  }

  function codexServiceTierFastUnsupportedMessage(modelName = codexServiceTierCurrentModelName()) {
    const modelText = modelName
      ? t("serviceTierFastUnsupportedModel", { model: modelName })
      : t("serviceTierFastUnsupportedUnknown");
    return t("serviceTierFastUnsupported", { models: codexServiceTierFastModelListLabel(), modelText });
  }

  function codexServiceTierMaybeLoadModelCatalog(force = false) {
    if (codexModelCatalogPromise) return;
    if (!force && codexModelCatalog.status === "failed") return;
    if (!force && codexModelCatalogLoadedAt && Date.now() - codexModelCatalogLoadedAt < 10000) return;
    loadCodexModelCatalog(force).then(() => {
      refreshCodexServiceTierControls();
    }).catch(() => {
      refreshCodexServiceTierControls();
    });
  }

  function codexServiceTierFastAvailability(modelName = codexServiceTierCurrentModelName()) {
    const normalizedModel = normalizeCodexServiceTierModelName(modelName);
    return {
      modelName: modelName || "",
      supported: !!normalizedModel && codexServiceTierSupportedFastModels.has(normalizedModel),
    };
  }

  function codexServiceTierValueForMode(mode) {
    if (mode === "fast") return codexFastServiceTierValue();
    if (mode === "standard") return null;
    return codexServiceTierState.serviceTier || null;
  }

  function codexServiceTierDefaultModeForControlMode(controlMode, fallback = "inherit") {
    if (controlMode === "global-fast") return "fast";
    if (controlMode === "global-standard") return "standard";
    if (controlMode === "inherit") return "inherit";
    return normalizeCodexThreadServiceTierMode(fallback);
  }

  function codexServiceTierControlModeForDefaultMode(defaultMode) {
    if (defaultMode === "fast") return "global-fast";
    if (defaultMode === "standard") return "global-standard";
    return "inherit";
  }

  function codexServiceTierEffectiveThreadMode(threadMode = "inherit", defaultMode = "inherit") {
    const normalizedThreadMode = normalizeCodexThreadServiceTierMode(threadMode);
    if (normalizedThreadMode !== "inherit") return normalizedThreadMode;
    return normalizeCodexThreadServiceTierMode(defaultMode);
  }

  function codexServiceTierValueForControlMode(controlMode, threadMode = "inherit", defaultMode = "inherit") {
    if (controlMode === "global-fast") return codexFastServiceTierValue();
    if (controlMode === "global-standard") return null;
    if (controlMode === "custom") return codexServiceTierValueForMode(codexServiceTierEffectiveThreadMode(threadMode, defaultMode));
    return codexServiceTierState.serviceTier || null;
  }

  function codexServiceTierEffectiveMode(value) {
    return isFastServiceTierValue(value) ? "fast" : "standard";
  }

  function normalizeCodexThreadServiceTierMode(mode) {
    const normalized = String(mode || "").trim().toLowerCase();
    return codexThreadServiceTierModes.has(normalized) ? normalized : "inherit";
  }

  function normalizeCodexServiceTierControlMode(mode) {
    const normalized = String(mode || "").trim().toLowerCase();
    return codexServiceTierControlModes.has(normalized) ? normalized : "inherit";
  }

  function serviceTierGlobalStatusMessage(serviceTier) {
    if (isFastServiceTierValue(serviceTier)) return t("serviceTierGlobalFastOn");
    if (!serviceTier) return t("serviceTierGlobalDefault");
    return t("serviceTierCurrentValue", { value: serviceTier });
  }

  function serviceTierStatusMessage(
    controlMode = codexServiceTierState.controlMode || "inherit",
    threadMode = codexServiceTierState.threadMode || "inherit",
    effectiveMode = codexServiceTierState.effectiveMode || "standard",
    defaultMode = codexServiceTierState.defaultMode || "inherit"
  ) {
    if (codexServiceTierState.status === "loading") return t("serviceTierReading");
    if (codexServiceTierState.status === "failed") return t("serviceTierReadFailed");
    if (controlMode === "inherit") return t("serviceTierInheritStatus", { mode: effectiveMode });
    if (controlMode === "global-standard") return t("serviceTierGlobalStandard");
    if (controlMode === "global-fast") return t("serviceTierGlobalFast");
    if (threadMode === "inherit") return t("serviceTierCustomDefault", { mode: defaultMode });
    return t("serviceTierCustomThread", { mode: threadMode });
  }

  function readThreadServiceTierState() {
    try {
      const parsed = JSON.parse(localStorage.getItem(codexThreadServiceTierKey) || "{}");
      const rawEntries = parsed?.version === codexThreadServiceTierVersion && parsed?.entries && typeof parsed.entries === "object"
        ? parsed.entries
        : {};
      const entries = Object.create(null);
      Object.entries(rawEntries).forEach(([key, value]) => {
        const safeKey = typeof validThreadScrollSessionKey === "function" ? validThreadScrollSessionKey(key) : String(key || "");
        const mode = normalizeCodexThreadServiceTierMode(value?.mode);
        if (safeKey && mode !== "inherit") entries[safeKey] = { mode, at: finiteNonNegativeNumber(value?.at) || Date.now() };
      });
      const draft = normalizeThreadServiceTierDraft(parsed?.draft);
      const hasCustomState = !!draft || Object.keys(entries).length > 0;
      const mode = parsed?.mode ? normalizeCodexServiceTierControlMode(parsed.mode) : (hasCustomState ? "custom" : "inherit");
      return {
        mode,
        defaultMode: normalizeCodexThreadServiceTierMode(parsed?.defaultMode || codexServiceTierDefaultModeForControlMode(mode)),
        entries,
        draft,
      };
    } catch (_) {
      return { mode: "inherit", defaultMode: "inherit", entries: Object.create(null), draft: null };
    }
  }

  function writeThreadServiceTierState(state) {
    const mode = normalizeCodexServiceTierControlMode(state?.mode);
    const defaultMode = normalizeCodexThreadServiceTierMode(state?.defaultMode || codexServiceTierDefaultModeForControlMode(mode));
    const rawEntries = state?.entries && typeof state.entries === "object" ? state.entries : {};
    const entries = Object.create(null);
    Object.entries(rawEntries)
      .map(([key, value]) => {
        const safeKey = validThreadScrollSessionKey(key);
        const mode = normalizeCodexThreadServiceTierMode(value?.mode);
        return safeKey && mode !== "inherit" ? [safeKey, { mode, at: finiteNonNegativeNumber(value?.at) || Date.now() }] : null;
      })
      .filter(Boolean)
      .sort((left, right) => right[1].at - left[1].at)
      .slice(0, codexThreadServiceTierMaxEntries)
      .forEach(([key, value]) => {
        entries[key] = value;
      });
    const draft = normalizeThreadServiceTierDraft(state?.draft);
    try {
      localStorage.setItem(codexThreadServiceTierKey, JSON.stringify({
        version: codexThreadServiceTierVersion,
        mode,
        defaultMode,
        entries,
        ...(draft ? { draft } : {}),
      }));
    } catch (_) {}
  }

  function normalizeThreadServiceTierDraft(value) {
    if (!value || typeof value !== "object") return null;
    const mode = normalizeCodexThreadServiceTierMode(value.mode);
    if (mode === "inherit") return null;
    const at = finiteNonNegativeNumber(value.at) || Date.now();
    return { mode, at };
  }

  function codexThreadServiceTierOverride(threadId) {
    const key = validThreadScrollSessionKey(threadId);
    if (!key) return null;
    const entry = readThreadServiceTierState().entries[key];
    const mode = normalizeCodexThreadServiceTierMode(entry?.mode);
    return mode === "inherit" ? null : { mode, at: finiteNonNegativeNumber(entry?.at) || 0 };
  }

  function codexThreadServiceTierDraft() {
    const draft = readThreadServiceTierState().draft;
    if (!draft) return null;
    if (Date.now() - draft.at > codexThreadServiceTierDraftBindWindowMs) return null;
    return draft;
  }

  function setCodexThreadServiceTierOverride(threadId, mode) {
    const normalizedMode = normalizeCodexThreadServiceTierMode(mode);
    const state = readThreadServiceTierState();
    state.mode = "custom";
    const key = validThreadScrollSessionKey(threadId);
    if (key) {
      if (normalizedMode === "inherit") {
        delete state.entries[key];
      } else {
        state.entries[key] = { mode: normalizedMode, at: Date.now() };
      }
    } else if (normalizedMode === "inherit") {
      state.draft = null;
    } else {
      state.draft = { mode: normalizedMode, at: Date.now() };
    }
    writeThreadServiceTierState(state);
  }

  function bindDraftServiceTierToThread(threadId) {
    const key = validThreadScrollSessionKey(threadId);
    const draft = codexThreadServiceTierDraft();
    if (!key || !draft) return false;
    const state = readThreadServiceTierState();
    if (normalizeCodexServiceTierControlMode(state.mode) !== "custom") {
      state.draft = null;
      writeThreadServiceTierState(state);
      return false;
    }
    if (!state.entries[key]) state.entries[key] = { mode: draft.mode, at: Date.now() };
    state.draft = null;
    writeThreadServiceTierState(state);
    return true;
  }

  function setCodexServiceTierControlMode(mode) {
    if (codexPlusBackendStatus.status !== "ok") {
      showToast(t("serviceTierBackendDisconnectedToast"), null);
      refreshCodexServiceTierControls();
      return;
    }
    const normalizedMode = normalizeCodexServiceTierControlMode(mode);
    if (normalizedMode === "global-fast") {
      const fastAvailability = codexServiceTierFastAvailability();
      if (!fastAvailability.supported) {
        codexServiceTierMaybeLoadModelCatalog(true);
        showToast(codexServiceTierFastUnsupportedMessage(fastAvailability.modelName), null);
        refreshCodexServiceTierControls();
        return;
      }
    }
    const state = readThreadServiceTierState();
    state.mode = normalizedMode;
    if (normalizedMode !== "custom") {
      state.defaultMode = codexServiceTierDefaultModeForControlMode(normalizedMode);
      state.entries = Object.create(null);
      state.draft = null;
    } else {
      state.defaultMode = normalizeCodexThreadServiceTierMode(state.defaultMode);
    }
    writeThreadServiceTierState(state);
    refreshCodexServiceTierControls();
    const labels = {
      inherit: t("serviceTierModeInherit"),
      "global-standard": t("serviceTierModeGlobalStandard"),
      "global-fast": t("serviceTierModeGlobalFast"),
      custom: t("serviceTierModeCustom"),
    };
    showToast(t("serviceTierToast", { mode: labels[normalizedMode] || normalizedMode }), null);
  }

  function syncCodexServiceTierEffectiveState() {
    if (!codexPlusSettings().serviceTierControls) {
      codexServiceTierState = {
        ...codexServiceTierState,
        activeThreadId: "",
        threadMode: "inherit",
        effectiveServiceTier: codexServiceTierState.serviceTier || null,
        effectiveMode: codexServiceTierEffectiveMode(codexServiceTierState.serviceTier),
        message: t("serviceTierDisabled"),
      };
      return;
    }
    const activeThreadId = validThreadScrollSessionKey(currentSessionRef().session_id);
    if (activeThreadId) bindDraftServiceTierToThread(activeThreadId);
    const storedState = readThreadServiceTierState();
    const controlMode = normalizeCodexServiceTierControlMode(storedState.mode);
    const defaultMode = normalizeCodexThreadServiceTierMode(storedState.defaultMode);
    const override = activeThreadId ? codexThreadServiceTierOverride(activeThreadId) : codexThreadServiceTierDraft();
    const threadMode = normalizeCodexThreadServiceTierMode(override?.mode);
    const effectiveServiceTier = codexServiceTierValueForControlMode(controlMode, threadMode, defaultMode);
    const effectiveMode = codexServiceTierEffectiveMode(effectiveServiceTier);
    const fastAvailability = codexServiceTierFastAvailability();
    const message = effectiveMode === "fast" && !fastAvailability.supported
      ? codexServiceTierFastUnsupportedMessage(fastAvailability.modelName)
      : serviceTierStatusMessage(controlMode, threadMode, effectiveMode, defaultMode);
    codexServiceTierState = {
      ...codexServiceTierState,
      controlMode,
      defaultMode,
      activeThreadId,
      threadMode,
      effectiveServiceTier,
      effectiveMode,
      fastModelName: fastAvailability.modelName,
      fastSupported: fastAvailability.supported,
      message,
    };
  }

  function codexServiceTierBadgeState() {
    if (codexPlusBackendStatus.status === "checking") return { tier: "loading", label: "...", disabled: true, title: t("serviceTierLoadingTitle") };
    if (codexPlusBackendStatus.status && codexPlusBackendStatus.status !== "ok") return { tier: "failed", label: t("backendDisconnected"), disabled: true, title: t("serviceTierFailedTitle") };
    if (codexServiceTierState.status === "loading") return { tier: "loading", label: "...", title: t("serviceTierReadingTitle") };
    if (codexServiceTierState.status === "failed") return { tier: "failed", label: "?", title: t("serviceTierReadFailedTitle") };
    const fastAvailability = codexServiceTierFastAvailability();
    const effectiveMode = codexServiceTierState.effectiveMode || "standard";
    const scope = codexServiceTierState.controlMode === "custom" && codexServiceTierState.threadMode !== "inherit"
      ? t("serviceTierCustomThread", { mode: codexServiceTierState.threadMode })
      : serviceTierStatusMessage(codexServiceTierState.controlMode, codexServiceTierState.threadMode, effectiveMode, codexServiceTierState.defaultMode);
    const title = [
      t("serviceTierBadgeTitlePrefix", { scope }),
      t("serviceTierStandardHelp"),
      t("serviceTierFastHelp", { models: codexServiceTierFastModelListLabel() }),
    ].join("\n");
    if (effectiveMode === "fast" && !fastAvailability.supported) {
      return {
        tier: "unsupported",
        label: t("serviceTierUnsupportedBadge"),
        title: `${title}\n${t("serviceTierUnsupportedSuffix", { message: codexServiceTierFastUnsupportedMessage(fastAvailability.modelName) })}`,
      };
    }
    if (effectiveMode === "fast") return { tier: "fast", label: "fast", title };
    return { tier: "standard", label: "standard", title };
  }

  function refreshCodexServiceTierBadges() {
    const state = codexServiceTierBadgeState();
    document.querySelectorAll(`[data-codex-service-tier-badge="true"]`).forEach((node) => {
      node.dataset.tier = state.tier;
      node.dataset.disabled = String(!!state.disabled);
      node.textContent = state.label;
      node.title = state.title;
      node.setAttribute("aria-label", state.title);
    });
  }

  function refreshCodexServiceTierControls() {
    syncCodexServiceTierEffectiveState();
    const featureEnabled = !!codexPlusSettings().serviceTierControls;
    const backendConnected = codexPlusBackendStatus.status === "ok";
    const backendChecking = codexPlusBackendStatus.status === "checking";
    if (featureEnabled && backendConnected) codexServiceTierMaybeLoadModelCatalog();
    const fastAvailability = codexServiceTierFastAvailability();
    const fastDisabled = !featureEnabled || !backendConnected || codexServiceTierState.status === "loading" || !fastAvailability.supported;
    const fastTitle = fastAvailability.supported
      ? t("serviceTierFastTitle")
      : codexServiceTierFastUnsupportedMessage(fastAvailability.modelName);
    const fastUnsupportedActive = codexServiceTierState.effectiveMode === "fast" && !fastAvailability.supported;
    document.querySelectorAll("[data-codex-service-tier-controls]").forEach((node) => {
      node.hidden = !featureEnabled;
    });
    document.querySelectorAll("[data-codex-service-tier-status]").forEach((node) => {
      node.dataset.status = fastUnsupportedActive ? "unsupported" : (featureEnabled && backendConnected ? (codexServiceTierState.status || "loading") : (backendChecking ? "loading" : "failed"));
      node.textContent = featureEnabled
        ? (backendConnected ? (codexServiceTierState.message || t("serviceTierNotRead")) : (backendChecking ? t("backendChecking") : t("backendDisconnected")))
        : t("serviceTierDisabled");
    });
    document.querySelectorAll("[data-codex-service-tier-inherit]").forEach((button) => {
      button.disabled = !featureEnabled || !backendConnected || codexServiceTierState.status === "loading";
      button.dataset.active = String(codexServiceTierState.controlMode === "inherit");
    });
    document.querySelectorAll("[data-codex-service-tier-standard]").forEach((button) => {
      button.disabled = !featureEnabled || !backendConnected || codexServiceTierState.status === "loading";
      button.dataset.active = String(codexServiceTierState.controlMode === "global-standard");
    });
    document.querySelectorAll("[data-codex-service-tier-fast]").forEach((button) => {
      button.disabled = fastDisabled;
      button.dataset.active = String(codexServiceTierState.controlMode === "global-fast");
      button.title = fastTitle;
    });
    document.querySelectorAll("[data-codex-service-tier-custom]").forEach((button) => {
      button.disabled = !featureEnabled || !backendConnected || codexServiceTierState.status === "loading";
      button.dataset.active = String(codexServiceTierState.controlMode === "custom");
    });
    document.querySelectorAll("[data-codex-service-tier-thread-inherit]").forEach((button) => {
      button.disabled = !featureEnabled || !backendConnected || codexServiceTierState.status === "loading";
      button.dataset.active = String(codexServiceTierState.controlMode === "custom" && codexServiceTierState.threadMode === "inherit");
      button.title = t("serviceTierThreadInheritDynamicTitle", { mode: codexServiceTierState.defaultMode || "inherit" });
    });
    document.querySelectorAll("[data-codex-service-tier-thread-standard]").forEach((button) => {
      button.disabled = !featureEnabled || !backendConnected || codexServiceTierState.status === "loading";
      button.dataset.active = String(codexServiceTierState.controlMode === "custom" && codexServiceTierState.threadMode === "standard");
    });
    document.querySelectorAll("[data-codex-service-tier-thread-fast]").forEach((button) => {
      button.disabled = fastDisabled;
      button.dataset.active = String(codexServiceTierState.controlMode === "custom" && codexServiceTierState.threadMode === "fast");
      button.title = fastTitle;
    });
    refreshCodexServiceTierBadges();
  }

  async function loadCodexServiceTierState() {
    if (!codexPlusSettings().serviceTierControls) {
      codexServiceTierState = { ...codexServiceTierState, status: "idle", message: t("serviceTierDisabled") };
      refreshCodexServiceTierControls();
      return;
    }
    codexServiceTierState = { ...codexServiceTierState, status: "loading", message: t("serviceTierReading") };
    refreshCodexServiceTierControls();
    try {
      const serviceTier = await getCodexServiceTierSetting();
      codexServiceTierState = {
        ...codexServiceTierState,
        status: "ok",
        serviceTier,
        message: serviceTierGlobalStatusMessage(serviceTier),
      };
    } catch (error) {
      codexServiceTierState = {
        ...codexServiceTierState,
        status: "failed",
        message: t("serviceTierReadFailed"),
      };
      sendCodexPlusDiagnostic("service_tier_read_failed", {
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
    } finally {
      refreshCodexServiceTierControls();
    }
  }

  function setCodexThreadServiceTierMode(mode) {
    if (codexPlusBackendStatus.status !== "ok") {
      showToast(t("serviceTierBackendDisconnectedToast"), null);
      refreshCodexServiceTierControls();
      return;
    }
    const normalizedMode = normalizeCodexThreadServiceTierMode(mode);
    if (normalizedMode === "fast") {
      const fastAvailability = codexServiceTierFastAvailability();
      if (!fastAvailability.supported) {
        codexServiceTierMaybeLoadModelCatalog(true);
        showToast(codexServiceTierFastUnsupportedMessage(fastAvailability.modelName), null);
        refreshCodexServiceTierControls();
        return;
      }
    }
    const threadId = validThreadScrollSessionKey(currentSessionRef().session_id);
    setCodexThreadServiceTierOverride(threadId, normalizedMode);
    refreshCodexServiceTierControls();
    const target = threadId ? t("serviceTierThreadTargetCurrent") : t("serviceTierThreadTargetDraft");
    showToast(t("serviceTierThreadToast", { target, mode: normalizedMode === "inherit" ? t("serviceTierInherit") : normalizedMode }), null);
  }

  function toggleCodexServiceTierFromBadge() {
    if (codexPlusBackendStatus.status !== "ok") {
      showToast(t("serviceTierBackendDisconnectedToast"), null);
      refreshCodexServiceTierControls();
      return;
    }
    syncCodexServiceTierEffectiveState();
    const nextMode = codexServiceTierState.effectiveMode === "fast" ? "standard" : "fast";
    if (nextMode === "fast") {
      const fastAvailability = codexServiceTierFastAvailability();
      if (!fastAvailability.supported) {
        codexServiceTierMaybeLoadModelCatalog(true);
        showToast(codexServiceTierFastUnsupportedMessage(fastAvailability.modelName), null);
        refreshCodexServiceTierControls();
        return;
      }
    }
    setCodexThreadServiceTierMode(nextMode);
  }

  function codexServiceTierRequestMethods() {
    return new Set(["thread/start", "thread/resume", "turn/start"]);
  }

  function codexServiceTierThreadIdForRequest(method, params, threadIdHint = "") {
    if (method === "thread/start") return validThreadScrollSessionKey(params?.threadId || threadIdHint);
    return validThreadScrollSessionKey(params?.threadId || params?.conversationId || threadIdHint || currentSessionRef().session_id);
  }

  function codexServiceTierOverrideResult(method, params, threadIdHint, mode, requestedServiceTier, modelHint = "") {
    const threadId = codexServiceTierThreadIdForRequest(method, params, threadIdHint);
    const requestedFast = isFastServiceTierValue(requestedServiceTier);
    const modelName = codexServiceTierModelForRequest(params, modelHint);
    const fastSupported = !requestedFast || codexServiceTierFastSupportedForModel(modelName);
    return {
      threadId,
      mode,
      serviceTier: requestedFast && fastSupported ? codexFastServiceTierValue() : null,
      requestedServiceTier: requestedServiceTier || null,
      modelName,
      fastSupported,
      fastBlocked: requestedFast && !fastSupported,
    };
  }

  function codexServiceTierOverrideForRequest(method, params, threadIdHint = "") {
    if (!codexPlusSettings().serviceTierControls) return null;
    if (!codexServiceTierRequestMethods().has(method) || !params || typeof params !== "object") return null;
    const state = readThreadServiceTierState();
    const controlMode = normalizeCodexServiceTierControlMode(state.mode);
    const defaultMode = normalizeCodexThreadServiceTierMode(state.defaultMode);
    if (controlMode === "inherit") {
      const inheritedServiceTier = params.serviceTier ?? params.service_tier ?? codexServiceTierState.serviceTier;
      const override = codexServiceTierOverrideResult(method, params, threadIdHint, "inherit", inheritedServiceTier);
      return override.fastBlocked ? override : null;
    }
    if (controlMode === "global-standard" || controlMode === "global-fast") {
      return codexServiceTierOverrideResult(
        method,
        params,
        threadIdHint,
        controlMode,
        controlMode === "global-fast" ? codexFastServiceTierValue() : null
      );
    }
    const threadId = codexServiceTierThreadIdForRequest(method, params, threadIdHint);
    const override = threadId ? codexThreadServiceTierOverride(threadId) : codexThreadServiceTierDraft();
    const mode = codexServiceTierEffectiveThreadMode(override?.mode, defaultMode);
    if (mode === "inherit") {
      const inheritedServiceTier = params.serviceTier ?? params.service_tier ?? codexServiceTierState.serviceTier;
      const inheritedOverride = codexServiceTierOverrideResult(method, params, threadIdHint, "inherit", inheritedServiceTier);
      return inheritedOverride.fastBlocked ? { ...inheritedOverride, threadId, mode } : null;
    }
    return {
      ...codexServiceTierOverrideResult(method, params, threadIdHint, mode, mode === "fast" ? codexFastServiceTierValue() : null),
      threadId,
      mode,
    };
  }

  function applyCodexServiceTierRequestOverride(method, params, threadIdHint = "") {
    const override = codexServiceTierOverrideForRequest(method, params, threadIdHint);
    if (!override) return params;
    const nextParams = { ...(params || {}), serviceTier: override.serviceTier };
    if (Object.prototype.hasOwnProperty.call(nextParams, "service_tier") || override.fastBlocked) {
      nextParams.service_tier = override.serviceTier;
    }
    sendCodexPlusDiagnostic("service_tier_request_override_applied", {
      method,
      threadId: override.threadId || "",
      mode: override.mode,
      serviceTier: override.serviceTier || "standard",
      model: override.modelName || "",
      fastSupported: override.fastSupported !== false,
      fastBlocked: !!override.fastBlocked,
    });
    return nextParams;
  }

  function codexServiceTierRequestOverride(message) {
    if (!codexPlusSettings().serviceTierControls) return message;
    if (!message || typeof message !== "object") return message;
    if (message.type === "send-cli-request-for-host") {
      const method = String(message.method || "");
      const params = applyCodexServiceTierRequestOverride(method, message.params);
      return params === message.params ? message : { ...message, params };
    }
    if (message.type === "mcp-request" && message.request && typeof message.request === "object") {
      const method = String(message.request.method || "");
      const params = applyCodexServiceTierRequestOverride(method, message.request.params);
      if (params === message.request.params) return message;
      return { ...message, request: { ...message.request, params } };
    }
    if (message.type === "worker-request" && message.request && typeof message.request === "object") {
      const method = String(message.request.method || "");
      const params = applyCodexServiceTierRequestOverride(method, message.request.params);
      if (params === message.request.params) return message;
      return { ...message, request: { ...message.request, params } };
    }
    if (message.type === "thread-prewarm-start" && message.request && typeof message.request === "object") {
      const params = applyCodexServiceTierRequestOverride("thread/start", message.request.params);
      if (params === message.request.params) return message;
      return { ...message, request: { ...message.request, params } };
    }
    if (message.type === "start-conversation") {
      const nextMessage = applyCodexServiceTierRequestOverride("thread/start", message);
      return nextMessage === message ? message : nextMessage;
    }
    if (message.type === "prewarm-thread-start-for-host" && message.params && typeof message.params === "object") {
      const params = applyCodexServiceTierRequestOverride("thread/start", message.params);
      return params === message.params ? message : { ...message, params };
    }
    if (message.type === "start-thread-for-host") {
      const params = applyCodexServiceTierRequestOverride("thread/start", message);
      return params === message ? message : params;
    }
    if (message.type === "start-turn-for-host" && message.params && typeof message.params === "object") {
      const params = applyCodexServiceTierRequestOverride("turn/start", message.params, message.conversationId);
      return params === message.params ? message : { ...message, params };
    }
    return message;
  }

  function installCodexServiceTierDispatcherPatch() {
    if (window.__codexServiceTierRequestOverrideInstalled === codexServiceTierRequestOverrideVersion) return;
    const patch = async () => {
      try {
        const module = await loadCodexAppModule("setting-storage-");
        const dispatcherClass = typeof module.v === "function" && String(module.v).includes("dispatchMessage") ? module.v : null;
        const dispatcher = dispatcherClass?.getInstance?.();
        if (!dispatcher || typeof dispatcher.dispatchMessage !== "function") throw new Error("Codex dispatcher unavailable");
        if (dispatcher.__codexServiceTierOriginalDispatchMessage) {
          window.__codexServiceTierRequestOverrideInstalled = codexServiceTierRequestOverrideVersion;
          return;
        }
        dispatcher.__codexServiceTierOriginalDispatchMessage = dispatcher.dispatchMessage.bind(dispatcher);
        dispatcher.dispatchMessage = (type, payload) => {
          const message = codexServiceTierRequestOverride({ ...(payload || {}), type });
          const nextType = message?.type || type;
          const { type: _type, ...nextPayload } = message || {};
          return dispatcher.__codexServiceTierOriginalDispatchMessage(nextType, nextPayload);
        };
        window.__codexServiceTierRequestOverrideInstalled = codexServiceTierRequestOverrideVersion;
        sendCodexPlusDiagnostic("service_tier_dispatcher_patch_installed", {});
      } catch (error) {
        sendCodexPlusDiagnostic("service_tier_dispatcher_patch_failed", {
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
      }
    };
    void patch();
  }

  async function loadBackendSettings() {
    try {
      const settings = await postJson("/settings/get", {});
      if (!settings || typeof settings !== "object" || (!("launchMode" in settings) && !("enhancementsEnabled" in settings) && !("providerSyncEnabled" in settings))) {
        throw new Error("invalid backend settings response");
      }
      codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
      codexPlusBackendSettingsLoaded = true;
      refreshCodexPlusBackendToggles();
      return true;
    } catch (_) {
      refreshCodexPlusBackendToggles();
      return false;
    }
  }

  function loadBackendSettingsForStartup(attempt = 0) {
    loadBackendSettings().then((loaded) => {
      if (loaded) {
        scan();
        return;
      }
      if (attempt < 60) {
        setTimeout(() => loadBackendSettingsForStartup(attempt + 1), 250);
      }
    });
  }

  async function setBackendSetting(key, value) {
    codexPlusBackendSettings = { ...codexPlusBackendSettings, [key]: value };
    refreshCodexPlusBackendToggles();
    try {
      const settings = await postJson("/settings/set", { [key]: value });
      codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
    } finally {
      refreshCodexPlusBackendToggles();
    }
  }

  function refreshCodexPlusBackendToggles() {
    document.querySelectorAll(".codex-plus-toggle[data-codex-backend-setting]").forEach((button) => {
      const key = button.getAttribute("data-codex-backend-setting");
      setCodexPlusToggleState(button, !!codexPlusBackendSettings[key]);
    });
    renderCodexPlusMenu();
    scan();
  }

  let codexPlusUserScripts = { enabled: true, builtin_dir: "", user_dir: "", scripts: [] };
  let codexPlusBackendStatus = window.__codexPlusBackendStatusCache || { status: "checking" };
  let codexPlusLastOkBackendStatus = window.__codexPlusLastOkBackendStatusCache || (codexPlusBackendStatus?.status === "ok" ? codexPlusBackendStatus : null);
  let codexPlusBackendFailureCount = window.__codexPlusBackendFailureCount || 0;
  let codexPlusBackendLastOkAt = window.__codexPlusBackendLastOkAt || (codexPlusLastOkBackendStatus ? Date.now() : 0);
  let codexPlusBackendCheckSeq = 0;

  function setCodexPlusTriggerLabel(trigger) {
    if (!trigger) return;
    let label = trigger.querySelector("[data-codex-plus-trigger-label]");
    if (!label) {
      label = document.createElement("span");
      label.dataset.codexPlusTriggerLabel = "true";
      trigger.appendChild(label);
    }
    label.textContent = `Codex++ ${codexPlusVersion}`;
  }

  function ensureCodexPlusTriggerIndicator(trigger) {
    if (!trigger) return null;
    let indicator = trigger.querySelector("[data-codex-backend-indicator]");
    if (!indicator) {
      indicator = document.createElement("span");
      indicator.className = "codex-plus-backend-indicator";
      indicator.dataset.codexBackendIndicator = "true";
      trigger.prepend(indicator);
    }
    return indicator;
  }

  function setCodexPlusTriggerStatus(trigger) {
    const indicator = ensureCodexPlusTriggerIndicator(trigger);
    if (indicator) indicator.dataset.status = backendStatusForDisplay().status || "checking";
  }

  function renderBackendStatus() {
    const statusPayload = backendStatusForDisplay();
    const status = statusPayload.status || "failed";
    if (codexPlusBackendStatus.version) {
      codexPlusVersion = codexPlusBackendStatus.version;
      document.querySelectorAll("[data-codex-plus-version]").forEach((node) => {
        node.textContent = `Codex++ ${codexPlusVersion}`;
      });
      document.querySelectorAll(`#${codexPlusMenuId} button`).forEach(setCodexPlusTriggerLabel);
    }
    const label = document.querySelector("[data-codex-backend-status]");
    if (label) {
      label.dataset.status = status;
      label.textContent = backendStatusText(statusPayload);
    }
    document.querySelectorAll("[data-codex-backend-indicator]").forEach((indicator) => {
      indicator.dataset.status = status;
      indicator.title = status === "ok" ? t("backendConnected") : status === "checking" ? t("backendIndicatorChecking") : t("backendIndicatorDisconnected");
    });
    const repair = document.querySelector("[data-codex-backend-repair]");
    if (repair) repair.hidden = status === "ok" || status === "checking";
    refreshCodexServiceTierControls();
  }

  function backendStatusText(statusPayload = codexPlusBackendStatus) {
    const status = statusPayload?.status || "failed";
    if (status === "ok") return t("backendConnected");
    if (status === "checking") return statusPayload?.repairing ? t("backendRepairing") : t("backendChecking");
    if (statusPayload?.timeout) return t("backendTimeout");
    if (statusPayload?.repairFailed) return t("backendRepairFailed");
    return t("backendDisconnected");
  }

  function backendStatusForDisplay() {
    if (codexPlusBackendStatus?.status === "ok") return codexPlusBackendStatus;
    const withinGrace = codexPlusBackendFailureCount > 0 && codexPlusBackendFailureCount < 2 && codexPlusBackendLastOkAt && Date.now() - codexPlusBackendLastOkAt <= 12_000;
    if (withinGrace && codexPlusLastOkBackendStatus) return codexPlusLastOkBackendStatus;
    return codexPlusBackendStatus || { status: "checking" };
  }

  function setBackendStatusForHeartbeat(nextStatus) {
    if (nextStatus?.status === "ok") {
      codexPlusBackendStatus = nextStatus;
      codexPlusLastOkBackendStatus = nextStatus;
      codexPlusBackendFailureCount = 0;
      codexPlusBackendLastOkAt = Date.now();
      window.__codexPlusBackendStatusCache = nextStatus;
      window.__codexPlusLastOkBackendStatusCache = nextStatus;
      window.__codexPlusBackendFailureCount = 0;
      window.__codexPlusBackendLastOkAt = codexPlusBackendLastOkAt;
      return;
    }
    codexPlusBackendFailureCount += 1;
    window.__codexPlusBackendFailureCount = codexPlusBackendFailureCount;
    const staleOk = !codexPlusBackendLastOkAt || Date.now() - codexPlusBackendLastOkAt > 12_000;
    if (!codexPlusLastOkBackendStatus || codexPlusBackendFailureCount >= 2 || staleOk) {
      codexPlusBackendStatus = nextStatus || { status: "failed" };
      window.__codexPlusBackendStatusCache = codexPlusBackendStatus;
    }
  }

  async function checkBackendStatus() {
    const seq = ++codexPlusBackendCheckSeq;
    const nextStatus = await postJson("/backend/status", {});
    if (seq !== codexPlusBackendCheckSeq) return;
    setBackendStatusForHeartbeat(nextStatus);
    if (nextStatus?.status !== "ok") {
      sendCodexPlusDiagnostic("backend_check_failed", {
        status: nextStatus?.status || "unknown",
        message: nextStatus?.message || "",
        timeout: !!nextStatus?.timeout,
      });
    }
    renderBackendStatus();
  }

  async function repairBackend() {
    codexPlusBackendStatus = { status: "checking", repairing: true };
    renderBackendStatus();
    try {
      codexPlusBackendStatus = await postJson("/backend/repair", {});
      if (codexPlusBackendStatus?.status === "ok") {
        codexPlusLastOkBackendStatus = codexPlusBackendStatus;
        codexPlusBackendFailureCount = 0;
        codexPlusBackendLastOkAt = Date.now();
        window.__codexPlusLastOkBackendStatusCache = codexPlusBackendStatus;
        window.__codexPlusBackendFailureCount = 0;
        window.__codexPlusBackendLastOkAt = codexPlusBackendLastOkAt;
      }
      window.__codexPlusBackendStatusCache = codexPlusBackendStatus;
    } catch (error) {
      codexPlusBackendStatus = { status: "failed", repairFailed: true };
      window.__codexPlusBackendStatusCache = codexPlusBackendStatus;
    }
    renderBackendStatus();
  }

  async function openManagerFromCodex() {
    const result = await postJson("/manager/open", {});
    if (result.status === "ok") {
      showToast(t("managerOpened"), null);
    } else {
      showToast(result.message || t("managerOpenFailed"), null);
    }
  }

  function scheduleBackendHeartbeat() {
    if (window.__codexPlusBackendHeartbeat && window.__codexPlusBackendHeartbeatVersion !== codexPlusBackendHeartbeatVersion) {
      clearInterval(window.__codexPlusBackendHeartbeat);
      window.__codexPlusBackendHeartbeat = null;
    }
    if (window.__codexPlusBackendHeartbeat) return;
    window.__codexPlusBackendHeartbeatVersion = codexPlusBackendHeartbeatVersion;
    window.__codexPlusBackendHeartbeat = setInterval(checkBackendStatus, 5000);
    checkBackendStatus();
  }

  function userScriptStatusLabel(status) {
    return {
      loaded: t("userScriptLoaded"),
      failed: t("userScriptFailed"),
      disabled: t("userScriptDisabled"),
      not_loaded: t("userScriptNotLoaded"),
      loading: t("userScriptLoading"),
    }[status] || status || t("userScriptUnknown");
  }

  function renderUserScripts() {
    const enabledToggle = document.querySelector("[data-codex-user-scripts-enabled]");
    if (enabledToggle) setCodexPlusToggleState(enabledToggle, !!codexPlusUserScripts.enabled);
    const dirs = document.querySelector("[data-codex-user-script-dirs]");
    if (dirs) {
      dirs.textContent = t("userScriptDirs", {
        builtin: codexPlusUserScripts.builtin_dir || t("userScriptDirMissing"),
        user: codexPlusUserScripts.user_dir || t("userScriptDirMissing"),
      });
    }
    const list = document.querySelector("[data-codex-user-script-list]");
    if (!list) return;
    if (!codexPlusUserScripts.scripts?.length) {
      list.textContent = t("userScriptEmpty");
      return;
    }
    list.innerHTML = codexPlusUserScripts.scripts.map((script) => `
      <div class="codex-plus-user-script-item">
        <div>
          <div class="codex-plus-user-script-name">${escapeHtml(script.name || script.key)}</div>
          <div class="codex-plus-user-script-meta">${script.source === "builtin" ? codexPlusEscapedText("userScriptBuiltin") : codexPlusEscapedText("userScriptUser")} · ${escapeHtml(userScriptStatusLabel(script.status))}</div>
          ${script.error ? `<div class="codex-plus-user-script-error">${escapeHtml(script.error)}</div>` : ""}
        </div>
        <button type="button" class="codex-plus-toggle" data-codex-user-script-key="${escapeHtml(script.key)}" data-enabled="${String(!!script.enabled)}"><span></span></button>
      </div>
    `).join("");
    list.querySelectorAll(".codex-plus-toggle[data-codex-user-script-key]").forEach((button) => {
      setCodexPlusToggleState(button, button.dataset.enabled === "true");
    });
  }

  async function loadUserScripts(path = "/user-scripts/list", payload = {}) {
    const result = await postJson(path, payload);
    if (result?.scripts) {
      codexPlusUserScripts = result;
      renderUserScripts();
    }
  }

  const codexPlusAdsUrl = "/ads";
  let codexPlusAds = [];
  let codexPlusAdsLoaded = false;

  function isCodexPlusAdExpired(ad) {
    if (!ad.expires_at) return false;
    const expiresAt = Date.parse(ad.expires_at);
    return Number.isFinite(expiresAt) && expiresAt < Date.now();
  }

  function normalizeCodexPlusAds(payload) {
    if (!payload || !Array.isArray(payload.ads)) return [];
    return payload.ads.filter((ad) => {
      return ad && ["sponsor", "normal"].includes(ad.type) && ad.title && ad.description && ad.url && !isCodexPlusAdExpired(ad);
    }).map((ad) => ({
      id: String(ad.id || ad.title),
      type: ad.type,
      title: String(ad.title),
      description: String(ad.description),
      url: String(ad.url),
      expires_at: ad.expires_at ? String(ad.expires_at) : "",
      highlights: Array.isArray(ad.highlights) ? ad.highlights.map((item) => String(item)).filter(Boolean) : [],
    }));
  }

  function renderCodexPlusAdGroup(type, emptyText) {
    const ads = codexPlusAds.filter((ad) => ad.type === type);
    if (!ads.length) return `<div class="codex-plus-ad-empty">${escapeHtml(emptyText)}</div>`;
    return ads.map((ad) => `
      <article class="codex-plus-ad-card">
        <div class="codex-plus-ad-content">
          <h3 class="codex-plus-ad-title">${escapeHtml(ad.title)}</h3>
          <p class="codex-plus-ad-description">${escapeHtml(ad.description)}</p>
          <div class="codex-plus-ad-highlights">
            ${ad.highlights.map((item) => `<span>${escapeHtml(item)}</span>`).join("")}
          </div>
          <a class="codex-plus-ad-link" href="${escapeHtml(ad.url)}" target="_blank" rel="noreferrer">${codexPlusEscapedText("visitHost", { host: new URL(ad.url).hostname })}</a>
        </div>
      </article>
    `).join("");
  }

  function renderCodexPlusAds() {
    if (!codexPlusAdsLoaded) return `<div class="codex-plus-ad-empty">${codexPlusEscapedText("adsLoading")}</div>`;
    if (!codexPlusAds.length) return `<div class="codex-plus-ad-empty">${codexPlusEscapedText("adsEmpty")}</div>`;
    return `
      <section class="codex-plus-ad-section">
        <h3 class="codex-plus-ad-section-title">${codexPlusEscapedText("sponsorAdsTitle")}</h3>
        <div class="codex-plus-ad-list">${renderCodexPlusAdGroup("sponsor", t("sponsorAdsEmpty"))}</div>
      </section>
      <section class="codex-plus-ad-section">
        <h3 class="codex-plus-ad-section-title">${codexPlusEscapedText("normalAdsTitle")}</h3>
        <div class="codex-plus-ad-list">${renderCodexPlusAdGroup("normal", t("normalAdsEmpty"))}</div>
      </section>
    `;
  }

  function cacheBustCodexPlusAdUrl(url, version) {
    return `${url}${url.includes("?") ? "&" : "?"}v=${version}`;
  }

  async function directFetchCodexPlusAds() {
    const urls = [
      "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
      "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json",
    ];
    let lastError = null;
    const cacheBust = Date.now();
    for (const url of urls) {
      try {
        const response = await fetch(cacheBustCodexPlusAdUrl(url, cacheBust), {
          headers: { "Accept": "application/json" },
          cache: "no-store",
        });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return await response.json();
      } catch (error) {
        lastError = error;
      }
    }
    throw lastError || new Error("ad list unavailable");
  }

  async function fetchCodexPlusAds() {
    try {
      codexPlusAds = normalizeCodexPlusAds(await directFetchCodexPlusAds());
    } catch (error) {
      sendCodexPlusDiagnostic("ads_fetch_failed", {
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
      codexPlusAds = [];
    } finally {
      codexPlusAdsLoaded = true;
      const panel = document.querySelector('[data-codex-plus-panel="sponsor"] .codex-plus-ad-remote');
      if (panel) panel.innerHTML = renderCodexPlusAds();
    }
  }

  function selectCodexPlusTab(tab) {
    document.querySelectorAll(".codex-plus-modal-content").forEach((modal) => {
      modal.dataset.codexPlusActiveTab = tab;
    });
    document.querySelectorAll("[data-codex-plus-tab]").forEach((button) => {
      button.dataset.active = String(button.getAttribute("data-codex-plus-tab") === tab);
    });
    document.querySelectorAll("[data-codex-plus-panel]").forEach((panel) => {
      panel.hidden = panel.getAttribute("data-codex-plus-panel") !== tab;
    });
    if (tab === "userScripts") loadUserScripts();
  }

  function openCodexPlusModal() {
    const previousTab = document.querySelector(".codex-plus-modal-content")?.dataset.codexPlusActiveTab || "home";
    document.querySelectorAll(".codex-plus-modal-overlay").forEach((node) => node.remove());
    document.querySelectorAll('[data-codex-plus-dialog="true"]').forEach((node) => node.remove());
    const backendDisplayStatus = backendStatusForDisplay();
    const backendDisplayStatusName = backendDisplayStatus.status || "failed";
    const relayMode = codexPlusBackendSettings.launchMode === "relay";
    const relayUnneededAttrs = relayMode
      ? `disabled data-relay-unneeded="true" data-codex-plus-unneeded-label="${codexPlusEscapedText("relaunchNotNeeded")}"`
      : "";
    const overlay = document.createElement("div");
    overlay.className = "codex-plus-modal-overlay";
    overlay.innerHTML = `
      <div class="codex-plus-modal-content" role="dialog" aria-modal="true" aria-label="Codex++">
        <div class="codex-plus-modal-header">
          <div class="codex-plus-modal-title"><span class="codex-plus-backend-indicator" data-codex-backend-indicator="true" data-status="${escapeHtml(backendDisplayStatusName)}"></span><span data-codex-plus-version="true">Codex++ ${codexPlusVersion}</span></div>
          <div class="codex-plus-modal-header-actions">
            <button type="button" class="codex-plus-modal-close" aria-label="${codexPlusEscapedText("close")}">×</button>
          </div>
        </div>
        <div class="codex-plus-tabs" role="tablist" aria-label="Codex++">
          <button type="button" class="codex-plus-tab-button" data-codex-plus-tab="home" data-active="true">${codexPlusEscapedText("tabHome")}</button>
          <button type="button" class="codex-plus-tab-button" data-codex-plus-tab="userScripts" data-active="false">${codexPlusEscapedText("tabUserScripts")}</button>
          <button type="button" class="codex-plus-tab-button" data-codex-plus-tab="sponsor" data-active="false">${codexPlusEscapedText("tabSponsor")}</button>
          <button type="button" class="codex-plus-tab-button" data-codex-plus-tab="support" data-active="false">${codexPlusEscapedText("tabSupport")}</button>
        </div>
        <div class="codex-plus-modal-body">
          <div class="codex-plus-panel" data-codex-plus-panel="home">
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("backendTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("backendDescription")}</div></div>
              <div class="codex-plus-backend-status">
                <div class="codex-plus-backend-label" data-codex-backend-status="true" data-status="${escapeHtml(backendDisplayStatusName)}">${escapeHtml(backendStatusText(backendDisplayStatus))}</div>
                <button type="button" class="codex-plus-backend-repair" data-codex-backend-repair="true" ${backendDisplayStatusName === "ok" || backendDisplayStatusName === "checking" ? "hidden" : ""}>${codexPlusEscapedText("backendRepair")}</button>
              </div>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("enhancementsTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("enhancementsDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-backend-setting="enhancementsEnabled"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("marketplaceTitle")}</div><div class="codex-plus-row-description">${relayMode ? codexPlusEscapedText("marketplaceRelayDescription") : codexPlusEscapedText("marketplacePatchDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="pluginMarketplaceUnlock" ${relayUnneededAttrs}><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("pluginEntryTitle")}</div><div class="codex-plus-row-description">${relayMode ? codexPlusEscapedText("pluginEntryRelayDescription") : codexPlusEscapedText("pluginEntryPatchDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="pluginEntryUnlock" ${relayUnneededAttrs}><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("forcePluginInstallTitle")}</div><div class="codex-plus-row-description">${relayMode ? codexPlusEscapedText("forcePluginInstallRelayDescription") : codexPlusEscapedText("forcePluginInstallPatchDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="forcePluginInstall" ${relayUnneededAttrs}><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("modelWhitelistTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("modelWhitelistDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="modelWhitelistUnlock"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("fastButtonTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("fastButtonDescription", { models: codexServiceTierFastModelListLabel() })}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="serviceTierControls"><span></span></button>
            </div>
            <div class="codex-plus-row" data-codex-service-tier-controls="true">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("serviceTierTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("serviceTierDescription")}</div></div>
              <div class="codex-plus-service-tier-control">
                <div class="codex-plus-service-tier-status" data-codex-service-tier-status="true" data-status="loading">${codexPlusEscapedText("serviceTierReading")}</div>
                <div class="codex-plus-service-tier-actions">
                  <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-inherit="true">${codexPlusEscapedText("serviceTierInherit")}</button>
                  <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-standard="true">${codexPlusEscapedText("serviceTierGlobalStandard")}</button>
                  <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-fast="true">${codexPlusEscapedText("serviceTierGlobalFast")}</button>
                  <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-custom="true">${codexPlusEscapedText("serviceTierCustom")}</button>
                </div>
                <div class="codex-plus-service-tier-actions codex-plus-service-tier-thread-actions">
                  <span class="codex-plus-service-tier-thread-label">${codexPlusEscapedText("serviceTierThreadOverride")}</span>
                  <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-thread-inherit="true" title="${codexPlusEscapedText("serviceTierThreadInheritTitle")}">${codexPlusEscapedText("serviceTierInherit")}</button>
                  <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-thread-standard="true" title="${codexPlusEscapedText("serviceTierThreadStandardTitle")}">Standard</button>
                  <button type="button" class="codex-plus-service-tier-button" data-codex-service-tier-thread-fast="true" title="${codexPlusEscapedText("serviceTierThreadFastTitle")}">Fast</button>
                </div>
              </div>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("sessionDeleteTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("sessionDeleteDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="sessionDelete"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("markdownExportTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("markdownExportDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="markdownExport"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("pasteFixTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("pasteFixDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="pasteFix"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("projectMoveTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("projectMoveDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="projectMove"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("timelineTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("timelineDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="conversationTimeline"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("threadIdBadgeTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("threadIdBadgeDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="threadIdBadge"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("conversationViewTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("conversationViewDescription")}</div></div>
              <div class="codex-plus-width-control">
                <input class="codex-plus-width-input" data-codex-plus-conversation-view-width="true" min="${conversationViewMinWidth}" max="${conversationViewMaxAllowedWidth}" step="10" type="number" value="${conversationViewWidth()}">
                <button type="button" class="codex-plus-toggle" data-codex-plus-setting="conversationView"><span></span></button>
              </div>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("threadScrollTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("threadScrollDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="threadScrollRestore"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("zedRemoteTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("zedRemoteDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="zedRemoteOpen"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("upstreamWorktreeTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("upstreamWorktreeDescription")}</div></div>
              <div class="codex-plus-worktree-actions">
                <button type="button" class="codex-plus-action-button" data-codex-upstream-worktree-open="true">${codexPlusEscapedText("create")}</button>
                <button type="button" class="codex-plus-toggle" data-codex-plus-setting="upstreamWorktreeCreate"><span></span></button>
              </div>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("providerSyncTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("providerSyncDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-backend-setting="providerSyncEnabled"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("pageModeTitle")}</div><div class="codex-plus-row-description">${relayMode ? codexPlusEscapedText("pageModeRelayDescription") : codexPlusEscapedText("pageModePatchDescription")}</div></div>
              <button type="button" class="codex-plus-action-button" data-codex-open-manager="true">${codexPlusEscapedText("openManager")}</button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("nativeMenuTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("nativeMenuDescription")}</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-plus-setting="nativeMenuPlacement"><span></span></button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("devtoolsTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("devtoolsDescription")}</div></div>
              <button type="button" class="codex-plus-action-button" data-codex-open-devtools="true">${codexPlusEscapedText("openDevtools")}</button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("aboutTitle")}</div><div class="codex-plus-about">${codexPlusEscapedText("aboutDescription")}<br>Build: <span data-codex-plus-build="true">${codexPlusBuild}</span><br>GitHub: <a href="https://github.com/BigPizzaV3/CodexPlusPlus" target="_blank" rel="noreferrer">https://github.com/BigPizzaV3/CodexPlusPlus</a><br>Discord: <a href="https://discord.gg/y96kX7A76v" target="_blank" rel="noreferrer">https://discord.gg/y96kX7A76v</a><br>Telegram: <a href="https://t.me/CodexPlusPlus" target="_blank" rel="noreferrer">https://t.me/CodexPlusPlus</a></div></div>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("discordTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("discordDescription")}</div></div>
              <button type="button" class="codex-plus-action-button" data-codex-plus-discord="true">${codexPlusEscapedText("openDiscord")}</button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("telegramTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("telegramDescription")}</div></div>
              <button type="button" class="codex-plus-action-button" data-codex-plus-telegram="true">${codexPlusEscapedText("openTelegram")}</button>
            </div>
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">${codexPlusEscapedText("issueTitle")}</div><div class="codex-plus-row-description">${codexPlusEscapedText("issueDescription")}</div></div>
              <button type="button" class="codex-plus-issue-button" data-codex-plus-issue="true">${codexPlusEscapedText("openIssue")}</button>
            </div>
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="userScripts" hidden>
            <div class="codex-plus-row" data-codex-user-scripts-section="true">
              <div>
                <div class="codex-plus-row-title">${codexPlusEscapedText("userScriptsTitle")}</div>
                <div class="codex-plus-row-description">${codexPlusEscapedText("userScriptsDescription")}</div>
                <div class="codex-plus-user-script-warning">${codexPlusEscapedText("userScriptsWarning")}</div>
                <div class="codex-plus-user-script-dirs" data-codex-user-script-dirs="true">${codexPlusEscapedText("userScriptsReadingDirs")}</div>
                <div class="codex-plus-user-script-list" data-codex-user-script-list="true">${codexPlusEscapedText("userScriptsReading")}</div>
              </div>
              <div class="codex-plus-user-script-actions">
                <button type="button" class="codex-plus-toggle" data-codex-user-scripts-enabled="true"><span></span></button>
                <button type="button" class="codex-plus-user-script-reload" data-codex-user-scripts-reload="true">${codexPlusEscapedText("reloadUserScripts")}</button>
              </div>
            </div>
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="sponsor" hidden>
            <div class="codex-plus-sponsor-text">${codexPlusEscapedText("sponsorDescription")}</div>
            <div class="codex-plus-ad-remote">
              ${renderCodexPlusAds()}
            </div>
          </div>
          <div class="codex-plus-panel" data-codex-plus-panel="support" hidden>
            <div class="codex-plus-sponsor-text">${codexPlusEscapedText("supportDescription")}</div>
            <div class="codex-plus-sponsor-grid">
              <div class="codex-plus-sponsor-card">
                <div class="codex-plus-sponsor-card-title">${codexPlusEscapedText("alipay")}</div>
                <img class="codex-plus-sponsor-qr" src="${window.__CODEX_PLUS_SPONSOR_IMAGES__?.alipay || `${helperBase}/assets/sponsor-alipay.jpg`}" alt="${codexPlusEscapedText("alipayQrAlt")}">
              </div>
              <div class="codex-plus-sponsor-card">
                <div class="codex-plus-sponsor-card-title">${codexPlusEscapedText("wechat")}</div>
                <img class="codex-plus-sponsor-qr" src="${window.__CODEX_PLUS_SPONSOR_IMAGES__?.wechat || `${helperBase}/assets/sponsor-wechat.jpg`}" alt="${codexPlusEscapedText("wechatQrAlt")}">
              </div>
            </div>
          </div>
        </div>
      </div>
    `;
    let codexPlusModalClosed = false;
    const closeCodexPlusModal = () => {
      if (codexPlusModalClosed) return;
      codexPlusModalClosed = true;
      document.removeEventListener("keydown", handleCodexPlusModalKeydown, true);
      overlay.remove();
    };
    const handleCodexPlusModalKeydown = (event) => {
      if (event.key !== "Escape") return;
      if (!overlay.isConnected) {
        closeCodexPlusModal();
        return;
      }
      const topDialog = document.querySelector(`.${upstreamWorktreeDialogClass}, .codex-delete-confirm-overlay, .${projectMoveOverlayClass}`);
      if (topDialog && topDialog !== overlay && !overlay.contains(topDialog)) return;
      event.preventDefault();
      event.stopPropagation();
      closeCodexPlusModal();
    };
    document.addEventListener("keydown", handleCodexPlusModalKeydown, true);
    const closeButton = overlay.querySelector(".codex-plus-modal-close");
    closeButton?.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      closeCodexPlusModal();
    }, true);
    overlay.addEventListener("input", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      const widthInput = target?.closest("[data-codex-plus-conversation-view-width]");
      if (widthInput) setConversationViewWidth(widthInput.value);
    }, true);
    overlay.addEventListener("change", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      const widthInput = target?.closest("[data-codex-plus-conversation-view-width]");
      if (widthInput) {
        const width = normalizeConversationViewWidth(widthInput.value);
        widthInput.value = String(width || conversationViewWidth());
        setConversationViewWidth(widthInput.value);
      }
    }, true);
    overlay.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      if (event.target === overlay || target?.closest(".codex-plus-modal-close")) {
        closeCodexPlusModal();
        return;
      }
      const tabButton = target?.closest("[data-codex-plus-tab]");
      if (tabButton) {
        selectCodexPlusTab(tabButton.getAttribute("data-codex-plus-tab"));
        return;
      }
      if (target?.closest("[data-codex-open-devtools]")) {
        postJson("/devtools/open", {});
        return;
      }
      if (target?.closest("[data-codex-open-manager]")) {
        openManagerFromCodex();
        return;
      }
      if (target?.closest("[data-codex-plus-discord]")) {
        window.open("https://discord.gg/y96kX7A76v", "_blank");
        return;
      }
      if (target?.closest("[data-codex-plus-telegram]")) {
        window.open("https://t.me/CodexPlusPlus", "_blank");
        return;
      }
      if (target?.closest("[data-codex-backend-repair]")) {
        repairBackend();
        return;
      }
      const issueButton = target?.closest("[data-codex-plus-issue]");
      if (issueButton) {
        const issueUrl = "https://github.com/BigPizzaV3/CodexPlusPlus/issues";
        window.open(issueUrl, "_blank");
        return;
      }
      const userScriptsEnabled = target?.closest("[data-codex-user-scripts-enabled]");
      if (userScriptsEnabled) {
        loadUserScripts("/user-scripts/set-enabled", { enabled: userScriptsEnabled.dataset.enabled !== "true" });
        return;
      }
      if (target?.closest("[data-codex-service-tier-inherit]")) {
        setCodexServiceTierControlMode("inherit");
        return;
      }
      if (target?.closest("[data-codex-service-tier-standard]")) {
        setCodexServiceTierControlMode("global-standard");
        return;
      }
      if (target?.closest("[data-codex-service-tier-fast]")) {
        setCodexServiceTierControlMode("global-fast");
        return;
      }
      if (target?.closest("[data-codex-service-tier-custom]")) {
        setCodexServiceTierControlMode("custom");
        return;
      }
      if (target?.closest("[data-codex-service-tier-thread-inherit]")) {
        setCodexThreadServiceTierMode("inherit");
        return;
      }
      if (target?.closest("[data-codex-service-tier-thread-standard]")) {
        setCodexThreadServiceTierMode("standard");
        return;
      }
      if (target?.closest("[data-codex-service-tier-thread-fast]")) {
        setCodexThreadServiceTierMode("fast");
        return;
      }
      const userScriptToggle = target?.closest("[data-codex-user-script-key]");
      if (userScriptToggle) {
        loadUserScripts("/user-scripts/set-script-enabled", { key: userScriptToggle.getAttribute("data-codex-user-script-key"), enabled: userScriptToggle.dataset.enabled !== "true" });
        return;
      }
      if (target?.closest("[data-codex-user-scripts-reload]")) {
        loadUserScripts("/user-scripts/reload", {});
        return;
      }
      if (target?.closest("[data-codex-upstream-worktree-open]")) {
        if (!codexPlusSettings().upstreamWorktreeCreate) {
          showToast(t("upstreamWorktreeDisabled"), null);
          return;
        }
        openUpstreamWorktreeDialog();
        return;
      }
      const toggle = target?.closest("[data-codex-plus-setting]");
      if (toggle) {
        if (toggle.disabled) return;
        const key = toggle.getAttribute("data-codex-plus-setting");
        setCodexPlusSetting(key, !codexPlusSettings()[key]);
        return;
      }
      const backendToggle = target?.closest("[data-codex-backend-setting]");
      if (backendToggle) {
        const key = backendToggle.getAttribute("data-codex-backend-setting");
        setBackendSetting(key, !codexPlusBackendSettings[key]);
        return;
      }
    }, true);
    document.body.appendChild(overlay);
    if (!codexPlusAdsLoaded) fetchCodexPlusAds();
    selectCodexPlusTab(previousTab);
    renderCodexPlusMenu();
    refreshCodexPlusBackendToggles();
    renderBackendStatus();
    void loadCodexServiceTierState();
    loadUserScripts();
  }

  function findNativeMenuInsertionPoint() {
    if (!codexPlusSettings().nativeMenuPlacement) return null;
    const header = document.querySelector(selectors.appHeader);
    const isIconOnlyButton = (button) => String(button.className || "").includes("aspect-square");
    const menuBar = Array.from(header?.querySelectorAll?.(selectors.nativeMenuBar) || [])
      .find((node) => !node.closest(".invisible") && elementHasVisibleBox(node));
    if (menuBar) {
      const buttons = Array.from(menuBar.querySelectorAll("button"))
        .filter((button) => !button.closest(`#${codexPlusMenuId}`) && elementHasVisibleBox(button));
      if (buttons.length && buttons.every(isIconOnlyButton)) return null;
      const openLocationButton = buttons.find((button) => /^(打开位置|Open location)$/i.test(button.getAttribute("aria-label") || ""));
      const openLocationGroup = openLocationButton?.closest?.(".inline-flex.self-start.items-stretch.overflow-hidden.rounded-lg");
      const openLocationIndex = buttons.indexOf(openLocationButton);
      const nativeButtonClass = openLocationButton
        ? buttons[openLocationIndex + 1]?.className || openLocationButton.className || ""
        : buttons[buttons.length - 1]?.className || "";
      if (openLocationGroup?.parentElement === menuBar) return { parent: menuBar, before: openLocationGroup, nativeButtonClass };
      if (openLocationGroup?.parentElement?.parentElement === menuBar) return { parent: menuBar, before: openLocationGroup.parentElement, nativeButtonClass };
      return { parent: menuBar, before: buttons[buttons.length - 1]?.nextSibling || null, nativeButtonClass: buttons[buttons.length - 1]?.className || "" };
    }
    const contextSurface = header?.querySelector(selectors.headerContextMenuSurface);
    const buttons = Array.from(contextSurface?.querySelectorAll?.("button") || [])
      .filter((button) => !button.closest(`#${codexPlusMenuId}`) && elementHasVisibleBox(button));
    if (buttons.length && buttons.every(isIconOnlyButton)) return null;
    const nativeButton = buttons.find((button) => !button.parentElement?.classList?.contains("inline-flex")) || buttons[0];
    const parent = nativeButton?.parentElement;
    if (!parent) {
      const emptyButtonGroup = Array.from(contextSurface?.querySelectorAll?.("div") || [])
        .find((node) => {
          const className = String(node.className || "");
          return className.includes("items-center") && className.includes("gap-2");
        });
      return emptyButtonGroup ? { parent: emptyButtonGroup, before: emptyButtonGroup.firstChild, nativeButtonClass: headerIconTextButtonClass } : null;
    }
    return { parent, before: nativeButton, nativeButtonClass: nativeButton.className || "" };
  }

  function removeDuplicateCodexPlusMenus(keep) {
    document.querySelectorAll(`#${codexPlusMenuId}, [data-codex-plus-menu="true"]`).forEach((node) => {
      if (node !== keep) node.remove();
    });
    Array.from(document.querySelectorAll("button")).forEach((button) => {
      if ((button.textContent || "").trim() === `Codex++ ${codexPlusVersion}` && !button.closest(`#${codexPlusMenuId}`)) {
        button.remove();
      }
    });
  }

  function normalizeCodexPlusTriggerClassName(className) {
    const classes = String(className || "").split(/\s+/).filter(Boolean);
    const incompatibleNativeGroupClasses = new Set(["gap-0", "rounded-l-none", "border-l-0", "pl-0.5", "pr-1.5", "aspect-square", "!px-0", "px-0"]);
    const hasIncompatibleNativeGroupClass = classes.some((name) => incompatibleNativeGroupClasses.has(name));
    const normalized = classes.filter((name) => !incompatibleNativeGroupClasses.has(name));
    if (hasIncompatibleNativeGroupClass) {
      ["gap-1", "rounded-lg", "border-l", "px-2"].forEach((name) => {
        if (!normalized.includes(name)) normalized.push(name);
      });
    }
    return normalized.join(" ");
  }

  function configureCodexPlusTrigger(menu, trigger, nativeButtonClass, nativeMenuPlacement = false) {
    if (!trigger) return;
    trigger.className = nativeButtonClass ? normalizeCodexPlusTriggerClassName(nativeButtonClass) : "codex-plus-trigger";
    if (nativeMenuPlacement) {
      trigger.classList.add("codex-plus-native-trigger");
      if (menu) menu.dataset.codexPlusNativeMenu = "true";
    } else {
      trigger.classList.remove("codex-plus-native-trigger");
      if (menu) delete menu.dataset.codexPlusNativeMenu;
    }
    setCodexPlusTriggerStatus(trigger);
    if (trigger.dataset.codexPlusTriggerInstalled === "9") return;
    trigger.dataset.codexPlusTriggerInstalled = "9";
    trigger.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      openCodexPlusModal();
    }, true);
  }

  function numericCssValue(value) {
    const parsed = Number.parseFloat(value || "");
    return Number.isFinite(parsed) ? parsed : 0;
  }

  function setCssPropIfChanged(menu, prop, value) {
    if (menu.style.getPropertyValue(prop) !== value) {
      menu.style.setProperty(prop, value);
    }
  }

  function headerTitleRegion(header) {
    const candidates = Array.from(header?.querySelectorAll?.('[data-state], [class*="truncate"], [class*="text-base"]') || []);
    return candidates.find((node) => {
      if (!node?.querySelector?.('[data-state], button')) return false;
      if (!node.textContent?.trim()) return false;
      return node.closest?.(".draggable") || node.closest?.('[class*="grid-cols-[minmax(0,1fr)]"]');
    }) || null;
  }

  function isHeaderToolbarButton(button, header, rect) {
    if (!button || button.closest?.(`#${codexPlusMenuId}`)) return false;
    if (!(rect.width > 0 && rect.height > 0 && rect.left > window.innerWidth / 2)) return false;
    const buttonCluster = button.closest(".ms-auto.flex.shrink-0.items-center");
    if (buttonCluster && header?.contains(buttonCluster)) return true;
    const titleRegion = headerTitleRegion(header);
    if (titleRegion?.contains?.(button)) return false;
    return !!button.closest?.('[class*="ms-auto"][class*="shrink-0"][class*="items-center"]');
  }

  function updateFloatingCodexPlusMenuPosition(menu) {
    if (!menu?.classList?.contains(codexPlusMenuFloatingClass)) return;
    const header = document.querySelector(selectors.appHeader) || document.querySelector("header");
    if (!header) return;
    const toolbarButtons = Array.from(header.querySelectorAll("button"))
      .map((button) => ({ button, rect: button.getBoundingClientRect() }))
      .filter(({ button, rect }) => isHeaderToolbarButton(button, header, rect))
      .sort((left, right) => left.rect.left - right.rect.left);
    const anchor = toolbarButtons[0];
    if (anchor) {
      const measuredGap = toolbarButtons[1] ? toolbarButtons[1].rect.left - toolbarButtons[0].rect.right : 0;
      const styles = anchor.button.parentElement ? getComputedStyle(anchor.button.parentElement) : null;
      const gap = Math.max(numericCssValue(styles?.columnGap || styles?.gap), measuredGap, 0);
      setCssPropIfChanged(menu, "--codex-plus-menu-top", `${anchor.rect.top}px`);
      setCssPropIfChanged(menu, "--codex-plus-menu-height", `${anchor.rect.height}px`);
      setCssPropIfChanged(menu, "--codex-plus-menu-right", `${Math.max(0, window.innerWidth - anchor.rect.left + gap)}px`);
      return;
    }

    const headerRect = header.getBoundingClientRect();
    if (headerRect.height) {
      setCssPropIfChanged(menu, "--codex-plus-menu-top", `${headerRect.top}px`);
      setCssPropIfChanged(menu, "--codex-plus-menu-height", `${headerRect.height}px`);
    }
    menu.style.removeProperty("--codex-plus-menu-right");
  }

  function codexHeaderActionsReady() {
    if (isCodexSettingsPage()) return false;
    const header = document.querySelector(selectors.appHeader);
    if (!header || !elementHasVisibleBox(header)) return false;
    const menuBar = header.querySelector(selectors.nativeMenuBar);
    if (menuBar && Array.from(menuBar.querySelectorAll("button")).some(elementHasVisibleBox)) return true;
    const contextSurface = header.querySelector(selectors.headerContextMenuSurface);
    if (contextSurface && Array.from(contextSurface.querySelectorAll("button")).some(elementHasVisibleBox)) return true;
    const toolbarButtons = Array.from(header.querySelectorAll("button"))
      .map((button) => ({ button, rect: button.getBoundingClientRect() }))
      .filter(({ button, rect }) => isHeaderToolbarButton(button, header, rect));
    return toolbarButtons.length > 0;
  }

  function installCodexPlusMenu() {
    const existing = document.getElementById(codexPlusMenuId);
    removeDuplicateCodexPlusMenus(existing);
    if (isCodexSettingsPage()) {
      removeDuplicateCodexPlusMenus(null);
      return;
    }
    let insertionPoint = findNativeMenuInsertionPoint();
    if (!insertionPoint && !codexHeaderActionsReady()) {
      removeDuplicateCodexPlusMenus(null);
      return;
    }
    if (existing && existing.dataset.codexPlusMenuVersion !== "11") {
      existing.remove();
      insertionPoint = findNativeMenuInsertionPoint();
    } else if (existing && insertionPoint && existing.parentElement === insertionPoint.parent) {
      configureCodexPlusTrigger(existing, existing.querySelector("button"), insertionPoint.nativeButtonClass, true);
      const safeBefore = insertionPoint.before?.parentElement === insertionPoint.parent ? insertionPoint.before : null;
      if (existing.nextSibling !== safeBefore) insertionPoint.parent.insertBefore(existing, safeBefore);
      removeDuplicateCodexPlusMenus(existing);
      return;
    } else if (existing && insertionPoint) {
      configureCodexPlusTrigger(existing, existing.querySelector("button"), insertionPoint.nativeButtonClass, true);
      existing.className = "";
      const safeBefore = insertionPoint.before?.parentElement === insertionPoint.parent ? insertionPoint.before : null;
      insertionPoint.parent.insertBefore(existing, safeBefore);
      removeDuplicateCodexPlusMenus(existing);
      return;
    } else if (existing) {
      configureCodexPlusTrigger(existing, existing.querySelector("button"), headerIconTextButtonClass);
      existing.className = codexPlusMenuFloatingClass;
      document.documentElement.appendChild(existing);
      updateFloatingCodexPlusMenuPosition(existing);
      removeDuplicateCodexPlusMenus(existing);
      return;
    }
    const menu = document.createElement("div");
    menu.id = codexPlusMenuId;
    menu.dataset.codexPlusMenu = "true";
    menu.dataset.codexPlusMenuVersion = "11";
    const trigger = document.createElement("button");
    trigger.type = "button";
    setCodexPlusTriggerStatus(trigger);
    setCodexPlusTriggerLabel(trigger);
    configureCodexPlusTrigger(menu, trigger, insertionPoint?.nativeButtonClass || "", !!insertionPoint);
    menu.appendChild(trigger);
    if (insertionPoint) {
      menu.className = "";
      const safeBefore = insertionPoint.before?.parentElement === insertionPoint.parent ? insertionPoint.before : null;
      insertionPoint.parent.insertBefore(menu, safeBefore);
    } else {
      menu.className = codexPlusMenuFloatingClass;
      document.documentElement.appendChild(menu);
      updateFloatingCodexPlusMenuPosition(menu);
    }
    removeDuplicateCodexPlusMenus(menu);
  }

  function nodeMatchesCodexHeaderMenu(node) {
    if (!node || node.nodeType !== 1) return false;
    const selector = `${selectors.appHeader}, ${selectors.nativeMenuBar}, #${codexPlusMenuId}, [data-codex-plus-menu="true"]`;
    return !!node.matches?.(selector) ||
      !!node.closest?.(selector) ||
      !!node.querySelector?.(selector);
  }

  function shouldRepairCodexPlusMenu(mutations) {
    if (isCodexOverlayWindow()) return false;
    if (isCodexSettingsPage()) {
      removeDuplicateCodexPlusMenus(null);
      return false;
    }
    if (!mutations) return true;
    if (!document.getElementById(codexPlusMenuId) && codexHeaderActionsReady()) return true;
    return mutations.some((mutation) => {
      if (nodeMatchesCodexHeaderMenu(mutation.target)) return true;
      return [...Array.from(mutation.addedNodes), ...Array.from(mutation.removedNodes)].some(nodeMatchesCodexHeaderMenu);
    });
  }

  function repairCodexPlusMenuNow(mutations) {
    if (!shouldRepairCodexPlusMenu(mutations)) return;
    if (window.__codexPlusMenuRepairing) return;
    window.__codexPlusMenuRepairing = true;
    try {
      installStyle();
      installCodexPlusMenu();
      renderBackendStatus();
    } finally {
      window.__codexPlusMenuRepairing = false;
    }
  }

  function runScheduledCodexPlusMenuRepair() {
    window.__codexPlusMenuRepairPending = false;
    clearTimeout(window.__codexPlusMenuRepairTimer);
    window.__codexPlusMenuRepairTimer = null;
    const mutations = window.__codexPlusMenuRepairMutations;
    window.__codexPlusMenuRepairMutations = null;
    repairCodexPlusMenuNow(mutations);
  }

  function scheduleCodexPlusMenuRepair(mutations) {
    window.__codexPlusMenuRepairMutations = mutations;
    if (window.__codexPlusMenuRepairPending) return;
    window.__codexPlusMenuRepairPending = true;
    window.__codexPlusMenuRepairTimer = setTimeout(runScheduledCodexPlusMenuRepair, 50);
  }

  function patchPluginMarketplaceRequestParams(method, params) {
    if (method === "list-plugins") {
      if (!params || typeof params !== "object") return params;
    } else {
      return params;
    }
    const next = { ...params };
    const hadMarketplaceKinds = Object.prototype.hasOwnProperty.call(next, "marketplaceKinds");
    if (hadMarketplaceKinds) delete next.marketplaceKinds;
    sendCodexPlusDiagnostic("plugin_marketplace_request_expanded", {
      hadMarketplaceKinds,
      cwdCount: Array.isArray(next.cwds) ? next.cwds.length : 0,
    });
    return next;
  }

  function displayNameForPluginMarketplaceName(name, fallback) {
    if (name === "openai-bundled") return "OpenAI插件1(Codex++)";
    if (name === "openai-curated") return "OpenAI插件2(Codex++)";
    if (name === "openai-primary-runtime") return "OpenAI插件3(Codex++)";
    return fallback;
  }

  function patchPluginMarketplaceObject(marketplace) {
    if (!marketplace || typeof marketplace !== "object" || marketplace.__codexPlusMarketplaceUnlockPatched) return false;
    const displayName = displayNameForPluginMarketplaceName(marketplace.name, marketplace.displayName || marketplace.title || marketplace.label || marketplace.name);
    if (!displayName || displayName === marketplace.name) return false;
    marketplace.displayName = displayName;
    marketplace.title = displayName;
    marketplace.label = displayName;
    if (marketplace.interface && typeof marketplace.interface === "object") {
      marketplace.interface = {
        ...marketplace.interface,
        displayName,
        name: displayName,
        title: displayName,
        label: displayName,
      };
    } else {
      marketplace.interface = { displayName, name: displayName, title: displayName, label: displayName };
    }
    marketplace.__codexPlusMarketplaceUnlockPatched = true;
    return true;
  }

  function cloneCodexPluginMarketplace(value) {
    if (!value || typeof value !== "object") return null;
    try {
      return JSON.parse(JSON.stringify(value));
    } catch {
      return null;
    }
  }

  function pluginMarketplacePluginKey(plugin) {
    if (!plugin || typeof plugin !== "object") return "";
    return String(plugin.name || plugin.id || plugin.pluginName || "").trim();
  }

  function normalizeLocalPluginMarketplacePlugin(plugin, marketplaceName) {
    const cloned = cloneCodexPluginMarketplace(plugin);
    if (!cloned || typeof cloned !== "object") return null;
    const name = String(cloned.name || cloned.id || cloned.pluginName || "").trim();
    if (!name) return null;
    if (!cloned.name) cloned.name = name;
    if (!cloned.id) cloned.id = `${name}@${marketplaceName}`;
    if (!cloned.interface || typeof cloned.interface !== "object") cloned.interface = {};
    if (!cloned.interface.displayName) cloned.interface.displayName = name;
    if (!Array.isArray(cloned.keywords)) cloned.keywords = [];
    return cloned;
  }

  function mergePluginMarketplacePlugins(target, source) {
    if (!target || !source || !Array.isArray(source.plugins)) return 0;
    if (!Array.isArray(target.plugins)) target.plugins = [];
    const marketplaceName = restorePluginMarketplaceName(target.name || source.name || "");
    const existing = new Set(target.plugins.map(pluginMarketplacePluginKey).filter(Boolean));
    let added = 0;
    source.plugins.forEach((plugin) => {
      const key = pluginMarketplacePluginKey(plugin);
      if (!key || existing.has(key)) return;
      const cloned = normalizeLocalPluginMarketplacePlugin(plugin, marketplaceName);
      if (!cloned) return;
      target.plugins.push(cloned);
      existing.add(key);
      added += 1;
    });
    return added;
  }

  function mergeLocalPluginMarketplaces(result) {
    if (!result || typeof result !== "object" || !Array.isArray(result.marketplaces)) {
      return { addedMarketplaces: 0, addedPlugins: 0 };
    }
    const localMarketplaces = Array.isArray(window.__CODEX_PLUS_PLUGIN_MARKETPLACES__)
      ? window.__CODEX_PLUS_PLUGIN_MARKETPLACES__
      : [];
    if (!localMarketplaces.length) return { addedMarketplaces: 0, addedPlugins: 0 };
    const byName = new Map();
    result.marketplaces.forEach((marketplace) => {
      const name = restorePluginMarketplaceName(marketplace?.name || "");
      if (name) byName.set(name, marketplace);
    });
    let addedMarketplaces = 0;
    let addedPlugins = 0;
    localMarketplaces.forEach((marketplace) => {
      const name = restorePluginMarketplaceName(marketplace?.name || "");
      if (!name) return;
      const existing = byName.get(name);
      if (existing) {
        addedPlugins += mergePluginMarketplacePlugins(existing, marketplace);
        return;
      }
      const cloned = cloneCodexPluginMarketplace(marketplace);
      if (!cloned) return;
      cloned.plugins = Array.isArray(cloned.plugins)
        ? cloned.plugins.map((plugin) => normalizeLocalPluginMarketplacePlugin(plugin, name)).filter(Boolean)
        : [];
      result.marketplaces.push(cloned);
      byName.set(name, cloned);
      addedMarketplaces += 1;
      addedPlugins += Array.isArray(cloned.plugins) ? cloned.plugins.length : 0;
    });
    if (addedMarketplaces > 0 || addedPlugins > 0) {
      sendCodexPlusDiagnostic("plugin_marketplace_local_merged", { addedMarketplaces, addedPlugins });
    }
    return { addedMarketplaces, addedPlugins };
  }

  function restorePluginMarketplaceName(name) {
    if (name === "codex-plus-openai-bundled") return "openai-bundled";
    if (name === "codex-plus-openai-curated") return "openai-curated";
    if (name === "codex-plus-openai-primary-runtime") return "openai-primary-runtime";
    return name;
  }

  function codexPluginOfficialMarketplaceName(name) {
    const restored = restorePluginMarketplaceName(name);
    return restored === "openai-bundled" || restored === "openai-curated" || restored === "openai-primary-runtime";
  }

  function isCodexPluginBuildFlavorFilter(callback, sample) {
    if (!Array.isArray(sample) || sample.length === 0 || typeof callback !== "function") return false;
    let source = "";
    try {
      source = Function.prototype.toString.call(callback);
    } catch {
      return false;
    }
    const isKnownFilterSource = source.includes("!u(e.marketplaceName)||e.marketplaceName===r")
      || source.includes("!ne(e.marketplaceName)||e.marketplaceName===n");
    if (!isKnownFilterSource) return false;
    if (!sample.some((plugin) => codexPluginOfficialMarketplaceName(plugin?.marketplaceName))) return false;
    return sample.some((plugin) => codexPluginOfficialMarketplaceName(plugin?.marketplaceName) && !callback(plugin));
  }

  function isCodexPluginMarketplaceHiddenFilter(callback, sample) {
    if (!Array.isArray(sample) || sample.length === 0 || typeof callback !== "function") return false;
    let source = "";
    try {
      source = Function.prototype.toString.call(callback);
    } catch {
      return false;
    }
    if (!source.includes("!t.includes(e.name)")) return false;
    if (!sample.some((marketplace) => codexPluginOfficialMarketplaceName(marketplace?.name))) return false;
    return sample.some((marketplace) => codexPluginOfficialMarketplaceName(marketplace?.name) && !callback(marketplace));
  }

  function installPluginBuildFlavorFilterPatch() {
    if (window.__codexPluginBuildFlavorFilterPatch === codexPluginMarketplaceUnlockVersion) return;
    if (pluginPatchDisabledInRelayMode()) return;
    if (!codexPlusSettings().pluginMarketplaceUnlock) return;
    const originalFilter = Array.prototype.__codexPluginBuildFlavorOriginalFilter || Array.prototype.filter;
    if (!Array.prototype.__codexPluginBuildFlavorOriginalFilter) {
      Object.defineProperty(Array.prototype, "__codexPluginBuildFlavorOriginalFilter", {
        value: originalFilter,
        configurable: true,
        writable: true,
      });
    }
    if (Array.prototype.filter.__codexPluginBuildFlavorPatched === codexPluginMarketplaceUnlockVersion) {
      window.__codexPluginBuildFlavorFilterPatch = codexPluginMarketplaceUnlockVersion;
      return;
    }
    const patchedFilter = function codexPluginBuildFlavorFilterPatch(callback, thisArg) {
      if (isCodexPluginBuildFlavorFilter(callback, this)) {
        sendCodexPlusDiagnostic("plugin_build_flavor_filter_bypassed", { pluginCount: this.length });
        return Array.from(this);
      }
      if (isCodexPluginMarketplaceHiddenFilter(callback, this)) {
        sendCodexPlusDiagnostic("plugin_marketplace_hidden_filter_bypassed", { marketplaceCount: this.length });
        return Array.from(this);
      }
      return originalFilter.call(this, callback, thisArg);
    };
    patchedFilter.__codexPluginBuildFlavorPatched = codexPluginMarketplaceUnlockVersion;
    Array.prototype.filter = patchedFilter;
    window.__codexPluginBuildFlavorFilterPatch = codexPluginMarketplaceUnlockVersion;
    sendCodexPlusDiagnostic("plugin_build_flavor_filter_patch_installed", {});
  }

  function restorePluginMarketplaceRequestParams(params, method = "") {
    if (!params || typeof params !== "object") return params;
    let next = params;
    if (Array.isArray(params.marketplaceKinds)) {
      const nextKinds = params.marketplaceKinds.map((kind) => {
        if (kind === "remote:openai-curated") return "openai-curated";
        return restorePluginMarketplaceName(kind);
      });
      next = { ...next, marketplaceKinds: Array.from(new Set(nextKinds)) };
    }
    if (method === "install-plugin") {
      next = next === params ? { ...params } : { ...next };
      if (next.remoteMarketplaceName) next.remoteMarketplaceName = restorePluginMarketplaceName(next.remoteMarketplaceName);
      if (typeof next.marketplacePath === "string" && next.marketplacePath.startsWith("remote:")) {
        const remoteMarketplaceName = next.marketplacePath.slice("remote:".length);
        delete next.marketplacePath;
        next.remoteMarketplaceName = restorePluginMarketplaceName(remoteMarketplaceName);
      }
    }
    return next;
  }

  function patchPluginMarketplaceResult(method, result) {
    if (method !== "list-plugins") return result;
    let patchedCount = 0;
    try {
      const pluginMarketplaceCounts = {};
      if (Array.isArray(result?.marketplaces)) {
        mergeLocalPluginMarketplaces(result);
        result.marketplaces.forEach((marketplace) => {
          if (Array.isArray(marketplace?.plugins)) {
            marketplace.plugins.forEach((plugin) => {
              const name = plugin?.marketplaceName || marketplace?.name || "";
              if (name) pluginMarketplaceCounts[name] = (pluginMarketplaceCounts[name] || 0) + 1;
            });
          }
          if (patchPluginMarketplaceObject(marketplace)) patchedCount += 1;
        });
        sendCodexPlusDiagnostic("plugin_marketplace_response_debug", {
          marketplaces: result.marketplaces.map((marketplace) => ({
            name: marketplace?.name || "",
            path: marketplace?.path || null,
            displayName: marketplace?.displayName || marketplace?.interface?.displayName || null,
            pluginCount: Array.isArray(marketplace?.plugins) ? marketplace.plugins.length : null,
            remoteMarketplaceName: marketplace?.remoteMarketplaceName || null,
          })),
          pluginMarketplaceCounts,
        });
      }
      if (patchedCount > 0) {
        sendCodexPlusDiagnostic("plugin_marketplace_response_expanded", { patchedCount });
      }
    } catch (error) {
      sendCodexPlusDiagnostic("plugin_marketplace_response_patch_failed", {
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
    }
    return result;
  }

  function pluginAutoExpandVisibleElement(el) {
    if (!(el instanceof HTMLElement) || !el.isConnected) return false;
    const style = getComputedStyle(el);
    if (style.display === "none" || style.visibility === "hidden" || style.pointerEvents === "none") return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }

  function pluginAutoExpandPageLooksRelevant() {
    const text = String(document.body?.innerText || "");
    return /插件|Plugins?|Marketplace|市场/i.test(text) && !!document.querySelector('button, [role="button"]');
  }

  function pluginAutoExpandButtonLooksScoped(button) {
    let node = button;
    for (let depth = 0; node instanceof HTMLElement && node !== document.body && depth < 8; depth += 1, node = node.parentElement) {
      const text = String(node.innerText || "");
      if (text.length > 16000) continue;
      if (/插件|Plugins?|Marketplace|市场/i.test(text)) return true;
    }
    return false;
  }

  function pluginAutoExpandButtonText(button) {
    return String(button?.textContent || button?.getAttribute?.("aria-label") || button?.getAttribute?.("title") || "")
      .replace(/\s+/g, " ")
      .trim();
  }

  function pluginAutoExpandButtonLooksLikeMore(button) {
    const text = pluginAutoExpandButtonText(button);
    if (!text || text.length > 120) return false;
    if (/^(更多|显示更多|查看更多|加载更多|Show more|Load more|More)$/i.test(text)) return true;
    if (/^查看\s+.+以及另外\s*\d+\s*个$/i.test(text)) return true;
    if (/^View\s+.+\s+and\s+\d+\s+more$/i.test(text)) return true;
    if (/^Show\s+.+\s+and\s+\d+\s+more$/i.test(text)) return true;
    return false;
  }

  function pluginAutoExpandButtonCandidates() {
    if (!codexPlusSettings().pluginAutoExpand || !pluginAutoExpandPageLooksRelevant()) return [];
    return Array.from(document.querySelectorAll('button, [role="button"]'))
      .filter(pluginAutoExpandVisibleElement)
      .filter((button) => !button.disabled && button.getAttribute("aria-disabled") !== "true")
      .filter(pluginAutoExpandButtonLooksLikeMore)
      .filter(pluginAutoExpandButtonLooksScoped)
      .filter((button) => !button.closest?.(`.${moreMenuClass}, #${codexPlusMenuId}, .codex-plus-modal-overlay`));
  }

  function pluginAutoExpandSignature() {
    return pluginAutoExpandButtonCandidates()
      .map((button) => {
        const rect = button.getBoundingClientRect();
        return `${pluginAutoExpandButtonText(button)}:${Math.round(rect.top)}:${Math.round(rect.left)}`;
      })
      .join("|");
  }

  function schedulePluginAutoExpand(force = false) {
    if (!codexPlusSettings().pluginAutoExpand) return;
    if (window.__codexPluginAutoExpandRunning && !force) return;
    clearTimeout(window.__codexPluginAutoExpandTimer);
    window.__codexPluginAutoExpandTimer = setTimeout(() => runPluginAutoExpand(force), force ? 30 : 180);
  }

  function runPluginAutoExpand(force = false) {
    if (!codexPlusSettings().pluginAutoExpand) return;
    const currentSignature = pluginAutoExpandSignature();
    if (!force && currentSignature && currentSignature === window.__codexPluginAutoExpandLastSignature) return;
    window.__codexPluginAutoExpandLastSignature = currentSignature;
    window.__codexPluginAutoExpandRunning = true;
    window.__codexPluginAutoExpandClicks = 0;
    const clickNext = () => {
      if (!codexPlusSettings().pluginAutoExpand) {
        window.__codexPluginAutoExpandRunning = false;
        return;
      }
      const button = pluginAutoExpandButtonCandidates()[0];
      if (!button || window.__codexPluginAutoExpandClicks >= codexPluginAutoExpandMaxClicks) {
        window.__codexPluginAutoExpandRunning = false;
        sendCodexPlusDiagnostic("plugin_auto_expand_finished", {
          version: codexPluginAutoExpandVersion,
          clicks: window.__codexPluginAutoExpandClicks || 0,
          exhausted: !!button,
        });
        return;
      }
      window.__codexPluginAutoExpandClicks = (window.__codexPluginAutoExpandClicks || 0) + 1;
      button.dataset.codexPluginAutoExpandClicked = String(Date.now());
      button.click();
      setTimeout(clickNext, codexPluginAutoExpandClickDelayMs);
    };
    clickNext();
  }

  function patchPluginMarketplaceRequestClient(client) {
    if (!client || typeof client.sendRequest !== "function") return false;
    if (client.__codexPluginMarketplaceUnlockPatch === codexPluginMarketplaceUnlockVersion) return true;
    const originalSendRequest = client.__codexPluginMarketplaceOriginalSendRequest || client.sendRequest.bind(client);
    client.__codexPluginMarketplaceOriginalSendRequest = originalSendRequest;
    client.sendRequest = async function codexPluginMarketplacePatchedSendRequest(method, params, options) {
      const requestMethod = appServerModelRequestMethod(String(method || ""), params);
      const requestParams = patchPluginMarketplaceRequestParams(requestMethod, restorePluginMarketplaceRequestParams(params, requestMethod));
      if (requestMethod === "install-plugin") {
        sendCodexPlusDiagnostic("plugin_install_request_debug", {
          method: String(method || ""),
          requestMethod,
          originalMarketplacePath: params?.marketplacePath || null,
          originalRemoteMarketplaceName: params?.remoteMarketplaceName || null,
          originalPluginName: params?.pluginName || null,
          requestMarketplacePath: requestParams?.marketplacePath || null,
          requestRemoteMarketplaceName: requestParams?.remoteMarketplaceName || null,
          requestPluginName: requestParams?.pluginName || null,
        });
      }
      try {
        const result = await originalSendRequest(method, requestParams, options);
        return patchPluginMarketplaceResult(requestMethod, result);
      } catch (error) {
        if (requestMethod === "install-plugin") {
          sendCodexPlusDiagnostic("plugin_install_request_failed", {
            method: String(method || ""),
            requestMethod,
            requestMarketplacePath: requestParams?.marketplacePath || null,
            requestRemoteMarketplaceName: requestParams?.remoteMarketplaceName || null,
            requestPluginName: requestParams?.pluginName || null,
            errorName: error?.name || "",
            errorMessage: error?.message || String(error),
          });
        }
        throw error;
      }
    };
    client.__codexPluginMarketplaceUnlockPatch = codexPluginMarketplaceUnlockVersion;
    return true;
  }

  function patchPluginMarketplaceRequestMessage(message) {
    if (!message || typeof message !== "object") return message;
    if (message.type === "fetch" && typeof message.url === "string") {
      const requestMethod = appServerModelRequestMethod(message.url, message.body);
      if (requestMethod !== "list-plugins" && requestMethod !== "install-plugin") return message;
      let requestBody = message.body;
      let params = null;
      if (typeof requestBody === "string" && requestBody.trim()) {
        try {
          params = JSON.parse(requestBody);
        } catch {
          params = null;
        }
      } else if (requestBody && typeof requestBody === "object") {
        params = requestBody;
      }
      const requestParams = patchPluginMarketplaceRequestParams(
        requestMethod,
        restorePluginMarketplaceRequestParams(params, requestMethod)
      );
      if (requestMethod === "list-plugins" && message.requestId != null) {
        window.__codexPluginMarketplaceFetchRequestIds = window.__codexPluginMarketplaceFetchRequestIds || new Set();
        window.__codexPluginMarketplaceFetchRequestIds.add(String(message.requestId));
      }
      if (requestParams === params) return message;
      if (requestMethod === "install-plugin") {
        sendCodexPlusDiagnostic("plugin_install_request_debug", {
          method: message.url,
          requestMethod,
          originalMarketplacePath: params?.marketplacePath || null,
          originalRemoteMarketplaceName: params?.remoteMarketplaceName || null,
          originalPluginName: params?.pluginName || null,
          requestMarketplacePath: requestParams?.marketplacePath || null,
          requestRemoteMarketplaceName: requestParams?.remoteMarketplaceName || null,
          requestPluginName: requestParams?.pluginName || null,
        });
      }
      return {
        ...message,
        body: typeof requestBody === "string" ? JSON.stringify(requestParams) : requestParams,
      };
    }
    if (message.type === "mcp-request" && message.request && typeof message.request === "object") {
      const requestMethod = appServerModelRequestMethod(String(message.request.method || ""), message.request.params);
      if (requestMethod !== "list-plugins" && requestMethod !== "install-plugin") return message;
      const requestParams = patchPluginMarketplaceRequestParams(
        requestMethod,
        restorePluginMarketplaceRequestParams(message.request.params, requestMethod)
      );
      if (requestMethod === "list-plugins" && message.request.id != null) {
        window.__codexPluginMarketplaceRequestIds = window.__codexPluginMarketplaceRequestIds || new Set();
        window.__codexPluginMarketplaceRequestIds.add(String(message.request.id));
      }
      if (requestParams === message.request.params) return message;
      if (requestMethod === "install-plugin") {
        sendCodexPlusDiagnostic("plugin_install_request_debug", {
          method: String(message.request.method || ""),
          requestMethod,
          originalMarketplacePath: message.request.params?.marketplacePath || null,
          originalRemoteMarketplaceName: message.request.params?.remoteMarketplaceName || null,
          originalPluginName: message.request.params?.pluginName || null,
          requestMarketplacePath: requestParams?.marketplacePath || null,
          requestRemoteMarketplaceName: requestParams?.remoteMarketplaceName || null,
          requestPluginName: requestParams?.pluginName || null,
        });
      }
      return { ...message, request: { ...message.request, params: requestParams } };
    }
    return message;
  }

  function patchPluginMarketplaceResponseData(data) {
    if (data?.type === "fetch-response") {
      const requestId = data.requestId != null ? String(data.requestId) : "";
      const requestIds = window.__codexPluginMarketplaceFetchRequestIds;
      if (requestIds instanceof Set && requestIds.size > 0) {
        if (!requestIds.has(requestId)) return false;
        requestIds.delete(requestId);
      }
      if (typeof data.bodyJsonString !== "string" || !data.bodyJsonString.trim()) return false;
      try {
        const result = JSON.parse(data.bodyJsonString);
        if (result && typeof result === "object") {
          patchPluginMarketplaceResult("list-plugins", result);
          patchPluginMarketplaceResult("list-plugins", result.data);
        }
        data.bodyJsonString = JSON.stringify(result);
        return true;
      } catch (error) {
        sendCodexPlusDiagnostic("plugin_marketplace_fetch_response_patch_failed", {
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
      }
      return false;
    }
    if (data?.type !== "mcp-response") return false;
    const message = data.message || data.response;
    const method = String(message?.method || data.method || "");
    if (appServerModelRequestMethod(method) === "install-plugin") {
      clearPluginMarketplaceQueryCache();
    }
    const requestId = message?.id != null ? String(message.id) : "";
    const requestIds = window.__codexPluginMarketplaceRequestIds;
    if (requestIds instanceof Set && requestIds.size > 0) {
      if (!requestIds.has(requestId)) return false;
      requestIds.delete(requestId);
    }
    const result = message?.result;
    if (!result || typeof result !== "object") return false;
    patchPluginMarketplaceResult("list-plugins", result);
    patchPluginMarketplaceResult("list-plugins", result.data);
    return true;
  }

  function clearPluginMarketplaceQueryCache() {
    try {
      const queryClient = window.__REACT_QUERY_CLIENT__ || window.__codexQueryClient;
      if (queryClient && typeof queryClient.invalidateQueries === "function") {
        queryClient.invalidateQueries({ queryKey: ["plugins"] });
      }
    } catch {
    }
  }

  function installPluginMarketplaceBridgePatch() {
    if (window.__codexPluginMarketplaceBridgePatch === codexPluginMarketplaceUnlockVersion) return;
    if (pluginPatchDisabledInRelayMode()) return;
    if (!codexPlusSettings().pluginMarketplaceUnlock) return;
    installPluginMarketplaceWindowEventPatchOnly();
    const bridge = window.electronBridge;
    if (!bridge || typeof bridge.sendMessageFromView !== "function") {
      sendCodexPlusDiagnostic("plugin_marketplace_bridge_patch_not_found", {});
      return;
    }
    if (!bridge.__codexPluginMarketplaceOriginalSendMessageFromView) {
      bridge.__codexPluginMarketplaceOriginalSendMessageFromView = bridge.sendMessageFromView.bind(bridge);
      bridge.sendMessageFromView = function codexPluginMarketplacePatchedSendMessageFromView(message) {
        let nextMessage = message;
        try {
          nextMessage = patchPluginMarketplaceRequestMessage(message);
        } catch (error) {
          sendCodexPlusDiagnostic("plugin_marketplace_bridge_request_patch_failed", {
            errorName: error?.name || "",
            errorMessage: error?.message || String(error),
          });
        }
        return bridge.__codexPluginMarketplaceOriginalSendMessageFromView(nextMessage);
      };
    }
    bridge.__codexPluginMarketplaceBridgePatch = codexPluginMarketplaceUnlockVersion;
    window.__codexPluginMarketplaceBridgePatch = codexPluginMarketplaceUnlockVersion;
    sendCodexPlusDiagnostic("plugin_marketplace_bridge_patch_installed", {});
  }

  function installPluginMarketplaceWindowEventPatchOnly() {
    if (window.__codexPluginMarketplaceWindowEventPatch === codexPluginMarketplaceUnlockVersion) return;
    if (pluginPatchDisabledInRelayMode()) return;
    if (!codexPlusSettings().pluginMarketplaceUnlock) return;
    const originalDispatchEvent = window.__codexPluginMarketplaceOriginalDispatchEvent || window.dispatchEvent;
    if (!window.__codexPluginMarketplaceOriginalDispatchEvent) {
      window.__codexPluginMarketplaceOriginalDispatchEvent = originalDispatchEvent;
      window.dispatchEvent = function patchedCodexPluginMarketplaceDispatchEvent(event) {
        try {
          const detail = event?.detail;
          if (event?.type === "codex-message-from-view" && detail?.type === "mcp-request") {
            const patched = patchPluginMarketplaceRequestMessage(detail);
            if (patched !== detail) {
              Object.keys(detail).forEach((key) => delete detail[key]);
              Object.assign(detail, patched);
            }
          }
          if (event?.type === "message") patchPluginMarketplaceResponseData(event.data);
        } catch (error) {
          sendCodexPlusDiagnostic("plugin_marketplace_dispatch_event_patch_failed", {
            errorName: error?.name || "",
            errorMessage: error?.message || String(error),
          });
        }
        return originalDispatchEvent.call(this, event);
      };
    }
    if (!window.__codexPluginMarketplaceResponseListenerInstalled) {
      window.__codexPluginMarketplaceResponseListenerInstalled = true;
      window.addEventListener("message", (event) => {
        try {
          patchPluginMarketplaceResponseData(event?.data);
        } catch (error) {
          sendCodexPlusDiagnostic("plugin_marketplace_response_message_patch_failed", {
            errorName: error?.name || "",
            errorMessage: error?.message || String(error),
          });
        }
      }, true);
    }
    window.__codexPluginMarketplaceWindowEventPatch = codexPluginMarketplaceUnlockVersion;
  }

  function installPluginMarketplaceRequestPatch() {
    if (window.__codexPluginMarketplaceUnlockInstalled === codexPluginMarketplaceUnlockVersion) return;
    if (pluginPatchDisabledInRelayMode()) return;
    if (!codexPlusSettings().pluginMarketplaceUnlock) return;
    const patch = async () => {
      try {
        const module = await loadCodexAppModule("app-server-manager-signals-");
        const candidates = Object.values(module).filter((value) => value && typeof value === "object");
        let patchedCount = 0;
        for (const candidate of candidates) {
          if (patchPluginMarketplaceRequestClient(candidate)) patchedCount += 1;
          if (typeof candidate.sendRequest !== "function" && typeof candidate.get === "function") {
            try {
              if (patchPluginMarketplaceRequestClient(candidate.get())) patchedCount += 1;
            } catch {
            }
          }
        }
        if (patchedCount > 0) {
          window.__codexPluginMarketplaceUnlockInstalled = codexPluginMarketplaceUnlockVersion;
          sendCodexPlusDiagnostic("plugin_marketplace_request_patch_installed", {
            candidateCount: candidates.length,
            patchedCount,
          });
        } else {
          sendCodexPlusDiagnostic("plugin_marketplace_request_patch_not_found", {
            exportCount: Object.keys(module || {}).length,
            candidateCount: candidates.length,
          });
        }
      } catch (error) {
        sendCodexPlusDiagnostic("plugin_marketplace_request_patch_failed", {
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
      }
    };
    void patch();
  }

  function pluginPatchDisabledInRelayMode() {
    return !codexPlusBackendSettingsLoaded || codexPlusBackendSettings.launchMode === "relay";
  }

  function pluginInstallCandidates() {
    const nodes = Array.from(document.querySelectorAll(selectors.disabledInstallButton));
    return Array.from(new Set(nodes.map((node) => node.closest?.("button, [role='button']") || node)));
  }

  function installButtonLabel(element) {
    return (element.textContent || "").trim();
  }

  function isInstallButtonLabel(text) {
    return /^安装\s*/.test(text) || /^Install\s*/i.test(text) || text === "强制安装";
  }

  function patchReactDisabledProps(element) {
    Object.keys(element)
      .filter((key) => key.startsWith("__reactProps"))
      .forEach((key) => {
        const props = element[key];
        if (!props || typeof props !== "object") return;
        props.disabled = false;
        props["aria-disabled"] = false;
        props["data-disabled"] = undefined;
      });
  }

  function clearDisabledState(element) {
    if (!(element instanceof HTMLElement)) return;
    if ("disabled" in element) element.disabled = false;
    element.removeAttribute("disabled");
    element.removeAttribute("aria-disabled");
    element.removeAttribute("data-disabled");
    element.removeAttribute("inert");
    element.classList.remove("disabled", "opacity-50", "cursor-not-allowed", "pointer-events-none");
    element.classList.add("codex-force-install-unlocked");
    element.style.pointerEvents = "auto";
    element.style.opacity = "";
    element.style.cursor = "pointer";
    element.tabIndex = 0;
    patchReactDisabledProps(element);
  }

  function installButtonUnlockNodes(button) {
    const nodes = [button];
    button.querySelectorAll?.("button, [role='button'], [disabled], [aria-disabled], [data-disabled], .cursor-not-allowed, .pointer-events-none")
      .forEach((node) => nodes.push(node));
    let parent = button.parentElement;
    for (let depth = 0; parent && depth < 3; depth += 1, parent = parent.parentElement) {
      if (parent.matches?.("button, [role='button'], [disabled], [aria-disabled], [data-disabled], .cursor-not-allowed, .pointer-events-none")) {
        nodes.push(parent);
      }
    }
    return Array.from(new Set(nodes));
  }

  function installForcedInstallGuard(button) {
    if (button.dataset.codexForceInstallUnlocked === "true") return;
    button.dataset.codexForceInstallUnlocked = "true";
    const keepUnlocked = () => installButtonUnlockNodes(button).forEach(clearDisabledState);
    ["pointerdown", "mousedown", "mouseup", "click", "focus"].forEach((eventName) => {
      button.addEventListener(eventName, keepUnlocked, true);
    });
  }

  function unblockButtonElement(button) {
    installButtonUnlockNodes(button).forEach(clearDisabledState);
    installForcedInstallGuard(button);
  }

  function labelForcedInstallButton(button) {
    const walker = document.createTreeWalker(button, NodeFilter.SHOW_TEXT);
    let textNode = null;
    while (!textNode && walker.nextNode()) {
      const node = walker.currentNode;
      if (isInstallButtonLabel((node.nodeValue || "").trim())) textNode = node;
    }
    if (textNode) {
      textNode.nodeValue = "强制安装";
    }
  }

  function clearForcedInstallButtonLabel(button) {
    const walker = document.createTreeWalker(button, NodeFilter.SHOW_TEXT);
    let textNode = null;
    while (!textNode && walker.nextNode()) {
      const node = walker.currentNode;
      if ((node.nodeValue || "").trim() === "强制安装") textNode = node;
    }
    if (textNode) {
      textNode.nodeValue = "安装";
    }
  }

  function clearPluginPatchArtifacts() {
    pluginInstallCandidates().forEach(clearForcedInstallButtonLabel);
  }

  function unblockPluginInstallButtons() {
    if (pluginPatchDisabledInRelayMode()) return;
    if (!codexPlusSettings().forcePluginInstall) return;
    pluginInstallCandidates().forEach((button) => {
      const text = installButtonLabel(button);
      if (!isInstallButtonLabel(text)) return;
      unblockButtonElement(button);
      labelForcedInstallButton(button);
    });
  }

  function refreshForcePluginInstallUnlockLoop() {
    const shouldRun = !pluginPatchDisabledInRelayMode() && codexPlusSettings().forcePluginInstall;
    if (!shouldRun) {
      clearInterval(window.__codexForcePluginInstallRefreshTimer);
      window.__codexForcePluginInstallRefreshTimer = null;
      return;
    }
    if (window.__codexForcePluginInstallRefreshTimer) return;
    window.__codexForcePluginInstallRefreshTimer = setInterval(() => {
      if (!codexPlusSettings().forcePluginInstall || pluginPatchDisabledInRelayMode()) {
        clearInterval(window.__codexForcePluginInstallRefreshTimer);
        window.__codexForcePluginInstallRefreshTimer = null;
        return;
      }
      unblockPluginInstallButtons();
    }, codexForcePluginInstallRefreshIntervalMs);
  }

  let cachedSessionRows = [];
  let cachedSessionRowsAt = 0;
  let threadIdBadgeActive = false;

  function sessionRows(forceRefresh = false) {
    const now = Date.now();
    if (!forceRefresh && now - cachedSessionRowsAt < 150) {
      cachedSessionRows = cachedSessionRows.filter((row) => row.isConnected);
      if (cachedSessionRows.length > 0) return cachedSessionRows;
    }

    cachedSessionRows = Array.from(document.querySelectorAll(selectors.sidebarThread));
    cachedSessionRowsAt = now;
    return cachedSessionRows;
  }

  function archivePageHintVisible() {
    if (window.location.href.includes("archive")) return true;
    if (document.querySelector('[data-codex-archive-page-row="true"], [data-codex-archive-delete-all]')) return true;
    const archiveNav = document.querySelector(selectors.archiveNav);
    if (archiveNav?.className?.includes?.("bg-token-list-hover-background")) return true;
    return !!Array.from(document.querySelectorAll("h1, h2, h3")).find((element) => (element.textContent || "").trim() === "已归档对话");
  }

  function archiveRowFromUnarchiveButton(button) {
    return button.closest('[data-codex-archive-page-row="true"]')
      || button.closest('[role="listitem"], [role="row"]')
      || button.closest(".flex.w-full.items-center.justify-between")
      || button.parentElement;
  }

  function archivedPageRows() {
    if (!archivePageHintVisible()) return [];
    const rows = Array.from(document.querySelectorAll("button")).filter((button) => (button.textContent || "").trim() === "取消归档").map(archiveRowFromUnarchiveButton).filter(Boolean);
    rows.forEach((row) => {
      row.dataset.codexArchivePageRow = "true";
      row.setAttribute("data-codex-archive-page-row", "true");
    });
    return rows;
  }

  function archivedSessionRows() {
    if (!archivePageHintVisible()) return [];
    return sessionRows().filter((row) => row.querySelector('button[aria-label="取消归档对话"]') || row.outerHTML.includes("取消归档") || row.outerHTML.includes("unarchive"));
  }

  function archivedRows() {
    if (!archivePageHintVisible()) return [];
    return [...archivedSessionRows(), ...archivedPageRows()];
  }

  function archivedPageVisible() {
    return archivePageHintVisible() && archivedRows().length > 0;
  }

  function sessionRefFromRow(row) {
    const href = row.getAttribute("href") || row.querySelector("a")?.getAttribute("href") || "";
    const idMatch = href.match(/(?:session|conversation|thread)[=/:-]([A-Za-z0-9_.-]+)/i) || href.match(/([A-Za-z0-9_-]{8,})$/);
    const codexThreadId = row.getAttribute("data-app-action-sidebar-thread-id") || "";
    const fallbackId = row.getAttribute("data-session-id") || row.getAttribute("data-testid") || "";
    const sessionId = codexThreadId || (idMatch && idMatch[1]) || fallbackId;
    const titleNode = row.querySelector(`${selectors.threadTitle}, .truncate.select-none, .truncate.text-base`);
    const rawTitle = (titleNode?.textContent || (titleNode ? "" : (row.textContent || "Untitled session")));
    const title = (titleNode ? rawTitle : rawTitle.replace(/\s*(导出|删除|移动|移出项目|Export|Delete|Move|Remove from project)(\s*(导出|删除|移动|移出项目|Export|Delete|Move|Remove from project))*$/gi, "")).trim().slice(0, 160);
    return { session_id: sessionId, title };
  }

  function threadIdBadgeTitleNode(row) {
    return row.querySelector(`${selectors.threadTitle}, .truncate.select-none, .truncate.text-base`);
  }

  function padThreadIdBadgePart(value) {
    return String(value).padStart(2, "0");
  }

  function threadIdBadgeCreatedAt(sessionId) {
    const timestampMs = uuidV7TimestampMs(sessionId);
    const minReasonableMs = Date.UTC(2020, 0, 1);
    const maxReasonableMs = Date.now() + 366 * 24 * 60 * 60 * 1000;
    if (!timestampMs || timestampMs < minReasonableMs || timestampMs > maxReasonableMs) return null;
    return new Date(timestampMs);
  }

  function formatThreadIdBadgeCreatedAt(date) {
    if (!(date instanceof Date) || Number.isNaN(date.getTime())) return "";
    return `${padThreadIdBadgePart(date.getMonth() + 1)}-${padThreadIdBadgePart(date.getDate())} ${padThreadIdBadgePart(date.getHours())}:${padThreadIdBadgePart(date.getMinutes())}`;
  }

  function threadIdBadgeMeta(sessionId) {
    const id = projectMoveSessionKey(sessionId);
    const compact = id.replaceAll("-", "");
    const shortId = compact.slice(0, 8);
    const createdAt = threadIdBadgeCreatedAt(sessionId);
    const createdLabel = formatThreadIdBadgeCreatedAt(createdAt);
    return {
      id,
      shortId,
      createdAt,
      label: shortId ? `[${shortId}${createdLabel ? ` ${createdLabel}` : ""}]` : "",
    };
  }

  function wrapThreadTitleForBadge(row, titleNode) {
    const parent = titleNode?.parentElement;
    if (!parent) return null;
    if (parent.dataset?.codexThreadIdBadgeWrap === "true") return parent;
    const wrapper = document.createElement("span");
    wrapper.dataset.codexThreadIdBadgeWrap = "true";
    parent.insertBefore(wrapper, titleNode);
    wrapper.appendChild(titleNode);
    return wrapper;
  }

  function removeThreadIdBadges(root = document) {
    root.querySelectorAll?.(`.${threadIdBadgeClass}`).forEach((badge) => badge.remove());
    root.querySelectorAll?.('[data-codex-thread-id-badge-wrap="true"]').forEach((wrapper) => {
      const parent = wrapper.parentElement;
      if (!parent) return;
      while (wrapper.firstChild) parent.insertBefore(wrapper.firstChild, wrapper);
      wrapper.remove();
    });
    const rows = root.matches?.(selectors.sidebarThread) ? [root] : Array.from(root.querySelectorAll?.(selectors.sidebarThread) || []);
    rows.forEach((row) => {
      delete row.dataset.codexThreadIdBadge;
      delete row.dataset.codexThreadIdBadgeVersion;
    });
  }

  function installThreadIdBadge(row) {
    const ref = sessionRefFromRow(row);
    if (!ref.session_id) {
      removeThreadIdBadges(row);
      return;
    }
    const meta = threadIdBadgeMeta(ref.session_id);
    const titleNode = threadIdBadgeTitleNode(row);
    if (!meta.label || !titleNode) {
      removeThreadIdBadges(row);
      return;
    }

    const wrapper = wrapThreadTitleForBadge(row, titleNode);
    if (!wrapper) return;

    let badge = wrapper.querySelector(`.${threadIdBadgeClass}`);
    if (!badge) {
      badge = document.createElement("span");
      badge.className = threadIdBadgeClass;
      wrapper.insertBefore(badge, titleNode);
    }

    badge.dataset.codexThreadIdBadgeVersion = codexThreadIdBadgeVersion;
    if (badge.textContent !== meta.label) badge.textContent = meta.label;
    const fullTitle = meta.createdAt
      ? `${meta.label}\nSession ID: ${meta.id}\nCreated: ${meta.createdAt.toLocaleString()}`
      : `${meta.label}\nSession ID: ${meta.id}`;
    badge.setAttribute("title", fullTitle);
    badge.setAttribute("aria-label", fullTitle);
    row.dataset.codexThreadIdBadge = meta.label;
    row.dataset.codexThreadIdBadgeVersion = codexThreadIdBadgeVersion;
  }

  function refreshThreadIdBadges() {
    if (!codexPlusSettings().threadIdBadge) {
      if (threadIdBadgeActive) {
        removeThreadIdBadges();
        threadIdBadgeActive = false;
      }
      return;
    }
    threadIdBadgeActive = true;
    sessionRows().forEach(installThreadIdBadge);
  }

  function codexPlusDiagnosticPayload(event, detail) {
    return {
      event,
      detail: detail || {},
      helperBase,
      hasBridge: !!window.__codexSessionDeleteBridge,
      location: window.location?.href || "",
      userAgent: navigator.userAgent || "",
      timestamp: new Date().toISOString(),
    };
  }

  function sendCodexPlusDiagnostic(event, detail) {
    const payload = codexPlusDiagnosticPayload(event, detail);
    if (window.__CODEX_PLUS_TEST_SERVICE_TIER__) {
      window.__codexPlusServiceTierTestDiagnostics = window.__codexPlusServiceTierTestDiagnostics || [];
      window.__codexPlusServiceTierTestDiagnostics.push(payload);
      return;
    }
    if (window.__codexSessionDeleteBridge) {
      window.__codexSessionDeleteBridge("/diagnostics/log", payload).catch(() => {});
    }
    const body = JSON.stringify(payload);
    try {
      if (navigator.sendBeacon) {
        const blob = new Blob([body], { type: "application/json" });
        if (navigator.sendBeacon(`${helperBase}/diagnostics/log`, blob)) return;
      }
    } catch (_) {}
    fetch(`${helperBase}/diagnostics/log`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body,
      keepalive: true,
    }).catch(() => {});
  }

  sendCodexPlusDiagnostic("script_loaded", {
    version: codexPlusVersion,
    build: codexPlusBuild,
  });

  function locationThreadId() {
    const source = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    const match = source.match(/(?:session|conversation|thread)(?:\/|=|:|-)([A-Za-z0-9_.-]+)/i)
      || source.match(/\/([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})(?:[/?#]|$)/)
      || source.match(/\/([A-Za-z0-9_-]{24,})(?:[/?#]|$)/);
    return match ? decodeURIComponent(match[1]) : "";
  }

  function finiteNonNegativeNumber(value) {
    const numeric = Number(value);
    return Number.isFinite(numeric) && numeric >= 0 ? numeric : 0;
  }

  function finiteScrollNumber(value) {
    const numeric = Number(value);
    return Number.isFinite(numeric) ? numeric : 0;
  }

  function validThreadScrollSessionKey(sessionId) {
    const key = projectMoveSessionKey(sessionId);
    if (!key || key === "__proto__" || key === "prototype" || key === "constructor") return "";
    return /^[A-Za-z0-9_.-]{8,128}$/.test(key) ? key : "";
  }

  function currentSessionRef() {
    const rows = sessionRows();
    for (const row of rows) {
      const ref = sessionRefFromRow(row);
      if (ref.session_id && isCurrentSessionRow(row, ref)) return ref;
    }
    return { session_id: locationThreadId(), title: "" };
  }

  function readThreadScrollEntries() {
    if (window.__codexThreadScrollEntries && typeof window.__codexThreadScrollEntries === "object") {
      return { ...window.__codexThreadScrollEntries };
    }
    try {
      const parsed = JSON.parse(localStorage.getItem(codexThreadScrollKey) || "{}");
      const rawEntries = parsed?.version === codexThreadScrollVersion && parsed?.entries && typeof parsed.entries === "object"
        ? parsed.entries
        : parsed && typeof parsed === "object"
          ? parsed
          : {};
      const entries = Object.create(null);
      Object.entries(rawEntries).forEach(([key, value]) => {
        const safeKey = validThreadScrollSessionKey(key);
        if (!safeKey || !value || typeof value !== "object") return;
        entries[safeKey] = {
          top: finiteScrollNumber(value.top),
          scrollHeight: finiteNonNegativeNumber(value.scrollHeight),
          clientHeight: finiteNonNegativeNumber(value.clientHeight),
          at: finiteNonNegativeNumber(value.at),
        };
      });
      window.__codexThreadScrollEntries = entries;
      return { ...entries };
    } catch {
      window.__codexThreadScrollEntries = Object.create(null);
      return {};
    }
  }

  function writeThreadScrollEntries(entries) {
    const pruned = Object.create(null);
    Object.entries(entries || {})
      .sort((left, right) => finiteNonNegativeNumber(right[1]?.at) - finiteNonNegativeNumber(left[1]?.at))
      .slice(0, codexThreadScrollMaxEntries)
      .forEach(([key, value]) => {
        const safeKey = validThreadScrollSessionKey(key);
        if (safeKey) pruned[safeKey] = value;
      });
    window.__codexThreadScrollEntries = pruned;
    localStorage.setItem(codexThreadScrollKey, JSON.stringify({ version: codexThreadScrollVersion, entries: pruned }));
  }

  function currentThreadScroller() {
    const explicit = document.querySelector(".thread-scroll-container");
    if (explicit?.isConnected) return explicit;
    const root = conversationRoot();
    if (!root?.isConnected) return document.scrollingElement || document.documentElement;
    const style = getComputedStyle(root);
    if (/(auto|scroll)/.test(style.overflowY) && root.scrollHeight > root.clientHeight) return root;
    return nearestScrollableAncestor(root);
  }

  function threadScrollRuntime() {
    if (!window.__codexThreadScrollRuntime || typeof window.__codexThreadScrollRuntime !== "object") {
      window.__codexThreadScrollRuntime = {
        activeSessionId: "",
        activeScroller: null,
        scrollListener: null,
        scrollListenerUsesWindow: false,
        lastSavedTop: -1,
        lastSavedHeight: -1,
        lastSavedClientHeight: -1,
        restoreLock: null,
        applyingRestore: false,
        pendingNavigation: null,
        userScrollIntentUntil: 0,
        userCancelledRestoreSessionId: "",
      };
    }
    return window.__codexThreadScrollRuntime;
  }

  function clearThreadScrollRestoreTimers() {
    (window.__codexThreadScrollRestoreTimers || []).forEach((timer) => clearTimeout(timer));
    window.__codexThreadScrollRestoreTimers = [];
  }

  function clearThreadScrollSyncTimers() {
    (window.__codexThreadScrollSyncTimers || []).forEach((timer) => clearTimeout(timer));
    window.__codexThreadScrollSyncTimers = [];
  }

  function clearThreadScrollRestoreLock() {
    threadScrollRuntime().restoreLock = null;
  }

  function cancelThreadScrollRestoreForUserIntent() {
    const runtime = threadScrollRuntime();
    const cancelledSessionId = validThreadScrollSessionKey(runtime.restoreLock?.sessionId)
      || validThreadScrollSessionKey(currentSessionRef().session_id)
      || validThreadScrollSessionKey(runtime.activeSessionId);
    runtime.userScrollIntentUntil = Date.now() + codexThreadScrollUserIntentWindowMs;
    runtime.userCancelledRestoreSessionId = cancelledSessionId;
    window.__codexThreadScrollRestoreRevision = (window.__codexThreadScrollRestoreRevision || 0) + 1;
    window.__codexThreadScrollSyncRevision = (window.__codexThreadScrollSyncRevision || 0) + 1;
    clearThreadScrollRestoreTimers();
    clearThreadScrollSyncTimers();
    clearThreadScrollRestoreLock();
  }

  function userScrollIntentActive() {
    return finiteNonNegativeNumber(threadScrollRuntime().userScrollIntentUntil) > Date.now();
  }

  function threadScrollRestoreCancelledForSession(sessionId = threadScrollRuntime().activeSessionId) {
    const key = validThreadScrollSessionKey(sessionId);
    return !!key && threadScrollRuntime().userCancelledRestoreSessionId === key;
  }

  function activeThreadScrollRestoreLock(sessionId = threadScrollRuntime().activeSessionId) {
    const runtime = threadScrollRuntime();
    const key = validThreadScrollSessionKey(sessionId);
    const lock = runtime.restoreLock;
    if (!lock || !key || lock.sessionId !== key) return null;
    if (lock.expiresAt <= Date.now()) {
      clearThreadScrollRestoreLock();
      return null;
    }
    return lock;
  }

  function currentThreadScrollRestoreLock() {
    const sessionId = threadScrollRuntime().restoreLock?.sessionId;
    return sessionId ? activeThreadScrollRestoreLock(sessionId) : null;
  }

  function threadScrollIsReversed(scroller) {
    return getComputedStyle(scroller).flexDirection === "column-reverse";
  }

  function threadScrollRange(scroller) {
    const extent = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
    return threadScrollIsReversed(scroller)
      ? { min: -extent, max: 0, bottom: 0 }
      : { min: 0, max: extent, bottom: extent };
  }

  function startThreadScrollRestoreLock(sessionId, entry) {
    const key = validThreadScrollSessionKey(sessionId);
    if (!key || !entry) {
      clearThreadScrollRestoreLock();
      return null;
    }
    const runtime = threadScrollRuntime();
    runtime.restoreLock = {
      sessionId: key,
      targetTop: finiteScrollNumber(entry.top),
      expiresAt: Date.now() + codexThreadScrollRestoreWindowMs,
    };
    return runtime.restoreLock;
  }

  function prepareThreadScrollRestoreLock(sessionId) {
    const key = validThreadScrollSessionKey(sessionId);
    const entry = key ? readThreadScrollEntries()[key] : null;
    if (entry) startThreadScrollRestoreLock(key, entry);
  }

  function threadScrollTargetTop(scroller, targetTop) {
    const range = threadScrollRange(scroller);
    return Math.max(range.min, Math.min(range.max, finiteScrollNumber(targetTop)));
  }

  function threadScrollNearBottom(scroller, top) {
    const range = threadScrollRange(scroller);
    return Math.abs(range.bottom - finiteScrollNumber(top)) <= Math.max(24, scroller.clientHeight * 0.15);
  }

  function threadScrollGuardScroller(scroller) {
    if (!scroller) return null;
    const runtime = threadScrollRuntime();
    const rootScroller = document.scrollingElement || document.documentElement || document.body;
    const normalizedScroller = scroller === document.body || scroller === document.documentElement ? rootScroller : scroller;
    if (normalizedScroller === runtime.activeScroller) return normalizedScroller;
    const currentScroller = currentThreadScroller();
    if (normalizedScroller === currentScroller) return normalizedScroller;
    return null;
  }

  function shouldBlockThreadScrollAutobottom(scroller, top) {
    const runtime = threadScrollRuntime();
    const lock = currentThreadScrollRestoreLock();
    if (!lock || !codexPlusSettings().threadScrollRestore) return false;
    const guardScroller = threadScrollGuardScroller(scroller);
    if (runtime.applyingRestore || !guardScroller) return false;
    const targetTop = threadScrollTargetTop(guardScroller, lock.targetTop);
    return Math.abs(finiteScrollNumber(top) - targetTop) > 8 && threadScrollNearBottom(guardScroller, top);
  }

  function scrollToRequestedTop(args, scroller) {
    if (!args.length) return null;
    const first = args[0];
    if (typeof first === "object" && first !== null) return first.top == null ? null : finiteScrollNumber(first.top);
    if (args.length >= 2) return finiteScrollNumber(args[1]);
    return scroller?.scrollTop ?? null;
  }

  function scrollByRequestedTop(args, scroller) {
    if (!args.length || !scroller) return null;
    const first = args[0];
    let delta = null;
    if (typeof first === "object" && first !== null) {
      delta = first.top == null ? null : Number(first.top);
    } else if (args.length >= 2) {
      delta = Number(args[1]);
    }
    return Number.isFinite(delta) ? finiteScrollNumber(scroller.scrollTop + delta) : null;
  }

  function shouldBlockThreadScrollIntoView(element) {
    const runtime = threadScrollRuntime();
    const lock = currentThreadScrollRestoreLock();
    if (runtime.applyingRestore || !lock || !element) return false;
    const activeScroller = threadScrollGuardScroller(runtime.activeScroller) || threadScrollGuardScroller(currentThreadScroller());
    if (!activeScroller || element === activeScroller || !activeScroller.contains?.(element)) return false;
    if (threadScrollIsReversed(activeScroller) && shouldBlockThreadScrollAutobottom(activeScroller, 0)) return true;
    const elementRect = element.getBoundingClientRect?.();
    if (!elementRect) return false;
    const elementBottomTop = activeScroller.scrollTop + elementRect.bottom - scrollerViewportTop(activeScroller) - activeScroller.clientHeight;
    return shouldBlockThreadScrollAutobottom(activeScroller, elementBottomTop);
  }

  function installThreadScrollProgrammaticScrollGuard() {
    if (window.__codexThreadScrollProgrammaticGuardInstalled === codexThreadScrollProgrammaticGuardVersion) return;
    window.__codexThreadScrollProgrammaticGuardInstalled = codexThreadScrollProgrammaticGuardVersion;
    window.__codexThreadScrollOriginals = window.__codexThreadScrollOriginals || {};
    const originals = window.__codexThreadScrollOriginals;
    originals.elementScrollTo = originals.elementScrollTo || Element.prototype.scrollTo;
    if (typeof originals.elementScrollTo === "function") {
      Element.prototype.scrollTo = function codexThreadScrollGuardedScrollTo(...args) {
        const top = scrollToRequestedTop(args, this);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(this, top)) return;
        return originals.elementScrollTo.apply(this, args);
      };
    }
    originals.elementScroll = originals.elementScroll || Element.prototype.scroll;
    if (typeof originals.elementScroll === "function") {
      Element.prototype.scroll = function codexThreadScrollGuardedScroll(...args) {
        const top = scrollToRequestedTop(args, this);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(this, top)) return;
        return originals.elementScroll.apply(this, args);
      };
    }
    originals.elementScrollBy = originals.elementScrollBy || Element.prototype.scrollBy;
    if (typeof originals.elementScrollBy === "function") {
      Element.prototype.scrollBy = function codexThreadScrollGuardedScrollBy(...args) {
        const top = scrollByRequestedTop(args, this);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(this, top)) return;
        return originals.elementScrollBy.apply(this, args);
      };
    }
    originals.scrollIntoView = originals.scrollIntoView || Element.prototype.scrollIntoView;
    if (typeof originals.scrollIntoView === "function") {
      Element.prototype.scrollIntoView = function codexThreadScrollGuardedScrollIntoView(...args) {
        if (window.__codexThreadScrollHandlers?.shouldBlockIntoView?.(this)) return;
        return originals.scrollIntoView.apply(this, args);
      };
    }
    originals.windowScrollTo = originals.windowScrollTo || window.scrollTo;
    if (typeof originals.windowScrollTo === "function") {
      window.scrollTo = function codexThreadScrollGuardedWindowScrollTo(...args) {
        const scroller = document.scrollingElement || document.documentElement || document.body;
        const top = scrollToRequestedTop(args, scroller);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(scroller, top)) return;
        return originals.windowScrollTo.apply(this, args);
      };
    }
    originals.windowScroll = originals.windowScroll || window.scroll;
    if (typeof originals.windowScroll === "function") {
      window.scroll = function codexThreadScrollGuardedWindowScroll(...args) {
        const scroller = document.scrollingElement || document.documentElement || document.body;
        const top = scrollToRequestedTop(args, scroller);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(scroller, top)) return;
        return originals.windowScroll.apply(this, args);
      };
    }
    originals.windowScrollBy = originals.windowScrollBy || window.scrollBy;
    if (typeof originals.windowScrollBy === "function") {
      window.scrollBy = function codexThreadScrollGuardedWindowScrollBy(...args) {
        const scroller = document.scrollingElement || document.documentElement || document.body;
        const top = scrollByRequestedTop(args, scroller);
        if (top != null && window.__codexThreadScrollHandlers?.shouldBlockAutobottom?.(scroller, top)) return;
        return originals.windowScrollBy.apply(this, args);
      };
    }
  }

  function bindThreadScrollListener(scroller) {
    const runtime = threadScrollRuntime();
    const currentUsesWindow = !runtime.activeScroller || runtime.activeScroller === document.scrollingElement || runtime.activeScroller === document.documentElement || runtime.activeScroller === document.body;
    const nextUsesWindow = !scroller || scroller === document.scrollingElement || scroller === document.documentElement || scroller === document.body;
    let listenerReplaced = false;
    if (runtime.scrollListener && runtime.scrollListenerVersion !== codexThreadScrollListenerVersion) {
      const currentTarget = currentUsesWindow ? window : runtime.activeScroller;
      currentTarget?.removeEventListener?.("scroll", runtime.scrollListener, true);
      runtime.scrollListener = null;
      runtime.scrollListenerVersion = "";
      listenerReplaced = true;
    }
    runtime.scrollListener = runtime.scrollListener || (() => scheduleThreadScrollSave());
    runtime.scrollListenerVersion = codexThreadScrollListenerVersion;
    if (!listenerReplaced && runtime.activeScroller === scroller && runtime.scrollListenerUsesWindow === nextUsesWindow) return;
    if (runtime.activeScroller) {
      const target = currentUsesWindow ? window : runtime.activeScroller;
      target.removeEventListener("scroll", runtime.scrollListener, true);
    }
    runtime.activeScroller = scroller;
    runtime.scrollListenerUsesWindow = nextUsesWindow;
    if (!scroller || !codexPlusSettings().threadScrollRestore) return;
    const target = nextUsesWindow ? window : scroller;
    target.addEventListener("scroll", runtime.scrollListener, true);
  }

  function saveThreadScrollPositionNow(sessionId = threadScrollRuntime().activeSessionId, scroller = threadScrollRuntime().activeScroller) {
    if (!codexPlusSettings().threadScrollRestore) return;
    const runtime = threadScrollRuntime();
    const key = validThreadScrollSessionKey(sessionId);
    if (!key || !scroller) return;
    if (activeThreadScrollRestoreLock(key)) return;
    const snapshot = {
      top: finiteScrollNumber(scroller.scrollTop),
      scrollHeight: finiteNonNegativeNumber(scroller.scrollHeight),
      clientHeight: finiteNonNegativeNumber(scroller.clientHeight),
      at: Date.now(),
    };
    if (Math.abs(runtime.lastSavedTop - snapshot.top) < 2 && runtime.lastSavedHeight === snapshot.scrollHeight && runtime.lastSavedClientHeight === snapshot.clientHeight) return;
    const entries = readThreadScrollEntries();
    entries[key] = snapshot;
    writeThreadScrollEntries(entries);
    runtime.lastSavedTop = snapshot.top;
    runtime.lastSavedHeight = snapshot.scrollHeight;
    runtime.lastSavedClientHeight = snapshot.clientHeight;
  }

  function scheduleThreadScrollSave() {
    if (!codexPlusSettings().threadScrollRestore || window.__codexThreadScrollSaveTimer) return;
    window.__codexThreadScrollSaveTimer = setTimeout(() => {
      window.__codexThreadScrollSaveTimer = null;
      saveThreadScrollPositionNow();
    }, codexThreadScrollSaveThrottleMs);
  }

  function restoreThreadScrollPosition(sessionId) {
    const runtime = threadScrollRuntime();
    const key = validThreadScrollSessionKey(sessionId);
    if (!codexPlusSettings().threadScrollRestore || !key || runtime.activeSessionId !== key || userScrollIntentActive() || threadScrollRestoreCancelledForSession(key)) return;
    const lock = activeThreadScrollRestoreLock(key);
    const entry = lock || readThreadScrollEntries()[key];
    if (!entry) return;
    const scroller = currentThreadScroller();
    if (!scroller) return;
    bindThreadScrollListener(scroller);
    const targetTop = threadScrollTargetTop(scroller, lock ? lock.targetTop : entry.top);
    if (Math.abs(scroller.scrollTop - targetTop) <= 1) return;
    runtime.applyingRestore = true;
    try {
      if (typeof scroller.scrollTo === "function") {
        scroller.scrollTo({ top: targetTop, behavior: "auto" });
      } else {
        scroller.scrollTop = targetTop;
      }
    } finally {
      runtime.applyingRestore = false;
    }
    runtime.lastSavedTop = targetTop;
    runtime.lastSavedHeight = finiteNonNegativeNumber(scroller.scrollHeight);
    runtime.lastSavedClientHeight = finiteNonNegativeNumber(scroller.clientHeight);
  }

  function scheduleThreadScrollRestore(sessionId) {
    clearThreadScrollRestoreTimers();
    const key = validThreadScrollSessionKey(sessionId);
    if (!codexPlusSettings().threadScrollRestore || !key || userScrollIntentActive() || threadScrollRestoreCancelledForSession(key)) return;
    const entry = readThreadScrollEntries()[key];
    if (!entry) {
      clearThreadScrollRestoreLock();
      return;
    }
    startThreadScrollRestoreLock(key, entry);
    const restoreRevision = (window.__codexThreadScrollRestoreRevision || 0) + 1;
    window.__codexThreadScrollRestoreRevision = restoreRevision;
    window.__codexThreadScrollRestoreTimers = codexThreadScrollRestoreDelaysMs.map((delay) => setTimeout(() => {
      if (window.__codexThreadScrollRestoreRevision !== restoreRevision) return;
      restoreThreadScrollPosition(key);
    }, delay));
  }

  function syncThreadScrollState(forceRestore = false) {
    const runtime = threadScrollRuntime();
    const currentRef = currentSessionRef();
    const nextSessionId = validThreadScrollSessionKey(currentRef.session_id);
    if (!nextSessionId) return;
    if (!codexPlusSettings().threadScrollRestore) {
      bindThreadScrollListener(null);
      clearThreadScrollRestoreTimers();
      clearThreadScrollRestoreLock();
      runtime.activeSessionId = nextSessionId;
      return;
    }
    if (runtime.activeSessionId !== nextSessionId) prepareThreadScrollRestoreLock(nextSessionId);
    const nextScroller = currentThreadScroller();
    bindThreadScrollListener(nextScroller);
    if (runtime.activeSessionId !== nextSessionId) {
      runtime.lastSavedTop = -1;
      runtime.lastSavedHeight = -1;
      runtime.lastSavedClientHeight = -1;
      clearThreadScrollRestoreLock();
      runtime.activeSessionId = nextSessionId;
      runtime.pendingNavigation = null;
      runtime.userScrollIntentUntil = 0;
      if (runtime.userCancelledRestoreSessionId !== nextSessionId) runtime.userCancelledRestoreSessionId = "";
      scheduleThreadScrollRestore(nextSessionId);
      return;
    }
    runtime.activeSessionId = nextSessionId;
    if (forceRestore && !userScrollIntentActive() && !threadScrollRestoreCancelledForSession(nextSessionId)) scheduleThreadScrollRestore(nextSessionId);
  }

  function scheduleThreadScrollSyncAttempts(forceRestore = true) {
    const currentKey = validThreadScrollSessionKey(currentSessionRef().session_id) || validThreadScrollSessionKey(threadScrollRuntime().activeSessionId);
    if (userScrollIntentActive() || threadScrollRestoreCancelledForSession(currentKey)) return;
    clearThreadScrollSyncTimers();
    const syncRevision = (window.__codexThreadScrollSyncRevision || 0) + 1;
    window.__codexThreadScrollSyncRevision = syncRevision;
    window.__codexThreadScrollSyncTimers = codexThreadScrollRestoreDelaysMs.map((delay) => setTimeout(() => {
      if (window.__codexThreadScrollSyncRevision !== syncRevision) return;
      scheduleThreadScrollSync(forceRestore);
    }, delay));
  }

  function captureThreadScrollNavigation(targetSessionId) {
    if (!codexPlusSettings().threadScrollRestore) return;
    const runtime = threadScrollRuntime();
    const targetKey = validThreadScrollSessionKey(targetSessionId);
    const sessionChanged = !!targetKey && targetKey !== runtime.activeSessionId;
    if (sessionChanged) {
      runtime.userScrollIntentUntil = 0;
      runtime.userCancelledRestoreSessionId = "";
    }
    const pending = runtime.pendingNavigation;
    const duplicatePendingTarget = !!targetKey && pending?.targetSessionId === targetKey && Date.now() - finiteNonNegativeNumber(pending.at) < 5000;
    if (!duplicatePendingTarget) saveThreadScrollPositionNow();
    if (targetKey) {
      runtime.pendingNavigation = { fromSessionId: runtime.activeSessionId, targetSessionId: targetKey, at: Date.now() };
      prepareThreadScrollRestoreLock(targetKey);
    }
    scheduleThreadScrollSyncAttempts(true);
  }

  function editableThreadScrollTarget(element) {
    return !!element?.closest?.("input, textarea, select, [contenteditable='true'], [contenteditable='']");
  }

  function eventTargetsActiveThreadScroller(event) {
    const runtime = threadScrollRuntime();
    const scroller = threadScrollGuardScroller(runtime.activeScroller) || threadScrollGuardScroller(currentThreadScroller());
    if (!scroller) return false;
    const target = event?.target;
    if (!target || target === document || target === window) return true;
    return target === scroller || scroller.contains?.(target) || scroller.contains?.(document.activeElement);
  }

  function markThreadScrollUserIntent(event) {
    if (!codexPlusSettings().threadScrollRestore || !eventTargetsActiveThreadScroller(event)) return;
    cancelThreadScrollRestoreForUserIntent();
  }

  function markThreadScrollKeyboardIntent(event) {
    if (editableThreadScrollTarget(event.target)) return;
    if (!["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " ", "Spacebar"].includes(event.key)) return;
    markThreadScrollUserIntent(event);
  }

  function markThreadScrollPointerIntent(event) {
    const scroller = threadScrollGuardScroller(threadScrollRuntime().activeScroller) || threadScrollGuardScroller(currentThreadScroller());
    if (event.target === scroller) markThreadScrollUserIntent(event);
  }

  function updateThreadScrollHandlers() {
    window.__codexThreadScrollHandlers = {
      shouldBlockAutobottom: shouldBlockThreadScrollAutobottom,
      shouldBlockIntoView: shouldBlockThreadScrollIntoView,
      markUserIntent: markThreadScrollUserIntent,
      markKeyboardIntent: markThreadScrollKeyboardIntent,
      markPointerIntent: markThreadScrollPointerIntent,
      captureNavigation: captureThreadScrollNavigation,
      saveNow: saveThreadScrollPositionNow,
      prepareRestoreLock: prepareThreadScrollRestoreLock,
      scheduleSyncAttempts: scheduleThreadScrollSyncAttempts,
    };
  }

  function installThreadScrollUserIntentCapture() {
    if (window.__codexThreadScrollUserIntentInstalled === codexThreadScrollUserIntentVersion) return;
    document.removeEventListener("wheel", window.__codexThreadScrollWheelIntentHandler, true);
    document.removeEventListener("touchmove", window.__codexThreadScrollTouchIntentHandler, true);
    document.removeEventListener("keydown", window.__codexThreadScrollKeyIntentHandler, true);
    document.removeEventListener("pointerdown", window.__codexThreadScrollPointerIntentHandler, true);
    window.__codexThreadScrollWheelIntentHandler = (event) => window.__codexThreadScrollHandlers?.markUserIntent?.(event);
    window.__codexThreadScrollTouchIntentHandler = (event) => window.__codexThreadScrollHandlers?.markUserIntent?.(event);
    window.__codexThreadScrollKeyIntentHandler = (event) => window.__codexThreadScrollHandlers?.markKeyboardIntent?.(event);
    window.__codexThreadScrollPointerIntentHandler = (event) => window.__codexThreadScrollHandlers?.markPointerIntent?.(event);
    document.addEventListener("wheel", window.__codexThreadScrollWheelIntentHandler, { capture: true, passive: true });
    document.addEventListener("touchmove", window.__codexThreadScrollTouchIntentHandler, { capture: true, passive: true });
    document.addEventListener("keydown", window.__codexThreadScrollKeyIntentHandler, true);
    document.addEventListener("pointerdown", window.__codexThreadScrollPointerIntentHandler, true);
    window.__codexThreadScrollUserIntentInstalled = codexThreadScrollUserIntentVersion;
  }

  function installThreadScrollNavigationCapture() {
    document.removeEventListener("pointerdown", window.__codexThreadScrollNavigationHandler, true);
    document.removeEventListener("click", window.__codexThreadScrollClickNavigationHandler, true);
    document.removeEventListener("keydown", window.__codexThreadScrollKeyboardHandler, true);
    const navigationHandler = (event) => {
      if (!codexPlusSettings().threadScrollRestore) return;
      const row = event.target?.closest?.(selectors.sidebarThread);
      if (!row) return;
      window.__codexThreadScrollHandlers?.captureNavigation?.(sessionRefFromRow(row).session_id);
    };
    const clickHandler = (event) => {
      if (!codexPlusSettings().threadScrollRestore) return;
      const row = event.target?.closest?.(selectors.sidebarThread);
      if (!row) return;
      window.__codexThreadScrollHandlers?.captureNavigation?.(sessionRefFromRow(row).session_id);
    };
    const keyboardHandler = (event) => {
      if (!codexPlusSettings().threadScrollRestore) return;
      if (event.key !== "Enter" && event.key !== " ") return;
      const row = event.target?.closest?.(selectors.sidebarThread);
      if (!row) return;
      window.__codexThreadScrollHandlers?.captureNavigation?.(sessionRefFromRow(row).session_id);
    };
    window.__codexThreadScrollNavigationHandler = navigationHandler;
    window.__codexThreadScrollClickNavigationHandler = clickHandler;
    window.__codexThreadScrollKeyboardHandler = keyboardHandler;
    document.addEventListener("pointerdown", navigationHandler, true);
    document.addEventListener("click", clickHandler, true);
    document.addEventListener("keydown", keyboardHandler, true);
  }

  function scheduleThreadScrollSync(forceRestore = false) {
    if (window.__codexThreadScrollSyncPending) return;
    window.__codexThreadScrollSyncPending = true;
    setTimeout(() => {
      window.__codexThreadScrollSyncPending = false;
      syncThreadScrollState(forceRestore);
    }, 0);
  }

  function installThreadScrollRouteHooks() {
    if (window.__codexThreadScrollRouteHooksInstalled === codexThreadScrollRouteHooksVersion) return;
    window.__codexThreadScrollRouteHooksInstalled = codexThreadScrollRouteHooksVersion;
    window.__codexThreadScrollOriginals = window.__codexThreadScrollOriginals || {};
    const originals = window.__codexThreadScrollOriginals;
    ["pushState", "replaceState"].forEach((method) => {
      const currentMethod = history[method];
      const original = originals[`history_${method}`] || currentMethod;
      originals[`history_${method}`] = original;
      if (typeof original !== "function") return;
      history[method] = function codexThreadScrollPatchedHistory(...args) {
        window.__codexThreadScrollHandlers?.saveNow?.();
        const result = original.apply(this, args);
        window.__codexThreadScrollHandlers?.captureNavigation?.(locationThreadId());
        return result;
      };
    });
    window.removeEventListener("popstate", window.__codexThreadScrollPopStateHandler, true);
    window.removeEventListener("hashchange", window.__codexThreadScrollHashChangeHandler, true);
    document.removeEventListener("visibilitychange", window.__codexThreadScrollVisibilityHandler, true);
    window.__codexThreadScrollPopStateHandler = () => {
      window.__codexThreadScrollHandlers?.saveNow?.();
      window.__codexThreadScrollHandlers?.captureNavigation?.(locationThreadId());
    };
    window.__codexThreadScrollHashChangeHandler = () => {
      window.__codexThreadScrollHandlers?.saveNow?.();
      window.__codexThreadScrollHandlers?.captureNavigation?.(locationThreadId());
    };
    window.__codexThreadScrollVisibilityHandler = () => {
      if (document.visibilityState === "hidden") window.__codexThreadScrollHandlers?.saveNow?.();
    };
    window.addEventListener("popstate", window.__codexThreadScrollPopStateHandler, true);
    window.addEventListener("hashchange", window.__codexThreadScrollHashChangeHandler, true);
    document.addEventListener("visibilitychange", window.__codexThreadScrollVisibilityHandler, true);
  }

  async function postJson(path, payload) {
    if (!window.__codexSessionDeleteBridge) {
      if (path === "/backend/status" || path === "/backend/repair") {
        try {
          const response = await fetch(`${helperBase}${path}`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(payload || {}),
          });
          return await response.json();
        } catch (error) {
          return { status: "failed", message: t("backendDisconnected") };
        }
      }
      sendCodexPlusDiagnostic("bridge_missing_for_route", { path });
      return { status: "failed", message: t("bridgeUnavailable") };
    }
    function bridgeWithBackendTimeout(path, payload) {
      return Promise.race([
        window.__codexSessionDeleteBridge(path, payload),
        new Promise((resolve) => setTimeout(() => resolve({ status: "failed", message: t("backendTimeout"), timeout: true }), 2000)),
      ]);
    }
    async function fetchBackendStatusFromHelper(path, payload) {
      try {
        const response = await fetch(`${helperBase}${path}`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload || {}),
        });
        return await response.json();
      } catch (error) {
        return { status: "failed", message: t("backendDisconnected") };
      }
    }
    try {
      if (path === "/backend/status" || path === "/backend/repair") {
        const result = await bridgeWithBackendTimeout(path, payload);
        if (result?.status === "ok") return result;
        if (result?.timeout) sendCodexPlusDiagnostic("backend_bridge_timeout", { path });
        const fallback = await fetchBackendStatusFromHelper(path, payload);
        if (fallback?.status === "ok") {
          sendCodexPlusDiagnostic("backend_status_bridge_failed_http_fallback_ok", {
            path,
            httpStatus: 200,
            responseStatus: fallback.status || "",
          });
          return fallback;
        }
        sendCodexPlusDiagnostic("backend_status_bridge_and_http_failed", {
          path,
          errorName: "",
          errorMessage: "",
        });
        return fallback;
      }
      return await window.__codexSessionDeleteBridge(path, payload);
    } catch (error) {
      sendCodexPlusDiagnostic("bridge_call_failed", {
        path,
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
      if (path === "/backend/status" || path === "/backend/repair") {
        const fallback = await fetchBackendStatusFromHelper(path, payload);
        if (fallback?.status === "ok") {
          sendCodexPlusDiagnostic("backend_status_bridge_failed_http_fallback_ok", {
            path,
            httpStatus: 200,
            responseStatus: fallback.status || "",
          });
          return fallback;
        }
        sendCodexPlusDiagnostic("backend_status_bridge_and_http_failed", {
          path,
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
        return fallback;
      }
      throw error;
    }
  }

  function downloadMarkdownFallback(filename, markdown) {
    if (!filename || typeof markdown !== "string") {
      throw new Error("导出结果不完整");
    }
    const blob = new Blob([markdown], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = filename;
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  async function saveMarkdown(filename, markdown) {
    if (!filename || typeof markdown !== "string") {
      throw new Error("导出结果不完整");
    }
    if (typeof window.showSaveFilePicker !== "function") {
      downloadMarkdownFallback(filename, markdown);
      return { status: "saved" };
    }
    try {
      const handle = await window.showSaveFilePicker({
        suggestedName: filename,
        types: [{
          description: "Markdown",
          accept: { "text/markdown": [".md", ".markdown"] },
        }],
      });
      const writable = await handle.createWritable();
      await writable.write(markdown);
      await writable.close();
      return { status: "saved" };
    } catch (error) {
      if (error?.name === "AbortError") {
        return { status: "cancelled", message: "导出已取消" };
      }
      throw error;
    }
  }

  let codexStateApiPromise = null;
  let chatsSortInFlight = false;
  let chatsSortSignature = "";
  let chatsSortLastFetchAt = 0;

  async function codexStateApi() {
    codexStateApiPromise = codexStateApiPromise || import("./assets/vscode-api-Dc9pX2Bc.js");
    const api = await codexStateApiPromise;
    if (typeof api.n !== "function") throw new Error("Codex 状态 API 不可用");
    return api.n;
  }

  async function codexStateCall(method, params) {
    const call = await codexStateApi();
    return await call(method, params);
  }

  async function getCodexGlobalState(key) {
    const result = await codexStateCall("get-global-state", { params: { key } });
    return result && Object.prototype.hasOwnProperty.call(result, "value") ? result.value : result;
  }

  async function setCodexGlobalState(key, value) {
    return await codexStateCall("set-global-state", { params: { key, value } });
  }

  function objectGlobalState(value) {
    return value && typeof value === "object" && !Array.isArray(value) ? { ...value } : {};
  }

  function uniqueValues(values) {
    return Array.from(new Set(values.filter((value) => typeof value === "string" && value.trim().length > 0)));
  }

  let codexModelCatalog = { status: "loading", model: "", default_model: "", model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
  let codexModelCatalogLoadedAt = 0;
  let codexModelCatalogPromise = null;
  let codexModelWhitelistRefreshTimer = 0;
  let codexModelWhitelistRefreshUntil = 0;
  const codexPlusModelListRequestIds = new Set();

  if (window.__CODEX_PLUS_TEST_SERVICE_TIER__) {
    window.__codexPlusServiceTierTest = {
      applyServiceTierOverride: (method, params, threadIdHint = "") => applyCodexServiceTierRequestOverride(method, params, threadIdHint),
      requestOverride: (message) => codexServiceTierRequestOverride(message),
      diagnostics: () => [...(window.__codexPlusServiceTierTestDiagnostics || [])],
      setModelCatalog: (catalog = {}) => {
        codexModelCatalog = {
          status: "ok",
          model: "",
          default_model: "",
          model_provider: "",
          provider_name: "",
          models: [],
          sources: [],
          responses_api: { status: "unknown", message: "" },
          ...catalog,
        };
        codexModelCatalogLoadedAt = Date.now();
        codexModelCatalogPromise = null;
      },
      setServiceTierState: (state = {}) => {
        codexServiceTierState = { ...codexServiceTierState, ...state };
      },
      setThreadState: (state = {}) => {
        localStorage.setItem(codexThreadServiceTierKey, JSON.stringify({
          version: codexThreadServiceTierVersion,
          mode: "inherit",
          defaultMode: "inherit",
          entries: {},
          ...state,
        }));
      },
    };
    return;
  }

  function codexPlusModelUnlockEnabled() {
    return !!codexPlusSettings().modelWhitelistUnlock;
  }

  function codexPlusModelNames() {
    return uniqueValues([
      codexModelCatalog.default_model,
      codexModelCatalog.model,
      ...(Array.isArray(codexModelCatalog.models) ? codexModelCatalog.models : []),
    ]);
  }

  async function loadCodexModelCatalog(force = false) {
    if (!force && codexModelCatalogPromise) return codexModelCatalogPromise;
    if (!force && codexModelCatalogLoadedAt && Date.now() - codexModelCatalogLoadedAt < 10000) return codexModelCatalog;
    codexModelCatalogPromise = postJson("/codex-model-catalog", {})
      .then((result) => {
        codexModelCatalog = result && typeof result === "object" ? result : { status: "failed", model: "", default_model: "", model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
        codexModelCatalogLoadedAt = Date.now();
        renderCodexPlusMenu();
        scheduleCodexModelWhitelistRefresh();
        return codexModelCatalog;
      })
      .catch((error) => {
        codexModelCatalog = { status: "failed", message: String(error?.message || error), model: "", default_model: "", model_provider: "", provider_name: "", models: [], sources: [], responses_api: { status: "unknown", message: "" } };
        codexModelCatalogLoadedAt = Date.now();
        return codexModelCatalog;
      })
      .finally(() => {
        codexModelCatalogPromise = null;
      });
    return codexModelCatalogPromise;
  }

  function modelReasoningEfforts() {
    return ["minimal", "low", "medium", "high", "xhigh"].map((reasoningEffort) => ({ reasoningEffort, description: `${reasoningEffort} effort` }));
  }

  function codexPlusModelDescriptor(modelName) {
    return {
      model: modelName,
      id: modelName,
      slug: modelName,
      name: modelName,
      displayName: modelName,
      description: codexModelCatalog.provider_name || codexModelCatalog.model_provider || "Custom model",
      hidden: false,
      isDefault: (codexModelCatalog.default_model || codexModelCatalog.model) === modelName,
      defaultReasoningEffort: "medium",
      supportedReasoningEfforts: modelReasoningEfforts(),
    };
  }

  function modelArrayLooksPatchable(value, allowEmpty = false) {
    return Array.isArray(value)
      && (allowEmpty || value.length > 0)
      && value.every((item) => item && typeof item === "object" && typeof item.model === "string");
  }

  function stringArrayLooksPatchable(value) {
    return Array.isArray(value) && value.every((item) => typeof item === "string");
  }

  function patchModelNameArray(models) {
    if (!stringArrayLooksPatchable(models)) return false;
    const customModels = codexPlusModelNames();
    if (!customModels.length) return false;
    let changed = false;
    customModels.forEach((modelName) => {
      if (!models.includes(modelName)) {
        models.push(modelName);
        changed = true;
      }
    });
    return changed;
  }

  function patchModelArray(models, allowEmpty = false) {
    if (!modelArrayLooksPatchable(models, allowEmpty)) return false;
    const customModels = codexPlusModelNames();
    if (!customModels.length) return false;
    let changed = false;
    const existing = new Map(models.map((item) => [item.model, item]));
    models.forEach((item) => {
      if (customModels.includes(item.model) && item.hidden !== false) {
        item.hidden = false;
        changed = true;
      }
    });
    customModels.forEach((modelName) => {
      if (!existing.has(modelName)) {
        models.push(codexPlusModelDescriptor(modelName));
        changed = true;
      }
    });
    return changed;
  }

  function patchModelContainer(value) {
    if (!value || typeof value !== "object") return false;
    let changed = false;
    if (patchModelArray(value.models, "defaultModel" in value || "availableModels" in value)) changed = true;
    if (patchModelNameArray(value.models)) changed = true;
    if (patchModelArray(value.data)) changed = true;
    if (patchModelArray(value.result)) changed = true;
    if (patchModelArray(value.pages?.[0]?.data)) changed = true;
    if (patchModelArray(value.result?.data)) changed = true;
    if (patchModelArray(value.result?.models)) changed = true;
    if (patchModelArray(value.message?.result?.data)) changed = true;
    if (patchModelArray(value.message?.result?.models)) changed = true;
    const names = codexPlusModelNames();
    if (value.availableModels instanceof Set) {
      names.forEach((name) => {
        if (!value.availableModels.has(name)) {
          value.availableModels.add(name);
          changed = true;
        }
      });
    }
    if (value.available_models instanceof Set) {
      names.forEach((name) => {
        if (!value.available_models.has(name)) {
          value.available_models.add(name);
          changed = true;
        }
      });
    }
    if (Array.isArray(value.availableModels)) {
      names.forEach((name) => {
        if (!value.availableModels.includes(name)) {
          value.availableModels.push(name);
          changed = true;
        }
      });
    }
    if (Array.isArray(value.available_models)) {
      names.forEach((name) => {
        if (!value.available_models.includes(name)) {
          value.available_models.push(name);
          changed = true;
        }
      });
    }
    if (Array.isArray(value.hiddenModels)) {
      const before = value.hiddenModels.length;
      value.hiddenModels = value.hiddenModels.filter((name) => !names.includes(name));
      if (value.hiddenModels.length !== before) changed = true;
    }
    if (Array.isArray(value.hidden_models)) {
      const before = value.hidden_models.length;
      value.hidden_models = value.hidden_models.filter((name) => !names.includes(name));
      if (value.hidden_models.length !== before) changed = true;
    }
    if (value.defaultModel == null && names.length > 0) {
      value.defaultModel = codexPlusModelDescriptor(names[0]);
      changed = true;
    } else if (typeof value.defaultModel === "string" && names.includes(value.defaultModel) && value.model == null) {
      value.model = value.defaultModel;
      changed = true;
    }
    return changed;
  }

  async function patchModelJsonResponse(payload) {
    if (!codexPlusModelUnlockEnabled()) return payload;
    if (!codexPlusModelNames().length) await loadCodexModelCatalog();
    if (!payload || typeof payload !== "object") return payload;
    try {
      patchModelContainer(payload);
      patchObjectGraphForModels(payload, new WeakSet(), 0);
    } catch (error) {
      window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
      window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
    }
    return payload;
  }

  function installModelJsonResponsePatch() {
    if (window.__codexPlusModelJsonResponsePatchInstalled === "1") return;
    window.__codexPlusModelJsonResponsePatchInstalled = "1";
    window.__codexPlusModelJsonResponseOriginals = window.__codexPlusModelJsonResponseOriginals || {};
    const originals = window.__codexPlusModelJsonResponseOriginals;
    originals.responseJson = originals.responseJson || Response.prototype.json;
    if (typeof originals.responseJson !== "function") return;
    Response.prototype.json = async function codexPlusPatchedResponseJson(...args) {
      const payload = await originals.responseJson.apply(this, args);
      return await patchModelJsonResponse(payload);
    };
  }

  function patchStatsigModelDynamicConfig(config) {
    const names = codexPlusModelNames();
    const value = config?.value;
    if (!names.length || !value || typeof value !== "object") return config;
    const availableModels = Array.isArray(value.available_models) ? [...value.available_models] : [];
    let changed = false;
    names.forEach((name) => {
      if (!availableModels.includes(name)) {
        availableModels.push(name);
        changed = true;
      }
    });
    const nextValue = {
      ...value,
      available_models: availableModels,
      default_model: names[0] || value.default_model,
    };
    if (!changed && nextValue.default_model === value.default_model) return config;
    try {
      config.value = nextValue;
    } catch {
      return { ...config, value: nextValue };
    }
    return config;
  }

  function statsigClients() {
    const root = window.__STATSIG__ || globalThis.__STATSIG__;
    if (!root || typeof root !== "object") return [];
    const clients = [root.firstInstance, typeof root.instance === "function" ? root.instance() : null];
    if (root.instances && typeof root.instances === "object") clients.push(...Object.values(root.instances));
    return clients.filter((client, index, array) => client && typeof client === "object" && array.indexOf(client) === index);
  }

  function patchStatsigModelWhitelist() {
    statsigClients().forEach((client) => {
      if (typeof client.getDynamicConfig !== "function") return;
      if (!client.__codexPlusModelWhitelistPatched) {
        const originalGetDynamicConfig = client.getDynamicConfig.bind(client);
        client.getDynamicConfig = (name, options) => {
          const result = originalGetDynamicConfig(name, options);
          return patchStatsigModelDynamicConfig(result);
        };
        client.__codexPlusModelWhitelistPatched = true;
      }
      try {
        patchStatsigModelDynamicConfig(client.getDynamicConfig("107580212", { disableExposureLog: true }));
      } catch {
      }
    });
  }

  function patchObjectGraphForModels(root, visited, depth = 0) {
    if (!root || typeof root !== "object" || visited.has(root) || depth > 5) return false;
    visited.add(root);
    let changed = patchModelContainer(root);
    if (root instanceof Element || root === window || root === document || root === document.body || root === document.documentElement) return changed;
    for (const key of Object.keys(root)) {
      if (key === "ownerDocument" || key === "parentElement" || key === "parentNode" || key === "children" || key === "childNodes") continue;
      let value;
      try {
        value = root[key];
      } catch {
        continue;
      }
      if (value && typeof value === "object" && patchObjectGraphForModels(value, visited, depth + 1)) changed = true;
    }
    return changed;
  }

  function reactFiberKeys(element) {
    return Object.keys(element).filter((key) => key.startsWith("__reactFiber") || key.startsWith("__reactInternalInstance") || key.startsWith("__reactProps"));
  }

  function isWorkspaceChromeNode(node) {
    if (!node || node.nodeType !== 1) return false;
    if (node.closest?.('[data-app-action-sidebar-section-heading="Chats"], [data-app-action-sidebar-section-heading="Projects"], [data-app-action-sidebar-thread-id], [data-app-action-sidebar-project-row], [data-app-action-sidebar-project-id]')) {
      return false;
    }
    return !!node.closest?.("main aside");
  }

  function patchReactModelStateNodes() {
    const selector = "[role='menu'], [role='dialog'], [role='listbox'], [data-radix-popper-content-wrapper]";
    return [document.body, ...document.querySelectorAll(selector)].filter((node) => node && !isWorkspaceChromeNode(node));
  }

  function shouldScheduleReactModelStatePatch(mutations) {
    if (!codexPlusModelUnlockEnabled() || !codexPlusModelNames().length) return false;
    if (!mutations) return false;
    const selector = "[role='menu'], [role='dialog'], [role='listbox'], [data-radix-popper-content-wrapper]";
    return mutations.some((mutation) => [...mutation.addedNodes].some((node) => {
      if (node.nodeType !== 1 || isWorkspaceChromeNode(node)) return false;
      return !!node.matches?.(selector) || !!node.querySelector?.(selector);
    }));
  }

  function schedulePatchReactModelState() {
    if (window.__codexPlusReactModelStatePatchPending) return;
    window.__codexPlusReactModelStatePatchPending = true;
    clearTimeout(window.__codexPlusReactModelStatePatchTimer);
    window.__codexPlusReactModelStatePatchTimer = setTimeout(() => {
      window.__codexPlusReactModelStatePatchPending = false;
      window.__codexPlusReactModelStatePatchTimer = null;
      patchReactModelState();
    }, 120);
  }

  function patchReactModelState() {
    const visited = new WeakSet();
    const nodes = patchReactModelStateNodes();
    let changed = false;
    for (const node of nodes.slice(0, 220)) {
      for (const key of reactFiberKeys(node)) {
        if (patchObjectGraphForModels(node[key], visited)) changed = true;
      }
    }
    return changed;
  }

  function patchAppServerModelMessages() {
    if (window.__codexPlusModelMessagePatchInstalled) return;
    window.__codexPlusModelMessagePatchInstalled = true;
    const originalDispatchEvent = window.dispatchEvent;
    window.dispatchEvent = function patchedCodexPlusDispatchEvent(event) {
      try {
        const detail = event?.detail;
        const request = detail?.request;
        if (event?.type === "codex-message-from-view" && detail?.type === "mcp-request" && request?.method === "model/list") {
          request.params = { ...(request.params || {}), includeHidden: true };
          if (request.id != null) codexPlusModelListRequestIds.add(String(request.id));
        }
        if (event?.type === "message") patchMcpModelResponseData(event.data);
      } catch (error) {
        window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
        window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
      }
      return originalDispatchEvent.call(this, event);
    };

    window.addEventListener("message", (event) => {
      try {
        patchMcpModelResponseData(event?.data);
      } catch (error) {
        window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
        window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
      }
    }, true);
  }

  function patchMcpModelResponseData(data) {
    if (data?.type !== "mcp-response") return false;
    const message = data.message || data.response;
    const requestId = message?.id != null ? String(message.id) : "";
    if (codexPlusModelListRequestIds.size > 0 && !codexPlusModelListRequestIds.has(requestId)) return false;
    codexPlusModelListRequestIds.delete(requestId);
    return patchModelContainer(data) || patchModelContainer(message) || patchModelContainer(message?.result) || patchModelContainer(message?.result?.data);
  }

  function appServerModelRequestMethod(method, params) {
    if (method === "send-cli-request-for-host" && params?.method) return String(params.method);
    if (method === "vscode://codex/list-plugins") return "list-plugins";
    if (method === "vscode://codex/plugin/install") return "install-plugin";
    if (method === "vscode://codex/plugin/uninstall") return "uninstall-plugin";
    if (method === "plugin/list") return "list-plugins";
    if (method === "plugin/install") return "install-plugin";
    if (method === "plugin/uninstall") return "uninstall-plugin";
    return String(method || "");
  }

  function patchAppServerModelResult(method, result) {
    if (method !== "list-models-for-host") return result;
    try {
      if (Array.isArray(result)) patchModelArray(result, true);
      if (Array.isArray(result?.data)) patchModelArray(result.data, true);
      if (Array.isArray(result?.models)) patchModelArray(result.models, true);
      patchModelContainer(result);
      patchObjectGraphForModels(result, new WeakSet(), 0);
      sendCodexPlusDiagnostic("model_app_server_result_patched", {
        method,
        modelCount: Array.isArray(result?.data) ? result.data.length : Array.isArray(result?.models) ? result.models.length : Array.isArray(result) ? result.length : null,
      });
    } catch (error) {
      window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
      window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
    }
    return result;
  }

  function patchAppServerModelRequestClient(client) {
    if (!client || typeof client.sendRequest !== "function") return false;
    if (client.__codexPlusModelRequestPatch === codexAppServerModelRequestPatchVersion) return true;
    const originalSendRequest = client.__codexPlusModelOriginalSendRequest || client.sendRequest.bind(client);
    client.__codexPlusModelOriginalSendRequest = originalSendRequest;
    client.sendRequest = async function codexPlusModelPatchedSendRequest(method, params, options) {
      const result = await originalSendRequest(method, params, options);
      if (!codexPlusModelUnlockEnabled()) return result;
      if (!codexPlusModelNames().length) await loadCodexModelCatalog();
      return patchAppServerModelResult(appServerModelRequestMethod(String(method || ""), params), result);
    };
    client.__codexPlusModelRequestPatch = codexAppServerModelRequestPatchVersion;
    return true;
  }

  function installAppServerModelRequestPatch() {
    if (window.__codexPlusAppServerModelRequestPatchInstalled === codexAppServerModelRequestPatchVersion) return;
    const patch = async () => {
      try {
        const module = await loadCodexAppModule("app-server-manager-signals-");
        const candidates = Object.values(module).filter((value) => value && typeof value === "object");
        let patchedCount = 0;
        for (const candidate of candidates) {
          if (patchAppServerModelRequestClient(candidate)) patchedCount += 1;
          if (typeof candidate.sendRequest !== "function" && typeof candidate.get === "function") {
            try {
              if (patchAppServerModelRequestClient(candidate.get())) patchedCount += 1;
            } catch {
            }
          }
        }
        if (patchedCount > 0) {
          window.__codexPlusAppServerModelRequestPatchInstalled = codexAppServerModelRequestPatchVersion;
          sendCodexPlusDiagnostic("model_app_server_request_patch_installed", {
            candidateCount: candidates.length,
            patchedCount,
          });
        } else {
          sendCodexPlusDiagnostic("model_app_server_request_patch_not_found", {
            exportCount: Object.keys(module || {}).length,
            candidateCount: candidates.length,
          });
        }
      } catch (error) {
        sendCodexPlusDiagnostic("model_app_server_request_patch_failed", {
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
      }
    };
    void patch();
  }

  function ensureCodexModelWhitelistInstalls() {
    if (!codexPlusModelUnlockEnabled()) return;
    installModelJsonResponsePatch();
    patchAppServerModelMessages();
    installAppServerModelRequestPatch();
  }

  function runCodexModelWhitelistRefreshPass() {
    if (!codexPlusModelUnlockEnabled() || !codexPlusModelNames().length) return false;
    let changed = false;
    try {
      patchStatsigModelWhitelist();
      if (patchReactModelState()) changed = true;
      installAppServerModelRequestPatch();
    } catch (error) {
      window.__codexPlusModelPatchFailures = window.__codexPlusModelPatchFailures || [];
      window.__codexPlusModelPatchFailures.push(String(error?.stack || error));
    }
    return changed;
  }

  function scheduleCodexModelWhitelistRefresh(durationMs = 2500) {
    if (!codexPlusModelUnlockEnabled()) return;
    codexModelWhitelistRefreshUntil = Math.max(codexModelWhitelistRefreshUntil, Date.now() + durationMs);
    if (codexModelWhitelistRefreshTimer) return;
    sendCodexPlusDiagnostic("model_whitelist_refresh_scheduled", { durationMs });
    const tick = () => {
      codexModelWhitelistRefreshTimer = 0;
      runCodexModelWhitelistRefreshPass();
      if (Date.now() < codexModelWhitelistRefreshUntil) {
        codexModelWhitelistRefreshTimer = window.setTimeout(tick, 120);
      }
    };
    tick();
  }

  function patchCodexModelWhitelist() {
    ensureCodexModelWhitelistInstalls();
    if (!codexPlusModelNames().length) {
      loadCodexModelCatalog();
      return;
    }
    runCodexModelWhitelistRefreshPass();
  }

  function refreshCodexModelWhitelistFromScan(mutations) {
    ensureCodexModelWhitelistInstalls();
    if (!codexPlusModelNames().length) {
      loadCodexModelCatalog();
      return;
    }
    if (shouldScheduleReactModelStatePatch(mutations)) {
      scheduleCodexModelWhitelistRefresh();
    } else {
      runCodexModelWhitelistRefreshPass();
    }
  }

  function threadIdVariants(sessionId) {
    if (typeof sessionId !== "string" || !sessionId.trim()) return [];
    const id = sessionId.trim();
    const bareId = id.startsWith("local:") ? id.slice("local:".length) : id;
    return uniqueValues([id, bareId, `local:${bareId}`]);
  }

  function projectMoveSessionKey(sessionId) {
    const variants = threadIdVariants(sessionId);
    const bareId = variants.find((id) => !id.startsWith("local:"));
    return bareId || variants[0] || "";
  }

  function uuidV7TimestampMs(sessionId) {
    const id = projectMoveSessionKey(sessionId).replaceAll("-", "");
    if (!/^[0-9a-fA-F]{12}/.test(id)) return 0;
    const timestamp = Number.parseInt(id.slice(0, 12), 16);
    return Number.isFinite(timestamp) ? timestamp : 0;
  }

  function numericTimestamp(value) {
    const timestamp = Number(value);
    return Number.isFinite(timestamp) && timestamp > 0 ? timestamp : 0;
  }

  function timestampValueToMs(value) {
    const timestamp = numericTimestamp(value);
    if (!timestamp) return 0;
    return timestamp < 1000000000000 ? timestamp * 1000 : timestamp;
  }

  function sortMsForSession(sessionId, preferredValue) {
    return numericTimestamp(preferredValue) || uuidV7TimestampMs(sessionId);
  }

  function timestampMsFromPayload(payload) {
    return numericTimestamp(payload?.updated_at_ms) || timestampValueToMs(payload?.updated_at) || numericTimestamp(payload?.created_at_ms);
  }

  function relativeTimeLabel(timestampMs, nowMs = Date.now()) {
    const timestamp = numericTimestamp(timestampMs);
    if (!timestamp) return "";
    const elapsedSeconds = Math.max(0, Math.floor((nowMs - timestamp) / 1000));
    if (elapsedSeconds < 60) return "刚刚";
    const elapsedMinutes = Math.floor(elapsedSeconds / 60);
    if (elapsedMinutes < 60) return `${elapsedMinutes} 分`;
    const elapsedHours = Math.floor(elapsedMinutes / 60);
    if (elapsedHours < 24) return `${elapsedHours} 小时`;
    const elapsedDays = Math.floor(elapsedHours / 24);
    if (elapsedDays < 7) return `${elapsedDays} 天`;
    const elapsedWeeks = Math.floor(elapsedDays / 7);
    if (elapsedWeeks < 5) return `${elapsedWeeks} 周`;
    const elapsedMonths = Math.floor(elapsedDays / 30);
    if (elapsedMonths < 12) return `${Math.max(1, elapsedMonths)} 月`;
    return `${Math.floor(elapsedDays / 365)} 年`;
  }

  function normalizeWorkspacePath(path) {
    const normalized = String(path || "").trim().replace(/\\/g, "/").replace(/\/+$/, "");
    return normalized || String(path || "").trim();
  }

  function sameWorkspacePath(left, right) {
    const leftPath = normalizeWorkspacePath(left);
    const rightPath = normalizeWorkspacePath(right);
    return !!leftPath && !!rightPath && leftPath === rightPath;
  }

  function displayProjectName(path) {
    const trimmed = String(path || "").replace(/\/+$/, "");
    return trimmed.split(/[\\/]+/).filter(Boolean).pop() || trimmed || "未命名项目";
  }

  function normalizeProjectLabel(value) {
    return String(value || "").replace(/\s+/g, " ").trim();
  }

  function projectsSection() {
    return document.querySelector('[data-app-action-sidebar-section-heading="Projects"]');
  }

  function chatsSection() {
    return document.querySelector('[data-app-action-sidebar-section-heading="Chats"]');
  }

  function projectRowListItem(projectRow) {
    return projectRow.closest?.('[role="listitem"][aria-label]') || projectRow.closest?.('[role="listitem"]') || projectRow;
  }

  function nativeProjectTargets() {
    const section = projectsSection();
    const seen = new Set();
    const targets = [];
    Array.from(document.querySelectorAll('[data-app-action-sidebar-project-row]')).forEach((row) => {
      if (section && !section.contains(row)) return;
      const path = row.getAttribute("data-app-action-sidebar-project-id") || "";
      const normalizedPath = normalizeWorkspacePath(path);
      if (!normalizedPath || seen.has(normalizedPath)) return;
      const label = row.getAttribute("data-app-action-sidebar-project-label") || row.getAttribute("aria-label") || displayProjectName(path);
      seen.add(normalizedPath);
      targets.push({ kind: "project", label: String(label || displayProjectName(path)), description: path, path, normalizedPath, row, listItem: projectRowListItem(row) });
    });
    return targets;
  }

  function serializableProjectTarget(target) {
    return { kind: target.kind, label: target.label, description: target.description, path: target.path, normalizedPath: target.normalizedPath || normalizeWorkspacePath(target.path) };
  }

  function projectMoveTargets() {
    return [
      { kind: "projectless", label: "普通对话", description: "不属于任何项目", path: "", normalizedPath: "" },
      ...nativeProjectTargets().map(serializableProjectTarget),
    ];
  }

  function readLegacyProjectMoveProjection() {
    try {
      const parsed = JSON.parse(localStorage.getItem(legacyProjectMoveOverridesKey) || "{}");
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
      const now = Date.now();
      const next = {};
      for (const [key, value] of Object.entries(parsed)) {
        if (!value || typeof value !== "object" || !value.targetCwd) continue;
        const sessionId = projectMoveSessionKey(value.sessionId || key);
        if (!sessionId) continue;
        next[sessionId] = {
          sessionId,
          targetKind: "project",
          targetCwd: String(value.targetCwd),
          targetLabel: String(value.targetLabel || displayProjectName(value.targetCwd)),
          title: String(value.title || ""),
          sortMs: sortMsForSession(sessionId, value.sortMs || value.updatedAtMs || value.updated_at_ms),
          sortMsTrusted: false,
          at: typeof value.at === "number" ? value.at : now,
        };
      }
      return next;
    } catch {
      return {};
    }
  }

  function readProjectMoveProjection() {
    try {
      const parsed = JSON.parse(localStorage.getItem(projectMoveProjectionKey) || "{}");
      const raw = parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
      const merged = { ...readLegacyProjectMoveProjection(), ...raw };
      const now = Date.now();
      const projection = {};
      for (const [key, value] of Object.entries(merged)) {
        if (!value || typeof value !== "object") continue;
        const sessionId = projectMoveSessionKey(value.sessionId || key);
        if (!sessionId) continue;
        if (typeof value.at === "number" && now - value.at > projectMoveProjectionTtlMs) continue;
        const targetKind = value.targetKind === "projectless" ? "projectless" : "project";
        const targetCwd = String(value.targetCwd || value.path || "");
        if (targetKind === "project" && !targetCwd) continue;
        projection[sessionId] = {
          sessionId,
          targetKind,
          targetCwd,
          targetLabel: String(value.targetLabel || value.label || (targetKind === "projectless" ? "普通对话" : displayProjectName(targetCwd))),
          title: String(value.title || ""),
          sortMs: sortMsForSession(sessionId, value.sortMs || value.updatedAtMs || value.updated_at_ms),
          sortMsTrusted: value.sortMsTrusted === true,
          at: typeof value.at === "number" ? value.at : now,
        };
      }
      return projection;
    } catch {
      return readLegacyProjectMoveProjection();
    }
  }

  function writeProjectMoveProjection(projection) {
    try {
      localStorage.setItem(projectMoveProjectionKey, JSON.stringify(projection || {}));
      localStorage.removeItem(legacyProjectMoveOverridesKey);
    } catch (error) {
      window.__codexProjectMoveProjectionFailures = window.__codexProjectMoveProjectionFailures || [];
      window.__codexProjectMoveProjectionFailures.push(String(error?.stack || error));
    }
  }

  function saveProjectMoveProjection(ref, target, sortMs) {
    const id = projectMoveSessionKey(ref.session_id);
    if (!id || !target) return;
    const projection = readProjectMoveProjection();
    projection[id] = {
      sessionId: id,
      targetKind: target.kind === "projectless" ? "projectless" : "project",
      targetCwd: target.path || "",
      targetLabel: target.label || (target.kind === "projectless" ? "普通对话" : displayProjectName(target.path)),
      title: ref.title || "",
      sortMs: sortMsForSession(ref.session_id, sortMs || target.sortMs),
      sortMsTrusted: target.sortMsTrusted === true,
      at: Date.now(),
    };
    writeProjectMoveProjection(projection);
  }

  function clearProjectMoveProjection(ref) {
    const projection = readProjectMoveProjection();
    const keys = threadIdVariants(ref.session_id).map(projectMoveSessionKey).filter(Boolean);
    let changed = false;
    keys.forEach((key) => {
      if (Object.prototype.hasOwnProperty.call(projection, key)) {
        delete projection[key];
        changed = true;
      }
    });
    if (changed) writeProjectMoveProjection(projection);
  }

  function projectionForSessionId(sessionId, projection = readProjectMoveProjection()) {
    const key = projectMoveSessionKey(sessionId);
    return key ? projection[key] || null : null;
  }

  function projectRowFromListItem(projectItem) {
    if (!projectItem) return null;
    if (projectItem.matches?.("[data-app-action-sidebar-project-row]")) return projectItem;
    return projectItem.querySelector?.("[data-app-action-sidebar-project-row]") || null;
  }

  function targetPath(target) {
    return target?.path || target?.targetCwd || "";
  }

  function targetLabel(target) {
    return target?.label || target?.targetLabel || displayProjectName(targetPath(target));
  }

  function projectItemMatchesTarget(projectItem, target) {
    const projectRow = projectRowFromListItem(projectItem);
    const projectPath = projectRow?.getAttribute?.("data-app-action-sidebar-project-id") || "";
    if (projectPath && sameWorkspacePath(projectPath, targetPath(target))) return true;
    const actual = normalizeProjectLabel(projectRow?.getAttribute?.("data-app-action-sidebar-project-label") || projectItem?.getAttribute?.("aria-label"));
    const labels = uniqueValues([targetLabel(target), displayProjectName(targetPath(target))]).map(normalizeProjectLabel).filter(Boolean);
    return !!actual && labels.includes(actual);
  }

  function findProjectListItem(target) {
    const nativeTarget = nativeProjectTargets().find((project) => sameWorkspacePath(project.path, targetPath(target)));
    if (nativeTarget?.listItem) return nativeTarget.listItem;
    const section = projectsSection();
    if (!section) return null;
    return Array.from(section.querySelectorAll('[role="listitem"][aria-label]')).find((item) => projectItemMatchesTarget(item, target)) || null;
  }

  function closestProjectListItem(row) {
    const item = row.closest?.('[role="listitem"][aria-label]');
    return item?.closest?.('[data-app-action-sidebar-section-heading="Projects"]') ? item : null;
  }

  function rowIsInChats(row) {
    return !!row.closest?.('[data-app-action-sidebar-section-heading="Chats"]');
  }

  function chatsThreadList() {
    return chatsSection()?.querySelector?.('[role="list"][aria-label="对话"], [role="list"]') || null;
  }

  function rowIsUnderTargetProject(row, target) {
    const item = closestProjectListItem(row);
    return !!item && projectItemMatchesTarget(item, target);
  }

  function rowIsUnderTarget(row, target) {
    return target?.targetKind === "projectless" || target?.kind === "projectless" ? rowIsInChats(row) : rowIsUnderTargetProject(row, target);
  }

  function rowListItem(row) {
    return row.closest?.('[role="listitem"]') || row;
  }

  function rowContentRoot(row) {
    return Array.from(row?.children || []).find((child) => String(child.className || "").includes("h-full w-full items-center")) || null;
  }

  function normalizedText(node) {
    return String(node?.textContent || "").replace(/\s+/g, " ").trim();
  }

  function classNameText(node) {
    return String(node?.className || "");
  }

  function isRelativeTimeText(text) {
    const value = String(text || "").replace(/\s+/g, " ").trim();
    return /^(刚刚|just now|\d+\s*(秒|秒钟|分|分钟|小时|天|日|周|星期|个月|月|年|sec|secs|second|seconds|min|mins|minute|minutes|h|hr|hrs|hour|hours|d|day|days|w|wk|wks|week|weeks|mo|mos|month|months|y|yr|yrs|year|years))$/i.test(value);
  }

  function nodeIsThreadTitle(row, node) {
    return Array.from(row?.querySelectorAll?.('[data-thread-title], .truncate.select-none, .truncate.text-base') || [])
      .some((titleNode) => titleNode === node || titleNode.contains(node));
  }

  function closestTimeWrapper(row, node) {
    const root = rowContentRoot(row) || row;
    let current = node?.parentElement || null;
    while (current && current !== root && current !== row) {
      const className = classNameText(current);
      if (current.dataset?.codexProjectMoveTimeWrapper === "true" || (className.includes("ml-[3px]") && className.includes("min-w-[26px]"))) return current;
      current = current.parentElement;
    }
    return null;
  }

  function nodeInsideStatusIcon(row, node) {
    const stop = closestTimeWrapper(row, node) || rowContentRoot(row) || row;
    let current = node || null;
    while (current && current !== stop && current !== row) {
      const className = classNameText(current);
      if (className.includes("animate-spin")) return true;
      if (className.includes("size-5") && className.includes("shrink-0")) return true;
      if (className.includes("contain-paint") && className.includes("contain-layout")) return true;
      current = current.parentElement;
    }
    return false;
  }

  function cleanupManagedStatusIconTimeNodes(row) {
    Array.from(row?.querySelectorAll?.('[data-codex-project-move-time="true"]') || []).forEach((node) => {
      if (!nodeInsideStatusIcon(row, node)) return;
      const text = normalizedText(node);
      delete node.dataset.codexProjectMoveTime;
      delete node.dataset.codexProjectMoveTimeMs;
      if (node.children.length === 0 && isRelativeTimeText(text)) node.textContent = "";
    });
  }

  function nodeLooksLikeTimeLabel(row, node) {
    if (nodeInsideStatusIcon(row, node)) return false;
    if (node?.dataset?.codexProjectMoveTime === "true") return true;
    if (node.children.length > 0) return false;
    const text = normalizedText(node);
    const className = classNameText(node);
    if ((className.includes("tabular-nums") || className.includes("text-token-description-foreground")) && text.length <= 24) return true;
    if (!isRelativeTimeText(text)) return false;
    const rowRect = row?.getBoundingClientRect?.();
    const nodeRect = node?.getBoundingClientRect?.();
    if (!rowRect || !nodeRect || rowRect.width <= 0 || nodeRect.width <= 0) return false;
    return nodeRect.left >= rowRect.left + rowRect.width * 0.45 || nodeRect.right >= rowRect.right - 96;
  }

  function rowTimeLabelCandidates(row) {
    cleanupManagedStatusIconTimeNodes(row);
    const root = rowContentRoot(row) || row;
    const raw = Array.from(root?.querySelectorAll?.("div, span, time, small") || []).filter((node) => {
      if (nodeIsThreadTitle(row, node)) return false;
      return nodeLooksLikeTimeLabel(row, node);
    });
    return raw.filter((node) => !raw.some((other) => other !== node && node.contains(other)));
  }

  function rowTimeLabelNode(row) {
    const candidates = rowTimeLabelCandidates(row);
    return candidates.find((node) => node.dataset?.codexProjectMoveTime !== "true" && !node.closest?.('[data-codex-project-move-time-wrapper="true"]')) || candidates[0] || null;
  }

  function removeTimeLabelNode(row, node) {
    if (!node || !row?.contains?.(node)) return;
    const wrapper = node.closest?.('[data-codex-project-move-time-wrapper="true"]') || closestTimeWrapper(row, node);
    if (wrapper && wrapper !== row && row.contains(wrapper)) {
      wrapper.remove();
      return;
    }
    node.remove();
  }

  function cleanupRowTimeLabels(row, keepNode) {
    if (!keepNode) return;
    rowTimeLabelCandidates(row).forEach((node) => {
      if (node === keepNode) return;
      if (node.dataset?.codexProjectMoveTime === "true" || node.closest?.('[data-codex-project-move-time-wrapper="true"]')) removeTimeLabelNode(row, node);
    });
  }

  function ensureRowTimeLabelNode(row) {
    const existing = rowTimeLabelNode(row);
    if (existing) {
      cleanupRowTimeLabels(row, existing);
      return existing;
    }
    const root = rowContentRoot(row);
    if (!root) return null;
    const wrapper = document.createElement("div");
    wrapper.className = "ml-[3px] flex items-center justify-end gap-1 min-w-[26px]";
    wrapper.dataset.codexProjectMoveTimeWrapper = "true";
    const inner = document.createElement("div");
    const label = document.createElement("div");
    label.className = "text-token-description-foreground text-sm leading-4 empty:hidden tabular-nums overflow-visible truncate text-right group-focus-within:opacity-0 group-hover:opacity-0";
    label.dataset.codexProjectMoveTime = "true";
    inner.appendChild(label);
    wrapper.appendChild(inner);
    root.appendChild(wrapper);
    return label;
  }

  function updateRowTimeLabel(row, sortMs) {
    const label = ensureRowTimeLabelNode(row);
    if (!label) return;
    const timestamp = numericTimestamp(sortMs);
    const text = relativeTimeLabel(timestamp);
    label.dataset.codexProjectMoveTime = "true";
    label.dataset.codexProjectMoveTimeMs = String(timestamp || 0);
    if (text && label.textContent !== text) label.textContent = text;
    cleanupRowTimeLabels(row, label);
  }

  function rowProjectionKind(row) {
    return row?.dataset?.codexProjectMoveTargetKind || rowListItem(row)?.dataset?.codexProjectMoveTargetKind || "";
  }

  function rowSortMs(row, ref = sessionRefFromRow(row), target = null) {
    return sortMsForSession(ref.session_id, target?.sortMs || row?.dataset?.codexProjectMoveSortMs || rowListItem(row)?.dataset?.codexProjectMoveSortMs);
  }

  function threadRowFromListItem(item) {
    if (!item) return null;
    if (item.matches?.("[data-app-action-sidebar-thread-id]")) return item;
    return item.querySelector?.("[data-app-action-sidebar-thread-id]") || null;
  }

  function rowPinned(row) {
    return row?.getAttribute?.("data-app-action-sidebar-thread-pinned") === "true" || rowListItem(row)?.getAttribute?.("data-app-action-sidebar-thread-pinned") === "true";
  }

  function insertRowItemByTime(list, item, row, target) {
    const ref = sessionRefFromRow(row);
    const sortMs = rowSortMs(row, ref, target);
    item.dataset.codexProjectMoveSortMs = String(sortMs || 0);
    row.dataset.codexProjectMoveSortMs = String(sortMs || 0);
    if (target?.sortMsTrusted) updateRowTimeLabel(row, sortMs);
    const pinned = rowPinned(row);
    const sessionKey = projectMoveSessionKey(ref.session_id);
    const existingItems = Array.from(list.children).filter((child) => child !== item);
    let firstNonThreadItem = null;
    for (const child of existingItems) {
      const childRow = threadRowFromListItem(child);
      if (!childRow) {
        firstNonThreadItem = firstNonThreadItem || child;
        continue;
      }
      const childPinned = rowPinned(childRow);
      if (childPinned && !pinned) continue;
      if (!childPinned && pinned) {
        list.insertBefore(item, child);
        return;
      }
      const childRef = sessionRefFromRow(childRow);
      const childSortMs = rowSortMs(childRow, childRef);
      const childKey = projectMoveSessionKey(childRef.session_id);
      if (sortMs > childSortMs || (sortMs === childSortMs && sessionKey > childKey)) {
        list.insertBefore(item, child);
        return;
      }
    }
    if (firstNonThreadItem) {
      list.insertBefore(item, firstNonThreadItem);
      return;
    }
    list.appendChild(item);
  }

  function projectMoveInjectedList(projectItem) {
    let list = projectItem.querySelector('[data-codex-project-move-injected-list="true"]');
    if (!list) {
      const body = Array.from(projectItem.children).find((child) => child.classList?.contains("overflow-hidden")) || projectItem;
      list = document.createElement("div");
      list.setAttribute("role", "list");
      list.setAttribute("data-codex-project-move-injected-list", "true");
      list.className = "flex flex-col";
      body.appendChild(list);
    }
    return list;
  }

  function projectThreadList(projectItem, target) {
    const targetCwd = targetPath(target);
    const projectLists = Array.from(projectItem.querySelectorAll("[data-app-action-sidebar-project-list-id]"));
    return projectLists.find((list) => sameWorkspacePath(list.getAttribute("data-app-action-sidebar-project-list-id"), targetCwd))
      || projectLists[0]
      || projectMoveInjectedList(projectItem);
  }

  function projectEmptyStateNodes(projectItem) {
    const emptyLabels = new Set(["暂无对话", "No conversations"]);
    return Array.from(projectItem.querySelectorAll("div, span")).filter((node) => {
      if (node.classList?.contains("overflow-hidden")) return false;
      if (node.closest('[data-app-action-sidebar-thread-id], [data-codex-project-move-injected-list="true"]')) return false;
      return emptyLabels.has(normalizeProjectLabel(node.textContent));
    });
  }

  function setProjectEmptyStateHidden(projectItem, hidden) {
    projectEmptyStateNodes(projectItem).forEach((node) => {
      if (hidden) {
        node.dataset.codexProjectMoveEmptyHidden = "true";
        node.classList.add("codex-project-move-hidden");
      } else if (node.dataset.codexProjectMoveEmptyHidden === "true") {
        delete node.dataset.codexProjectMoveEmptyHidden;
        node.classList.remove("codex-project-move-hidden");
      }
    });
  }

  function updateProjectMoveEmptyStates() {
    document.querySelectorAll('[data-codex-project-move-injected-list="true"]').forEach((list) => {
      const projectItem = list.closest('[role="listitem"][aria-label]');
      const hasRows = Array.from(list.children).some((child) => child.querySelector?.("[data-app-action-sidebar-thread-id]") || child.matches?.("[data-app-action-sidebar-thread-id]"));
      if (!hasRows) list.remove();
      if (projectItem) setProjectEmptyStateHidden(projectItem, hasRows);
    });
    document.querySelectorAll('[data-codex-project-move-empty-hidden="true"]').forEach((node) => {
      const projectItem = node.closest('[role="listitem"][aria-label]');
      const list = projectItem?.querySelector?.('[data-codex-project-move-injected-list="true"]');
      if (!list || list.children.length === 0) {
        delete node.dataset.codexProjectMoveEmptyHidden;
        node.classList.remove("codex-project-move-hidden");
      }
    });
  }

  function moveRowToProjectList(row, target) {
    const projectItem = findProjectListItem(target);
    if (!projectItem) return false;
    const list = projectThreadList(projectItem, target);
    const item = rowListItem(row);
    if (!list) return false;
    insertRowItemByTime(list, item, row, target);
    cachedSessionRowsAt = 0;
    item.dataset.codexProjectMoveTargetKind = "project";
    item.dataset.codexProjectMoveTargetCwd = targetPath(target);
    row.dataset.codexProjectMoveTargetKind = "project";
    row.dataset.codexProjectMoveTargetCwd = targetPath(target);
    setProjectEmptyStateHidden(projectItem, true);
    return true;
  }

  function moveRowToChats(row, target = null) {
    const list = chatsThreadList();
    if (!list) return false;
    const item = rowListItem(row);
    insertRowItemByTime(list, item, row, target);
    cachedSessionRowsAt = 0;
    item.dataset.codexProjectMoveTargetKind = "projectless";
    row.dataset.codexProjectMoveTargetKind = "projectless";
    delete item.dataset.codexProjectMoveTargetCwd;
    delete row.dataset.codexProjectMoveTargetCwd;
    updateProjectMoveEmptyStates();
    return true;
  }

  function applyProjectMoveProjection() {
    if (!codexPlusSettings().projectMove) return;
    const projection = readProjectMoveProjection();
    const targetRowsById = new Map();
    const settledRefs = [];
    const now = Date.now();
    const rows = sessionRows(true);
    rows.forEach((row) => {
      const ref = sessionRefFromRow(row);
      const target = projectionForSessionId(ref.session_id, projection);
      if (target && rowIsUnderTarget(row, target)) {
        const rowId = projectMoveSessionKey(ref.session_id);
        const hadProjectionKind = !!rowProjectionKind(row);
        const existingRow = targetRowsById.get(rowId);
        if (existingRow && existingRow !== row) {
          const existingIsProjection = !!rowProjectionKind(existingRow);
          const currentIsProjection = !!rowProjectionKind(row);
          const rowToRemove = existingIsProjection && !currentIsProjection ? existingRow : row;
          rowListItem(rowToRemove).remove();
          if (rowToRemove === existingRow) targetRowsById.set(rowId, row);
          if (rowToRemove === row) return;
        } else {
          targetRowsById.set(rowId, row);
        }
        if (!hadProjectionKind && typeof target.at === "number" && now - target.at > projectMoveProjectionSettleMs) settledRefs.push(ref);
        const moved = target.targetKind === "projectless" ? moveRowToChats(row, target) : moveRowToProjectList(row, target);
        if (moved) targetRowsById.set(rowId, row);
        const projectItem = closestProjectListItem(row);
        if (projectItem) setProjectEmptyStateHidden(projectItem, true);
      }
    });
    rows.forEach((row) => {
      const ref = sessionRefFromRow(row);
      const rowId = projectMoveSessionKey(ref.session_id);
      const target = projectionForSessionId(ref.session_id, projection);
      if (!target) {
        const item = rowListItem(row);
        delete row.dataset.codexProjectMoveTargetKind;
        delete row.dataset.codexProjectMoveTargetCwd;
        delete item.dataset.codexProjectMoveTargetKind;
        delete item.dataset.codexProjectMoveTargetCwd;
        return;
      }
      if (rowIsUnderTarget(row, target)) return;
      if (targetRowsById.has(rowId)) {
        rowListItem(row).remove();
        return;
      }
      const moved = target.targetKind === "projectless" ? moveRowToChats(row, target) : moveRowToProjectList(row, target);
      if (moved) targetRowsById.set(rowId, row);
    });
    settledRefs.forEach(clearProjectMoveProjection);
    updateProjectMoveEmptyStates();
  }

  function scheduleProjectMoveProjection() {
    if (!codexPlusSettings().projectMove || window.__codexProjectMoveProjectionTimer) return;
    window.__codexProjectMoveProjectionTimer = setTimeout(() => {
      if (window.__codexProjectMoveRuntimeId !== codexProjectMoveRuntimeId) return;
      window.__codexProjectMoveProjectionTimer = null;
      applyProjectMoveProjection();
    }, 80);
  }

  async function refreshRecentConversationsForHost() {
    try {
      const signals = await import("./assets/app-server-manager-signals-C1h8B-R-.js");
      if (typeof signals.rn === "function") await signals.rn("refresh-recent-conversations-for-host", { hostId: "local", sortKey: "updated_at" });
    } catch (error) {
      window.__codexProjectMoveRefreshFailures = window.__codexProjectMoveRefreshFailures || [];
      window.__codexProjectMoveRefreshFailures.push(String(error?.stack || error));
    }
  }

  function refreshAfterProjectMove() {
    const refreshVisibleSidebar = () => {
      applyProjectMoveProjection();
      scheduleChatsSortCorrection(0);
    };
    refreshVisibleSidebar();
    refreshRecentConversationsForHost().finally(() => {
      projectMoveRefreshDelaysMs.forEach((delay) => setTimeout(refreshVisibleSidebar, delay));
    });
  }

  function visibleChatsRows() {
    const list = chatsThreadList();
    if (!list) return [];
    return Array.from(list.children).map(threadRowFromListItem).filter(Boolean).filter((row) => rowIsInChats(row));
  }

  function chatsSortNeedsCorrection(rows) {
    let previousPinned = true;
    let previousSortMs = Infinity;
    let previousKey = "\uffff";
    for (const row of rows) {
      const pinned = rowPinned(row);
      const ref = sessionRefFromRow(row);
      const sortMs = rowSortMs(row, ref);
      const key = projectMoveSessionKey(ref.session_id);
      if (previousPinned && !pinned) {
        previousPinned = false;
        previousSortMs = sortMs;
        previousKey = key;
        continue;
      }
      if (!previousPinned && pinned) return true;
      if (sortMs > previousSortMs || (sortMs === previousSortMs && key > previousKey)) return true;
      previousSortMs = sortMs;
      previousKey = key;
    }
    return false;
  }

  function reorderChatsRows(rows) {
    const list = chatsThreadList();
    if (!list || rows.length < 2) return;
    const rowItems = new Set(rows.map(rowListItem));
    const firstNonThreadItem = Array.from(list.children).find((child) => !rowItems.has(child) && !threadRowFromListItem(child));
    const orderedRows = [...rows].sort((left, right) => {
      const leftPinned = rowPinned(left);
      const rightPinned = rowPinned(right);
      if (leftPinned !== rightPinned) return leftPinned ? -1 : 1;
      const leftRef = sessionRefFromRow(left);
      const rightRef = sessionRefFromRow(right);
      const leftSortMs = rowSortMs(left, leftRef);
      const rightSortMs = rowSortMs(right, rightRef);
      if (leftSortMs !== rightSortMs) return rightSortMs - leftSortMs;
      return projectMoveSessionKey(rightRef.session_id).localeCompare(projectMoveSessionKey(leftRef.session_id));
    });
    orderedRows.forEach((row) => list.insertBefore(rowListItem(row), firstNonThreadItem || null));
    cachedSessionRowsAt = 0;
  }

  async function applyChatsSortCorrection() {
    if (!codexPlusSettings().projectMove || chatsSortInFlight) return;
    const rows = visibleChatsRows();
    if (rows.length < 2) return;
    const refs = rows.map(sessionRefFromRow).filter((ref) => ref.session_id);
    const signature = refs.map((ref) => projectMoveSessionKey(ref.session_id)).join("|");
    const allRowsHaveSortMs = rows.every((row) => numericTimestamp(row.dataset.codexProjectMoveSortMs || rowListItem(row).dataset.codexProjectMoveSortMs));
    const shouldRefreshSortKeys = signature !== chatsSortSignature || !allRowsHaveSortMs || Date.now() - chatsSortLastFetchAt > chatsSortDbRefreshIntervalMs;
    if (!shouldRefreshSortKeys && !chatsSortNeedsCorrection(rows)) return;
    chatsSortInFlight = true;
    try {
      if (shouldRefreshSortKeys) {
        const result = await postJson("/thread-sort-keys", { sessions: refs }).catch(() => ({ status: "failed", sort_keys: [] }));
        chatsSortLastFetchAt = Date.now();
        const byId = new Map();
        if (result?.status === "ok" && Array.isArray(result?.sort_keys)) {
          result.sort_keys.forEach((item) => {
            const key = projectMoveSessionKey(String(item?.session_id || ""));
            if (key) byId.set(key, item);
          });
        }
        rows.forEach((row) => {
          const ref = sessionRefFromRow(row);
          const payload = byId.get(projectMoveSessionKey(ref.session_id));
          const trustedSortMs = timestampMsFromPayload(payload);
          const sortMs = trustedSortMs || sortMsForSession(ref.session_id, row.dataset.codexProjectMoveSortMs || rowListItem(row).dataset.codexProjectMoveSortMs);
          row.dataset.codexProjectMoveSortMs = String(sortMs || 0);
          rowListItem(row).dataset.codexProjectMoveSortMs = String(sortMs || 0);
          if (trustedSortMs) updateRowTimeLabel(row, trustedSortMs);
        });
      }
      if (chatsSortNeedsCorrection(rows)) reorderChatsRows(rows);
      chatsSortSignature = visibleChatsRows().map((row) => projectMoveSessionKey(sessionRefFromRow(row).session_id)).join("|");
    } finally {
      chatsSortInFlight = false;
    }
  }

  function scheduleChatsSortCorrection(delay = chatsSortRefreshIntervalMs) {
    if (!codexPlusSettings().projectMove || window.__codexProjectMoveChatsSortTimer) return;
    window.__codexProjectMoveChatsSortTimer = setTimeout(() => {
      if (window.__codexProjectMoveRuntimeId !== codexProjectMoveRuntimeId) return;
      window.__codexProjectMoveChatsSortTimer = null;
      applyChatsSortCorrection().catch((error) => {
        window.__codexProjectMoveSortFailures = window.__codexProjectMoveSortFailures || [];
        window.__codexProjectMoveSortFailures.push(String(error?.stack || error));
      }).finally(() => {
        if (codexPlusSettings().projectMove) scheduleChatsSortCorrection();
      });
    }, delay);
  }

  async function setProjectlessThreadIds(ref, mode) {
    const variants = threadIdVariants(ref.session_id);
    if (variants.length === 0) throw new Error("未找到会话 ID");
    const existingIds = await getCodexGlobalState("projectless-thread-ids").catch(() => []);
    const ids = Array.isArray(existingIds) ? existingIds : [];
    const variantSet = new Set(variants);
    const nextIds = mode === "add" ? uniqueValues([...ids, ...variants]) : ids.filter((id) => !variantSet.has(id));
    if (nextIds.length !== ids.length || nextIds.some((id, index) => id !== ids[index])) await setCodexGlobalState("projectless-thread-ids", nextIds);
  }

  async function clearThreadWorkspaceHints(ref) {
    const variants = threadIdVariants(ref.session_id);
    if (variants.length === 0) return;
    const hints = objectGlobalState(await getCodexGlobalState("thread-workspace-root-hints").catch(() => ({})));
    const hintKeys = variants.filter((id) => Object.prototype.hasOwnProperty.call(hints, id));
    if (hintKeys.length > 0) {
      hintKeys.forEach((id) => delete hints[id]);
      await setCodexGlobalState("thread-workspace-root-hints", hints);
    }
  }

  async function moveSessionToProjectless(ref) {
    if (!ref.session_id) throw new Error("未找到会话 ID");
    await setProjectlessThreadIds(ref, "add");
    await clearThreadWorkspaceHints(ref);
    const sortKey = await postJson("/thread-sort-key", ref).catch(() => ({}));
    return { status: "moved", session_id: ref.session_id, updated_at: sortKey?.updated_at, updated_at_ms: sortKey?.updated_at_ms, created_at_ms: sortKey?.created_at_ms };
  }

  function isNativeProjectTarget(target) {
    return target?.kind === "project" && nativeProjectTargets().some((project) => sameWorkspacePath(project.path, target.path));
  }

  async function moveSessionToProject(ref, target) {
    if (!ref.session_id) throw new Error("未找到会话 ID");
    if (!target?.path) throw new Error("目标项目路径为空");
    if (!isNativeProjectTarget(target)) throw new Error("目标项目不在 Codex 项目列表中");
    const result = await postJson("/move-thread-workspace", { ...ref, target_cwd: target.path });
    if (result.status !== "moved") throw new Error(result.message || "移动项目失败");
    await setProjectlessThreadIds(ref, "remove");
    await clearThreadWorkspaceHints(ref);
    return result;
  }

  function showToast(message, undoToken) {
    document.querySelectorAll(".codex-delete-toast").forEach((node) => node.remove());
    const toast = document.createElement("div");
    toast.className = "codex-delete-toast";
    toast.textContent = message;
    if (undoToken) {
      const undo = document.createElement("button");
      undo.textContent = "撤销";
      undo.addEventListener("click", async () => {
        const result = await postJson("/undo", { undo_token: undoToken });
        toast.textContent = result.message || "撤销完成";
        setTimeout(() => toast.remove(), 5000);
      });
      toast.appendChild(undo);
    }
    document.body.appendChild(toast);
    setTimeout(() => toast.remove(), 10000);
  }

  function upstreamWorktreeField(dialog, name) {
    return dialog.querySelector(`[data-codex-upstream-worktree-field="${name}"]`);
  }

  function upstreamWorktreePayload(dialog) {
    return {
      repoPath: upstreamWorktreeField(dialog, "repoPath")?.value || "",
      branchName: upstreamWorktreeField(dialog, "branchName")?.value || "",
      worktreePath: upstreamWorktreeField(dialog, "worktreePath")?.value || "",
      remote: upstreamWorktreeField(dialog, "remote")?.value || "upstream",
      baseBranch: upstreamWorktreeField(dialog, "baseBranch")?.value || "main",
      fetch: true,
    };
  }

  function readUpstreamBranchSelection() {
    try {
      return JSON.parse(sessionStorage.getItem(upstreamBranchSelectionKey) || "null");
    } catch {
      return null;
    }
  }

  function writeUpstreamBranchSelection(selection) {
    if (!selection) {
      sessionStorage.removeItem(upstreamBranchSelectionKey);
      return;
    }
    sessionStorage.setItem(upstreamBranchSelectionKey, JSON.stringify(selection));
  }

  function nativeBranchMenuCandidates() {
    return [...document.querySelectorAll('[role="menu"], [data-radix-menu-content], [cmdk-list]')];
  }

  function looksLikeBranchMenu(menu, trigger = branchMenuTriggerFromMenu(menu)) {
    const text = (menu.innerText || menu.textContent || "").toLowerCase();
    if (!branchMenuTriggerIsBranchControl(trigger)) return false;
    if (/^start in\b/.test(text) || /\bwork locally\b.*\bnew worktree\b.*\bcloud\b/s.test(text)) return false;
    return /\bbranches?\b|\bbranche\b|create and checkout new branch|create branch/.test(text);
  }

  function visibleElement(node) {
    if (!(node instanceof Element)) return false;
    const rect = node.getBoundingClientRect?.();
    return !!rect && rect.width > 0 && rect.height > 0;
  }

  function effectiveElementRect(node) {
    if (!(node instanceof Element)) return null;
    const rect = node.getBoundingClientRect?.();
    if (rect && rect.width > 0 && rect.height > 0) return rect;
    const controls = [...node.closest?.(".composer-footer")?.querySelectorAll?.("button, [role='button']") || []]
      .filter((candidate) => candidate !== node && visibleElement(candidate));
    const matching = controls.find((candidate) => normalizedElementText(candidate) === normalizedElementText(node));
    return matching?.getBoundingClientRect?.() || rect || null;
  }

  function sidebarProjectRows() {
    const section = projectsSection?.();
    return [...document.querySelectorAll('[data-app-action-sidebar-project-row][data-app-action-sidebar-project-id]')]
      .filter((row) => !section || section.contains(row));
  }

  function projectRowPath(row) {
    return row?.getAttribute?.("data-app-action-sidebar-project-id") || "";
  }

  function projectContextFromRow(row) {
    const path = projectRowPath(row);
    if (!path) return null;
    const label = row.getAttribute("data-app-action-sidebar-project-label")
      || row.getAttribute("aria-label")
      || displayProjectName(path);
    return {
      repoPath: path.startsWith("/") ? path : "",
      projectId: path.startsWith("/") ? "" : path,
      label: normalizeProjectLabel(label),
      at: Date.now(),
    };
  }

  function remoteProjectContextFromGlobalState(projectId) {
    const normalizedProjectId = String(projectId || "").trim();
    if (!normalizedProjectId) return null;
    return { projectId: normalizedProjectId, repoPath: "", label: "", at: Date.now() };
  }

  function readUpstreamProjectContext() {
    try {
      const context = JSON.parse(sessionStorage.getItem(upstreamProjectContextKey) || "null");
      if (!context || typeof context !== "object") return null;
      if (typeof context.at === "number" && Date.now() - context.at > upstreamProjectContextTtlMs) return null;
      if (!context.repoPath && !context.projectId) return null;
      return context;
    } catch {
      return null;
    }
  }

  function writeUpstreamProjectContext(context) {
    if (!context?.repoPath && !context?.projectId) return;
    try {
      sessionStorage.setItem(upstreamProjectContextKey, JSON.stringify({
        repoPath: context.repoPath || "",
        projectId: context.projectId || "",
        label: context.label || "",
        at: Date.now(),
      }));
    } catch {
    }
  }

  function projectContextFromStartButton(button) {
    const row = button?.closest?.('[data-app-action-sidebar-project-row][data-app-action-sidebar-project-id]');
    return projectContextFromRow(row);
  }

  function rememberStartNewChatProjectContext(event) {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    const button = target?.closest?.('button[aria-label^="Start new chat in "]');
    const context = projectContextFromStartButton(button);
    if (context) writeUpstreamProjectContext(context);
  }

  function visibleProjectRows() {
    return sidebarProjectRows().filter((row) => visibleElement(row));
  }

  function currentProjectRepoPathFromStartButton() {
    const startButtons = [...document.querySelectorAll('button[aria-label^="Start new chat in "]')]
      .filter((button) => visibleElement(button));
    const bottomHalf = window.innerHeight * 0.5;
    startButtons.sort((left, right) => {
      const leftRect = left.getBoundingClientRect();
      const rightRect = right.getBoundingClientRect();
      const leftScore = Math.abs(leftRect.y - bottomHalf) + Math.max(0, bottomHalf - leftRect.y) * 0.5;
      const rightScore = Math.abs(rightRect.y - bottomHalf) + Math.max(0, bottomHalf - rightRect.y) * 0.5;
      return leftScore - rightScore;
    });
    for (const button of startButtons) {
      const row = button.closest('[data-app-action-sidebar-project-row][data-app-action-sidebar-project-id]');
      const path = projectRowPath(row);
      if (path?.startsWith?.("/")) return path;
    }
    return "";
  }

  function currentProjectContextFromStartButton() {
    const startButtons = [...document.querySelectorAll('button[aria-label^="Start new chat in "]')]
      .filter((button) => visibleElement(button));
    const bottomHalf = window.innerHeight * 0.5;
    startButtons.sort((left, right) => {
      const leftRect = left.getBoundingClientRect();
      const rightRect = right.getBoundingClientRect();
      const leftScore = Math.abs(leftRect.y - bottomHalf) + Math.max(0, bottomHalf - leftRect.y) * 0.5;
      const rightScore = Math.abs(rightRect.y - bottomHalf) + Math.max(0, bottomHalf - rightRect.y) * 0.5;
      return leftScore - rightScore;
    });
    for (const button of startButtons) {
      const context = projectContextFromStartButton(button);
      if (context) return context;
    }
    return null;
  }

  function currentProjectRepoPathFromSelectedProjectButton() {
    const projectButtons = [...document.querySelectorAll('button[aria-haspopup="menu"]')]
      .filter((button) => visibleElement(button))
      .filter((button) => button.getBoundingClientRect().x > 300)
      .map((button) => (button.innerText || button.textContent || "").trim())
      .filter(Boolean);
    for (const label of projectButtons) {
      const match = visibleProjectRows().find((row) => {
        const rowLabel = row.getAttribute("data-app-action-sidebar-project-label") || row.getAttribute("aria-label") || "";
        return rowLabel.trim() === label;
      });
      const path = projectRowPath(match);
      if (path?.startsWith?.("/")) return path;
    }
    return "";
  }

  function projectContextFromProjectLabel(label) {
    const normalizedLabel = normalizeProjectLabel(label);
    if (!normalizedLabel) return null;
    const row = visibleProjectRows().find((candidate) => {
      const rowPath = projectRowPath(candidate);
      const rowLabels = [
        candidate.getAttribute("data-app-action-sidebar-project-label"),
        candidate.getAttribute("aria-label"),
        displayProjectName(rowPath),
      ].map(normalizeProjectLabel).filter(Boolean);
      return rowLabels.includes(normalizedLabel);
    });
    const context = projectContextFromRow(row);
    if (!context) return null;
    return context.projectId ? { ...remoteProjectContextFromGlobalState(context.projectId), label: context.label } : context;
  }

  function repoPathFromProjectLabel(label) {
    return projectContextFromProjectLabel(label)?.repoPath || "";
  }

  function contextMatchesProjectLabel(context, label) {
    const expected = normalizeProjectLabel(label);
    if (!expected) return true;
    const actual = normalizeProjectLabel(context?.label);
    return !actual || actual === expected;
  }

  function currentProjectContextFromStoredSelection(label = "") {
    const context = readUpstreamProjectContext();
    return contextMatchesProjectLabel(context, label) ? context : null;
  }

  function currentProjectContextForBranchMenu(menu, trigger = branchMenuTriggerFromMenu(menu)) {
    const footer = trigger?.closest?.(".composer-footer");
    const projectButton = footer ? [...footer.querySelectorAll('button, [role="button"]')]
      .filter((node) => node !== trigger && visibleElement(node))
      .filter((node) => {
        const rect = effectiveElementRect(node);
        const triggerRect = effectiveElementRect(trigger);
        return rect && triggerRect && rect.x < triggerRect.x;
      })
      .sort((left, right) => effectiveElementRect(left).x - effectiveElementRect(right).x)
      .find((node) => projectContextFromProjectLabel(normalizedElementText(node))) : null;
    const projectLabel = normalizedElementText(projectButton);
    return currentProjectContextFromStoredSelection(projectLabel)
      || projectContextFromProjectLabel(projectLabel)
      || currentProjectContextFromStoredSelection()
      || currentProjectContext();
  }

  function currentProjectRepoPathForBranchMenu(menu, trigger = branchMenuTriggerFromMenu(menu)) {
    return currentProjectContextForBranchMenu(menu, trigger)?.repoPath || "";
  }

  function currentProjectRepoPathFromExpandedRows() {
    const expandedRows = visibleProjectRows().filter((row) => row.getAttribute("data-app-action-sidebar-project-collapsed") === "false");
    const pathRows = expandedRows.filter((row) => projectRowPath(row).startsWith("/"));
    if (pathRows.length === 1) return projectRowPath(pathRows[0]);
    return "";
  }

  function currentProjectRepoPath() {
    return currentProjectRepoPathFromSelectedProjectButton()
      || currentProjectRepoPathFromStartButton()
      || currentProjectRepoPathFromExpandedRows();
  }

  function currentProjectContext() {
    const stored = currentProjectContextFromStoredSelection();
    if (stored) return stored;
    const selectedPath = currentProjectRepoPathFromSelectedProjectButton();
    if (selectedPath) return { repoPath: selectedPath, projectId: "", label: displayProjectName(selectedPath), at: Date.now() };
    const startContext = currentProjectContextFromStartButton();
    if (startContext) return startContext;
    const expandedPath = currentProjectRepoPathFromExpandedRows();
    if (expandedPath) return { repoPath: expandedPath, projectId: "", label: displayProjectName(expandedPath), at: Date.now() };
    return null;
  }

  function newWorktreeModeActive() {
    return [...document.querySelectorAll('button, [role="button"]')]
      .filter((node) => visibleElement(node))
      .some((node) => {
        return normalizedElementText(node) === "New worktree";
      });
  }

  function normalizedElementText(node) {
    return (node?.innerText || node?.textContent || "").replace(/\s+/g, " ").trim();
  }

  function codexMenuLocalizationScopeSelector() {
    return [
      "[role='menu']",
      "[role='dialog']",
      "[role='listbox']",
      "[cmdk-list]",
      "[data-radix-menu-content]",
      "[data-radix-popper-content-wrapper]",
      "[data-testid='app-shell-header-context-menu-surface']",
      "[data-codex-keyboard-shortcuts]",
      "[class*='command']",
      "[class*='Command']",
      "[class*='shortcut']",
      "[class*='Shortcut']",
    ].join(", ");
  }

  function codexMenuLocalizationRoot() {
    return document.body || document.documentElement;
  }

  function shouldLocalizeCodexMenuNode(node) {
    if (!node || node.nodeType !== Node.TEXT_NODE || !node.nodeValue) return false;
    const parent = node.parentElement;
    if (!parent || isExtensionUiNode(parent)) return false;
    if (parent.closest?.("textarea, input, [contenteditable='true'], [data-message-author-role], [data-testid='conversation-turn'], main .prose")) return false;
    return !!parent.closest?.(codexMenuLocalizationScopeSelector());
  }

  function localizeCodexMenuTextNode(node) {
    if (!shouldLocalizeCodexMenuNode(node)) return false;
    const original = node.nodeValue;
    const leading = original.match(/^\s*/)?.[0] || "";
    const trailing = original.match(/\s*$/)?.[0] || "";
    const normalized = original.replace(/\s+/g, " ").trim();
    const localized = codexMenuLocalizationMap.get(normalized);
    if (!localized) return false;
    const next = `${leading}${localized}${trailing}`;
    if (next === original) return false;
    node.nodeValue = next;
    return true;
  }

  function localizeCodexMenuAttributes(root) {
    if (!root?.querySelectorAll) return false;
    let changed = false;
    const selector = "button[aria-label], [role='menuitem'][aria-label], [title], [placeholder]";
    root.querySelectorAll(selector).forEach((element) => {
      if (isExtensionUiNode(element)) return;
      if (element.closest?.("textarea, input, [contenteditable='true'], [data-message-author-role], [data-testid='conversation-turn'], main .prose")) return;
      if (!element.closest?.(codexMenuLocalizationScopeSelector())) return;
      for (const attribute of ["aria-label", "title", "placeholder"]) {
        const value = element.getAttribute(attribute);
        const localized = codexMenuLocalizationMap.get((value || "").replace(/\s+/g, " ").trim());
        if (localized && localized !== value) {
          element.setAttribute(attribute, localized);
          changed = true;
        }
      }
    });
    return changed;
  }

  function localizeCodexMenus(root = codexMenuLocalizationRoot()) {
    if (!root) return false;
    let changed = false;
    const scopes = [];
    if (root.nodeType === 1 && root.matches?.(codexMenuLocalizationScopeSelector())) scopes.push(root);
    root.querySelectorAll?.(codexMenuLocalizationScopeSelector()).forEach((scope) => scopes.push(scope));
    for (const scope of scopes.slice(0, 80)) {
      if (!(scope instanceof HTMLElement) || isExtensionUiNode(scope)) continue;
      const walker = document.createTreeWalker(scope, NodeFilter.SHOW_TEXT);
      let node;
      while ((node = walker.nextNode())) {
        if (localizeCodexMenuTextNode(node)) changed = true;
      }
      if (localizeCodexMenuAttributes(scope)) changed = true;
      scope.dataset.codexMenuLocalizationVersion = codexMenuLocalizationVersion;
    }
    return changed;
  }

  async function loadUpstreamBranchDefaults(context) {
    const repoPath = typeof context === "string" ? context : context?.repoPath || "";
    const projectId = typeof context === "string" ? "" : context?.projectId || "";
    if (!repoPath && !projectId) return null;
    const cacheKey = projectId ? `project:${projectId}` : `repo:${repoPath}`;
    const cacheTtlMs = projectId ? upstreamRemoteBranchDefaultsCacheTtlMs : upstreamBranchDefaultsCacheTtlMs;
    const cached = upstreamBranchDefaultsCache.get(cacheKey);
    if (cached && Date.now() - cached.loadedAt < cacheTtlMs) return cached;
    const inflight = upstreamBranchDefaultsInflight.get(cacheKey);
    if (inflight) return inflight;
    const request = postJson("/upstream-worktree/defaults", { repoPath, projectId })
      .then((result) => {
        const entry = { repoPath, projectId, result, loadedAt: Date.now() };
        if (result?.status === "ok") upstreamBranchDefaultsCache.set(cacheKey, entry);
        return entry;
      })
      .finally(() => upstreamBranchDefaultsInflight.delete(cacheKey));
    upstreamBranchDefaultsInflight.set(cacheKey, request);
    return request;
  }

  function renderUpstreamBranchOption(menu, context, ref) {
    const repoPath = context?.repoPath || "";
    const label = ref.label || `${ref.remote || "upstream"}/${ref.branch || "main"}`;
    const item = document.createElement("div");
    item.setAttribute("role", "menuitem");
    item.setAttribute("aria-checked", "false");
    item.setAttribute(upstreamBranchOptionAttribute, "true");
    item.setAttribute("data-repo-path", repoPath);
    item.setAttribute("data-project-id", context?.projectId || "");
    item.setAttribute("data-remote", ref.remote || "upstream");
    item.setAttribute("data-base-branch", ref.branch || "main");
    item.setAttribute("data-label", label);
    item.className = "codex-upstream-branch-option cursor-interaction flex items-center gap-2 rounded-sm px-2 py-1.5 text-sm text-token-foreground hover:bg-token-list-hover-background";
    item.innerHTML = `${branchIconSvg()}<span class="min-w-0 flex-1 truncate">${escapeHtml(label)}</span>${checkmarkSvg()}`;
    menu.appendChild(item);
  }

  function branchIconSvg() {
    return '<svg aria-hidden="true" data-codex-upstream-branch-icon="true" viewBox="0 0 24 24" class="h-4 w-4 shrink-0 text-token-text-tertiary" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="6" x2="6" y1="3" y2="15"></line><circle cx="18" cy="6" r="3"></circle><circle cx="6" cy="18" r="3"></circle><path d="M18 9a9 9 0 0 1-9 9"></path></svg>';
  }

  function checkmarkSvg() {
    return '<svg hidden aria-hidden="true" data-codex-upstream-branch-check="true" viewBox="0 0 24 24" class="h-4 w-4 shrink-0 text-token-text-secondary" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"></path></svg>';
  }

  function branchMenuItems(menu) {
    return [...menu.querySelectorAll('[role="menuitem"], [data-radix-collection-item]')]
      .filter((item) => !item.closest?.(`[${upstreamBranchOptionAttribute}]`));
  }

  function branchMenuItemLabel(menuItem) {
    return normalizedElementText(menuItem);
  }

  function upstreamBranchOptionLabel(option) {
    return option?.getAttribute?.("data-label") || normalizedElementText(option);
  }

  function worktreeBranchMap(defaultsResult) {
    const repoRoot = defaultsResult?.repoRoot || "";
    const entries = Array.isArray(defaultsResult?.worktreeBranches) ? defaultsResult.worktreeBranches : [];
    return new Map(entries
      .filter((entry) => entry?.branch && entry?.path && entry.path !== repoRoot)
      .map((entry) => [entry.branch, entry.path]));
  }

  function annotateBranchMenuWorktreeUsage(menu, defaultsResult) {
    const usedBranches = worktreeBranchMap(defaultsResult);
    for (const item of branchMenuItems(menu)) {
      item.removeAttribute(branchWorktreePathAttribute);
      item.removeAttribute("title");
      const worktreePath = usedBranches.get(branchMenuItemLabel(item));
      if (!worktreePath) continue;
      item.setAttribute(branchWorktreePathAttribute, worktreePath);
      item.setAttribute("title", t("upstreamBranchInUse", { path: worktreePath }));
    }
  }

  function branchWorktreePathFromMenuItem(menuItem) {
    const annotatedPath = menuItem?.getAttribute?.(branchWorktreePathAttribute) || "";
    if (annotatedPath) return annotatedPath;
    const menu = menuItem?.closest?.('[role="menu"], [data-radix-menu-content]');
    const context = currentProjectContextForBranchMenu(menu);
    const cacheKey = context?.projectId ? `project:${context.projectId}` : `repo:${context?.repoPath || ""}`;
    const usedBranches = worktreeBranchMap(upstreamBranchDefaultsCache.get(cacheKey)?.result);
    return usedBranches.get(branchMenuItemLabel(menuItem)) || "";
  }

  function upstreamBranchOptionsMatchRefs(menu, context, refs) {
    const repoPath = context?.repoPath || "";
    const projectId = context?.projectId || "";
    const options = [...menu.querySelectorAll(`[${upstreamBranchOptionAttribute}]`)];
    if (options.length !== refs.length) return false;
    return options.every((option, index) => {
      const ref = refs[index];
      return option.getAttribute("data-repo-path") === repoPath
        && option.getAttribute("data-project-id") === projectId
        && option.getAttribute("data-remote") === (ref.remote || "upstream")
        && option.getAttribute("data-base-branch") === (ref.branch || "main")
        && upstreamBranchOptionLabel(option) === (ref.label || `${ref.remote || "upstream"}/${ref.branch || "main"}`);
    });
  }

  function syncUpstreamBranchMenuSelection(menu) {
    if (!menu) return;
    const selection = readUpstreamBranchSelection();
    for (const option of menu.querySelectorAll(`[${upstreamBranchOptionAttribute}]`)) {
      const selected = !!selection
        && option.getAttribute("data-repo-path") === (selection.repoPath || "")
        && option.getAttribute("data-project-id") === (selection.projectId || "")
        && option.getAttribute("data-remote") === (selection.remote || "upstream")
        && option.getAttribute("data-base-branch") === (selection.baseBranch || "main");
      option.setAttribute("aria-checked", selected ? "true" : "false");
      option.toggleAttribute("data-selected", selected);
      const check = option.querySelector('[data-codex-upstream-branch-check="true"]');
      if (check && selected) check.removeAttribute("hidden");
      if (check && !selected) check.setAttribute("hidden", "");
    }
  }

  function removeUpstreamBranchOptions(scope = document) {
    scope.querySelectorAll(`[${upstreamBranchOptionAttribute}], .codex-upstream-branch-group`)
      .forEach((node) => node.remove());
  }

  function cleanupInvalidUpstreamBranchOptions() {
    for (const menu of nativeBranchMenuCandidates()) {
      if (!menu.querySelector(`[${upstreamBranchOptionAttribute}], .codex-upstream-branch-group`)) continue;
      const trigger = branchMenuTriggerFromMenu(menu);
      if (!looksLikeBranchMenu(menu, trigger) || !branchMenuInNewWorktreeMode(trigger)) {
        removeUpstreamBranchOptions(menu);
      }
    }
  }

  function branchMenuTriggerFromMenu(menu) {
    const labelledBy = menu?.getAttribute?.("aria-labelledby") || "";
    if (labelledBy) {
      const trigger = document.getElementById(labelledBy);
      if (trigger instanceof Element) return trigger;
    }
    return [...document.querySelectorAll('button')]
      .filter((button) => (button.innerText || button.textContent || "").trim() === "main")
      .sort((left, right) => right.getBoundingClientRect().x - left.getBoundingClientRect().x)[0] || null;
  }

  function branchMenuTriggerIsBranchControl(trigger) {
    const text = normalizedElementText(trigger);
    if (!text || /^(work locally|new worktree|cloud|no environment)$/i.test(text)) return false;
    const rect = effectiveElementRect(trigger);
    const footer = trigger?.closest?.(".composer-footer");
    if (!rect || !footer) return /branch|main|create branch/i.test(text);
    const modeTrigger = [...footer.querySelectorAll('button, [role="button"]')]
      .filter((node) => node !== trigger && visibleElement(node))
      .filter((node) => node.getBoundingClientRect().x < rect.x)
      .sort((left, right) => right.getBoundingClientRect().x - left.getBoundingClientRect().x)
      .find((node) => /^(work locally|new worktree|cloud)$/i.test(normalizedElementText(node)));
    return !!modeTrigger;
  }

  function branchMenuInNewWorktreeMode(trigger) {
    if (!trigger) return newWorktreeModeActive();
    const footer = trigger.closest?.(".composer-footer");
    const scope = footer || trigger.parentElement || document;
    const triggerRect = effectiveElementRect(trigger);
    if (!triggerRect) return false;
    const modeTrigger = [...scope.querySelectorAll('button, [role="button"]')]
      .filter((node) => node !== trigger && visibleElement(node))
      .filter((node) => node.getBoundingClientRect().x < triggerRect.x)
      .sort((left, right) => right.getBoundingClientRect().x - left.getBoundingClientRect().x)
      .find((node) => /worktree|work locally/i.test(normalizedElementText(node)));
    return normalizedElementText(modeTrigger) === "New worktree";
  }

  function branchTriggerLabelNode(trigger) {
    if (!trigger) return null;
    const nodes = [...trigger.querySelectorAll("span, div")]
      .filter((node) => (node.innerText || node.textContent || "").trim());
    return nodes.find((node) => node.classList?.contains("composer-footer__label--sm")) || nodes[0] || trigger;
  }

  function ensureNativeBranchTriggerLabel(trigger) {
    if (!trigger || trigger.querySelector?.('[data-codex-upstream-branch-selection-label="true"]')) return;
    const labelNode = branchTriggerLabelNode(trigger);
    if (!labelNode) return;
    trigger.setAttribute("data-codex-upstream-branch-trigger", "true");
    labelNode.setAttribute("data-codex-native-branch-label", "true");
    const selectionLabel = document.createElement("span");
    selectionLabel.setAttribute("data-codex-upstream-branch-selection-label", "true");
    selectionLabel.className = labelNode.className || "composer-footer__label--sm composer-footer__secondary-label max-w-40 truncate";
    selectionLabel.hidden = true;
    labelNode.insertAdjacentElement("afterend", selectionLabel);
  }

  function clearUpstreamBranchTriggerLabel() {
    document.querySelectorAll('[data-codex-upstream-branch-trigger="true"]').forEach((trigger) => {
      const nativeLabel = trigger.querySelector('[data-codex-native-branch-label="true"]');
      const selectionLabel = trigger.querySelector('[data-codex-upstream-branch-selection-label="true"]');
      if (nativeLabel) nativeLabel.hidden = false;
      if (selectionLabel) selectionLabel.hidden = true;
      trigger.removeAttribute("aria-label");
      trigger.removeAttribute("title");
    });
  }

  function syncUpstreamBranchTriggerLabel() {
    const selection = readUpstreamBranchSelection();
    if (!selection?.label) {
      clearUpstreamBranchTriggerLabel();
      return;
    }
    document.querySelectorAll('[data-codex-upstream-branch-trigger="true"]').forEach((trigger) => {
      const nativeLabel = trigger.querySelector('[data-codex-native-branch-label="true"]');
      const selectionLabel = trigger.querySelector('[data-codex-upstream-branch-selection-label="true"]');
      if (!selectionLabel) return;
      if (nativeLabel) nativeLabel.hidden = true;
      selectionLabel.hidden = false;
      selectionLabel.textContent = selection.label;
      trigger.setAttribute("aria-label", selection.label);
      trigger.setAttribute("title", selection.label);
    });
  }

  function handleNativeBranchSelection(event) {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    const menuItem = target?.closest?.('[role="menuitem"], [data-radix-collection-item]');
    if (!menuItem || menuItem.closest?.(`[${upstreamBranchOptionAttribute}]`)) return;
    const menu = menuItem.closest?.('[role="menu"], [data-radix-menu-content]');
    if (!menu || !looksLikeBranchMenu(menu)) return;
    const text = (menuItem.innerText || menuItem.textContent || "").replace(/\s+/g, " ").trim();
    if (!text || /^branches$/i.test(text) || /^upstream$/i.test(text) || text === readUpstreamBranchSelection()?.label) return;
    const usedWorktreePath = branchWorktreePathFromMenuItem(menuItem);
    writeUpstreamBranchSelection(null);
    clearUpstreamBranchTriggerLabel();
    syncUpstreamBranchMenuSelection(menu);
    if (usedWorktreePath) {
      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation?.();
      showToast(t("upstreamBranchInUse", { path: usedWorktreePath }), null);
    }
  }

  async function injectUpstreamBranchOptions() {
    if (!codexPlusSettings().upstreamWorktreeCreate) {
      removeUpstreamBranchOptions();
      return;
    }
    cleanupInvalidUpstreamBranchOptions();
    for (const menu of nativeBranchMenuCandidates()) {
      const trigger = branchMenuTriggerFromMenu(menu);
      if (!looksLikeBranchMenu(menu, trigger)) continue;
      const context = currentProjectContextForBranchMenu(menu, trigger);
      if (!context?.repoPath && !context?.projectId) {
        removeUpstreamBranchOptions(menu);
        continue;
      }
      const defaults = await loadUpstreamBranchDefaults(context);
      const defaultsResult = defaults?.result;
      const refs = defaults?.result?.upstreamRefs || [];
      annotateBranchMenuWorktreeUsage(menu, defaultsResult);
      if (!branchMenuInNewWorktreeMode(trigger)) {
        removeUpstreamBranchOptions(menu);
        writeUpstreamBranchSelection(null);
        clearUpstreamBranchTriggerLabel();
        continue;
      }
      if (!refs.length) {
        removeUpstreamBranchOptions(menu);
        continue;
      }
      const resolvedContext = {
        repoPath: defaults?.repoPath || context.repoPath || defaultsResult?.repoRoot || "",
        projectId: defaults?.projectId || context.projectId || "",
      };
      if (upstreamBranchOptionsMatchRefs(menu, resolvedContext, refs)) {
        syncUpstreamBranchTriggerLabel();
        syncUpstreamBranchMenuSelection(menu);
        continue;
      }
      removeUpstreamBranchOptions(menu);
      ensureNativeBranchTriggerLabel(trigger);
      const group = document.createElement("div");
      group.className = "codex-upstream-branch-group px-2 py-1 text-xs text-token-text-tertiary";
      group.textContent = "Upstream";
      menu.appendChild(group);
      refs.forEach((ref) => renderUpstreamBranchOption(menu, resolvedContext, ref));
      syncUpstreamBranchTriggerLabel();
      syncUpstreamBranchMenuSelection(menu);
    }
  }

  function installUpstreamBranchDropdownAdapter() {
    const adapterVersion = "actual-upstream-refs-v16";
    window.__codexUpstreamBranchDropdownAdapterVersion = adapterVersion;
    if (window.__codexUpstreamBranchDropdownAdapterInstalled === adapterVersion) return;
    window.__codexUpstreamBranchDropdownAdapterInstalled = adapterVersion;
    document.addEventListener("click", (event) => {
      rememberStartNewChatProjectContext(event);
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      const option = target?.closest?.(`[${upstreamBranchOptionAttribute}]`);
      if (!option) {
        handleNativeBranchSelection(event);
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      const selection = {
        repoPath: option.getAttribute("data-repo-path") || "",
        projectId: option.getAttribute("data-project-id") || "",
        remote: option.getAttribute("data-remote") || "upstream",
        baseBranch: option.getAttribute("data-base-branch") || "main",
        label: upstreamBranchOptionLabel(option) || "upstream/main",
      };
      writeUpstreamBranchSelection(selection);
      prepareUpstreamBranchSelection(selection);
      syncUpstreamBranchTriggerLabel();
      syncUpstreamBranchMenuSelection(option.closest?.('[role="menu"], [data-radix-menu-content], [cmdk-list]'));
      showToast(t("upstreamBranchWillCreate", { label: upstreamBranchOptionLabel(option) || "upstream/main" }), null);
    }, true);
    let upstreamBranchInjectTimer = null;
    const schedule = () => {
      clearTimeout(upstreamBranchInjectTimer);
      upstreamBranchInjectTimer = setTimeout(() => {
        injectUpstreamBranchOptions().catch((error) => reportDiagnostic("upstream_branch_inject_failed", { error: error?.message || String(error) }));
      }, 80);
    };
    new MutationObserver(schedule).observe(document.body || document.documentElement, { childList: true, subtree: true });
    schedule();
  }

  function upstreamQualifiedSourceRef(selection) {
    if (selection?.qualifiedSourceRef) return selection.qualifiedSourceRef;
    const remote = (selection?.remote || "upstream").trim();
    const baseBranch = (selection?.baseBranch || "main").trim();
    return remote && baseBranch ? `refs/remotes/${remote}/${baseBranch}` : "";
  }

  function prepareUpstreamBranchSelection(selection) {
    if ((!selection?.repoPath && !selection?.projectId) || !selection.remote || !selection.baseBranch) return;
    void postJson("/upstream-worktree/prepare", {
      repoPath: selection.repoPath || "",
      projectId: selection.projectId || "",
      remote: selection.remote,
      baseBranch: selection.baseBranch,
      fetch: true,
    }).then((result) => {
      if (result?.status !== "ok") throw new Error(result?.message || "prepare failed");
      writePreparedUpstreamBranchSelection(selection, result);
    }).catch((error) => {
      sendCodexPlusDiagnostic("upstream_branch_prepare_failed", {
        label: selection.label || "",
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
    });
  }

  function writePreparedUpstreamBranchSelection(selection, result) {
    const current = readUpstreamBranchSelection();
    if (!upstreamSelectionMatches(current, selection)) return;
    writeUpstreamBranchSelection({
      ...current,
      qualifiedSourceRef: result.qualifiedSourceRef || upstreamQualifiedSourceRef(selection),
      sourceHead: result.sourceHead || "",
      preparedAt: Date.now(),
    });
  }

  function upstreamSelectionMatches(left, right) {
    return !!left && !!right
      && (left.repoPath || "") === (right.repoPath || "")
      && (left.projectId || "") === (right.projectId || "")
      && (left.remote || "upstream") === (right.remote || "upstream")
      && (left.baseBranch || "main") === (right.baseBranch || "main");
  }

  function pendingWorktreeRequestMatchesSelection(request, selection) {
    if (!selection || !request || request.launchMode !== "start-conversation") return false;
    const sourceRoot = request.sourceWorkspaceRoot || "";
    if (selection.repoPath && sourceRoot) return sameWorkspacePath(sourceRoot, selection.repoPath);
    if (selection.projectId) return true;
    return !selection.repoPath || sameWorkspacePath(sourceRoot, selection.repoPath);
  }

  function applyUpstreamPendingWorktreeOverride(payload) {
    const selection = readUpstreamBranchSelection();
    const request = payload?.request;
    const sourceRef = upstreamQualifiedSourceRef(selection);
    if (!codexPlusSettings().upstreamWorktreeCreate || !sourceRef) return payload;
    if (!pendingWorktreeRequestMatchesSelection(request, selection)) return payload;
    if (request?.startingState?.type !== "branch") return payload;
    if (request.startingState.branchName === sourceRef) return payload;
    const nextRequest = {
      ...request,
      startingState: { ...request.startingState, branchName: sourceRef },
    };
    prepareUpstreamBranchSelection(selection);
    sendCodexPlusDiagnostic("upstream_pending_worktree_override_applied", {
      label: selection.label || "",
      sourceRef,
      sourceWorkspaceRoot: request.sourceWorkspaceRoot || "",
    });
    return { ...(payload || {}), request: nextRequest };
  }

  function installUpstreamPendingWorktreeDispatcherPatch() {
    const patchVersion = "1";
    if (window.__codexUpstreamPendingWorktreeDispatcherPatch === patchVersion) return;
    const patch = async () => {
      try {
        const module = await loadCodexAppModule("setting-storage-");
        const dispatcherClass = typeof module.v === "function" && String(module.v).includes("dispatchMessage") ? module.v : null;
        const dispatcher = dispatcherClass?.getInstance?.();
        if (!dispatcher || typeof dispatcher.dispatchMessage !== "function") throw new Error("Codex dispatcher unavailable");
        if (!dispatcher.__codexUpstreamWorktreeOriginalDispatchMessage) {
          dispatcher.__codexUpstreamWorktreeOriginalDispatchMessage = dispatcher.dispatchMessage.bind(dispatcher);
          dispatcher.dispatchMessage = (type, payload) => {
            const nextPayload = type === "pending-worktree-create"
              ? applyUpstreamPendingWorktreeOverride(payload)
              : payload;
            return dispatcher.__codexUpstreamWorktreeOriginalDispatchMessage(type, nextPayload);
          };
        }
        window.__codexUpstreamPendingWorktreeDispatcherPatch = patchVersion;
      } catch (error) {
        sendCodexPlusDiagnostic("upstream_pending_worktree_patch_failed", {
          errorName: error?.name || "",
          errorMessage: error?.message || String(error),
        });
      }
    };
    void patch();
  }

  function upstreamWorktreeNativePayloadFromElement(element) {
    const trigger = element?.closest?.("[data-codex-worktree-create], [data-worktree-create]") || element;
    const scopes = [
      trigger,
      trigger?.closest?.("form"),
      trigger?.closest?.("dialog, [role='dialog']"),
    ].filter((scope, index, all) => scope?.querySelector && all.indexOf(scope) === index);
    if (!scopes.length) return null;
    const valueFrom = (selectors) => {
      for (const scope of scopes) {
        for (const selector of selectors) {
          const node = scope.matches?.(selector) ? scope : scope.querySelector(selector);
          const dataAttribute = selector.match(/^\[([a-z0-9-]+)\]$/i)?.[1] || "";
          const value = node?.value || node?.getAttribute?.(dataAttribute) || node?.getAttribute?.("data-value") || node?.textContent || "";
          if (String(value).trim()) return String(value).trim();
        }
      }
      return "";
    };
    const repoPath = valueFrom(["[data-repo-path]", "[name='repoPath']", "[name='repo']"]);
    const branchName = valueFrom(["[data-branch-name]", "[name='branchName']", "[name='branch']"]);
    const worktreePath = valueFrom(["[data-worktree-path]", "[name='worktreePath']", "[name='path']"]);
    const remote = valueFrom(["[data-remote]", "[name='remote']"]) || "upstream";
    const baseBranch = valueFrom(["[data-base-branch]", "[name='baseBranch']", "[name='base']"]) || "main";
    if (!repoPath || !branchName || !worktreePath || !remote || !baseBranch) return null;
    return { repoPath, branchName, worktreePath, remote, baseBranch, fetch: true };
  }

  function upstreamWorktreePayloadFromSelection(trigger) {
    const selection = readUpstreamBranchSelection();
    if ((!selection?.repoPath && !selection?.projectId) || !selection?.remote || !selection?.baseBranch) return null;
    const nativePayload = upstreamWorktreeNativePayloadFromElement(trigger);
    if (!nativePayload?.branchName || !nativePayload?.worktreePath) return null;
    return {
      ...nativePayload,
      repoPath: selection.repoPath,
      projectId: selection.projectId || "",
      remote: selection.remote,
      baseBranch: selection.baseBranch,
      fetch: true,
    };
  }

  async function handleUpstreamWorktreeNativeCreate(event) {
    if (!codexPlusSettings().upstreamWorktreeCreate) return false;
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    const trigger = target?.closest?.("[data-codex-worktree-create], [data-worktree-create]");
    if (!trigger) return false;
    const payload = upstreamWorktreePayloadFromSelection(trigger) || upstreamWorktreeNativePayloadFromElement(trigger);
    if (!payload) {
      showToast(t("upstreamNativeFormUnknown"), null);
      return false;
    }
    event.preventDefault();
    event.stopPropagation();
    try {
      const result = await postJson("/upstream-worktree/create", payload);
      if (result?.status === "ok") {
        writeUpstreamBranchSelection(null);
        syncUpstreamBranchTriggerLabel();
        showToast(t("upstreamCreatedNativeToast", { sourceRef: result.sourceRef || "" }), null);
      } else {
        showToast(result?.message || t("upstreamCreateFailed"), null);
      }
    } catch (error) {
      showToast(error?.message || t("upstreamCreateFailed"), null);
    }
    return true;
  }

  function installUpstreamWorktreeNativeAdapter() {
    const adapterVersion = "2";
    installUpstreamPendingWorktreeDispatcherPatch();
    if (window.__codexUpstreamWorktreeNativeAdapterInstalled === adapterVersion) return;
    window.__codexUpstreamWorktreeNativeAdapterInstalled = adapterVersion;
    document.addEventListener("click", (event) => {
      handleUpstreamWorktreeNativeCreate(event);
    }, true);
  }

  function setUpstreamWorktreeMessage(dialog, message, status = "idle") {
    const messageNode = dialog.querySelector("[data-codex-upstream-worktree-message]");
    if (!messageNode) return;
    messageNode.dataset.status = status;
    messageNode.textContent = message || "";
  }

  async function loadUpstreamWorktreeDefaults(dialog) {
    const repoPath = upstreamWorktreeField(dialog, "repoPath")?.value?.trim() || "";
    if (!repoPath) {
      setUpstreamWorktreeMessage(dialog, t("upstreamDefaultsHint"), "idle");
      return;
    }
    setUpstreamWorktreeMessage(dialog, t("upstreamDefaultsReading"), "loading");
    try {
      const result = await postJson("/upstream-worktree/defaults", { repoPath });
      if (result?.status !== "ok") {
        setUpstreamWorktreeMessage(dialog, result?.message || t("upstreamDefaultsFailed"), "failed");
        return;
      }
      const remote = upstreamWorktreeField(dialog, "remote");
      const baseBranch = upstreamWorktreeField(dialog, "baseBranch");
      if (remote && !remote.value) remote.value = result.defaultRemote || "upstream";
      if (baseBranch && (!baseBranch.value || baseBranch.value === "main")) baseBranch.value = result.defaultBaseBranch || "main";
      setUpstreamWorktreeMessage(dialog, t("upstreamDefaultsReady", { remote: remote?.value || "upstream", baseBranch: baseBranch?.value || "main" }), "ok");
    } catch (error) {
      setUpstreamWorktreeMessage(dialog, error?.message || t("upstreamDefaultsFailed"), "failed");
    }
  }

  async function submitUpstreamWorktree(dialog) {
    const payload = upstreamWorktreePayload(dialog);
    if (!payload.repoPath || !payload.branchName || !payload.worktreePath || !payload.remote || !payload.baseBranch) {
      setUpstreamWorktreeMessage(dialog, t("upstreamCreateMissingFields"), "failed");
      return;
    }
    setUpstreamWorktreeMessage(dialog, t("upstreamCreating"), "loading");
    try {
      const result = await postJson("/upstream-worktree/create", payload);
      if (result?.status === "ok") {
        setUpstreamWorktreeMessage(dialog, t("upstreamCreatedMessage", { sourceRef: result.sourceRef || "", worktreePath: result.worktreePath || "" }), "ok");
        showToast(t("upstreamCreatedToast", { branchName: result.branchName || "" }), null);
      } else {
        setUpstreamWorktreeMessage(dialog, result?.message || t("upstreamCreateFailed"), "failed");
      }
    } catch (error) {
      setUpstreamWorktreeMessage(dialog, error?.message || t("upstreamCreateFailed"), "failed");
    }
  }

  function openUpstreamWorktreeDialog() {
    document.querySelectorAll(`.${upstreamWorktreeDialogClass}`).forEach((node) => {
      if (typeof node.__codexUpstreamWorktreeClose === "function") node.__codexUpstreamWorktreeClose();
      else node.remove();
    });
    const overlay = document.createElement("div");
    overlay.className = `codex-delete-confirm-overlay ${upstreamWorktreeDialogClass}`;
    overlay.innerHTML = `
      <div class="codex-delete-confirm-content" role="dialog" aria-modal="true" aria-label="${codexPlusEscapedText("upstreamWorktreeDialogLabel")}">
        <div class="codex-delete-confirm-title">${codexPlusEscapedText("upstreamWorktreeDialogTitle")}</div>
        <div class="codex-delete-confirm-message">${codexPlusEscapedText("upstreamWorktreeDialogDescription")}</div>
        <label class="codex-plus-form-field">${codexPlusEscapedText("upstreamRepoPathLabel")}<input data-codex-upstream-worktree-field="repoPath" type="text" placeholder="/path/to/repo"></label>
        <label class="codex-plus-form-field">${codexPlusEscapedText("upstreamBranchNameLabel")}<input data-codex-upstream-worktree-field="branchName" type="text" placeholder="feature/my-task"></label>
        <label class="codex-plus-form-field">${codexPlusEscapedText("upstreamWorktreePathLabel")}<input data-codex-upstream-worktree-field="worktreePath" type="text" placeholder="/path/to/worktrees/my-task"></label>
        <label class="codex-plus-form-field">${codexPlusEscapedText("upstreamRemoteLabel")}<input data-codex-upstream-worktree-field="remote" type="text" value="upstream"></label>
        <label class="codex-plus-form-field">${codexPlusEscapedText("upstreamBaseBranchLabel")}<input data-codex-upstream-worktree-field="baseBranch" type="text" value="main"></label>
        <div class="codex-plus-form-message" data-codex-upstream-worktree-message>${codexPlusEscapedText("upstreamDefaultsHint")}</div>
        <div class="codex-delete-confirm-actions">
          <button type="button" data-codex-upstream-worktree-cancel="true">${codexPlusEscapedText("cancel")}</button>
          <button type="button" data-codex-upstream-worktree-defaults="true">${codexPlusEscapedText("upstreamReadDefaults")}</button>
          <button type="button" data-codex-upstream-worktree-submit="true">${codexPlusEscapedText("upstreamWorktreeDialogTitle")}</button>
        </div>
      </div>
    `;
    let closed = false;
    const close = (event) => {
      event?.preventDefault?.();
      event?.stopPropagation?.();
      if (closed) return;
      closed = true;
      document.removeEventListener("keydown", handleKeydown, true);
      overlay.remove();
    };
    const handleKeydown = (event) => {
      if (event.key !== "Escape") return;
      close(event);
    };
    overlay.__codexUpstreamWorktreeClose = close;
    document.addEventListener("keydown", handleKeydown, true);
    overlay.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      if (event.target === overlay || target?.closest("[data-codex-upstream-worktree-cancel]")) {
        close(event);
        return;
      }
      if (target?.closest("[data-codex-upstream-worktree-defaults]")) {
        loadUpstreamWorktreeDefaults(overlay);
        return;
      }
      if (target?.closest("[data-codex-upstream-worktree-submit]")) {
        submitUpstreamWorktree(overlay);
      }
    }, true);
    upstreamWorktreeField(overlay, "repoPath")?.addEventListener("change", () => loadUpstreamWorktreeDefaults(overlay));
    document.body.appendChild(overlay);
    upstreamWorktreeField(overlay, "repoPath")?.focus();
  }

  function escapeHtml(value) {
    return String(value)
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#39;");
  }

  function confirmDelete(title) {
    document.querySelectorAll(".codex-delete-confirm-overlay").forEach((node) => node.remove());
    return new Promise((resolve) => {
      const overlay = document.createElement("div");
      overlay.className = "codex-delete-confirm-overlay";
      overlay.innerHTML = `
        <div class="codex-delete-confirm-content" role="dialog" aria-modal="true" aria-label="${codexPlusEscapedText("deleteSessionDialogLabel")}">
          <div class="codex-delete-confirm-title">${codexPlusEscapedText("deleteSessionTitle")}</div>
          <div class="codex-delete-confirm-message">${codexPlusEscapedText("deleteSessionMessage", { title })}</div>
          <div class="codex-delete-confirm-actions">
            <button type="button" data-codex-delete-cancel="true">${codexPlusEscapedText("cancel")}</button>
            <button type="button" data-codex-delete-confirm="true">${codexPlusEscapedText("delete")}</button>
          </div>
        </div>
      `;
      const finish = (value, event) => {
        event?.preventDefault();
        event?.stopPropagation();
        event?.target?.blur?.();
        overlay.remove();
        resolve(value);
      };
      overlay.addEventListener("click", (event) => {
        if (event.target === overlay || event.target.closest("[data-codex-delete-cancel]")) {
          finish(false, event);
          return;
        }
        if (event.target.closest("[data-codex-delete-confirm]")) {
          finish(true, event);
        }
      }, true);
      overlay.addEventListener("keydown", (event) => {
        if (event.key === "Escape") finish(false, event);
      }, true);
      document.body.appendChild(overlay);
      overlay.querySelector("[data-codex-delete-cancel]")?.focus();
    });
  }

  function rowHref(row) {
    return row.getAttribute("href") || row.querySelector("a")?.getAttribute("href") || "";
  }

  function isCurrentSessionRow(row, ref) {
    if (row.getAttribute("aria-current") === "page" || row.getAttribute("aria-current") === "true") return true;
    const href = rowHref(row);
    if (href) {
      try {
        const url = new URL(href, window.location.href);
        if (url.href === window.location.href || url.pathname === window.location.pathname) return true;
      } catch {
        if (window.location.href.includes(href)) return true;
      }
    }
    return !!ref.session_id && window.location.href.includes(ref.session_id);
  }

  function releaseDeleteFocus(row, button) {
    button.blur();
    if (row.contains(document.activeElement)) {
      document.activeElement.blur();
    }
  }

  function removeDeletedRow(row, button, ref) {
    releaseDeleteFocus(row, button);
    const shouldReload = isCurrentSessionRow(row, ref);
    row.remove();
    if (shouldReload) {
      window.location.reload();
    }
  }

  function updateDeleteButtonOffsets() {
    sessionRows().forEach((row) => {
      const hasArchiveConfirm = Array.from(row.querySelectorAll("button")).some((button) => {
        const rect = button.getBoundingClientRect();
        const label = button.getAttribute("aria-label") || "";
        const text = (button.textContent || "").trim();
        if (button.classList.contains(buttonClass) || button.classList.contains(exportButtonClass) || label === "归档对话" || label === "置顶对话") return false;
        return text === "确认" || (text.length > 0 && rect.width > 0 && rect.width <= 36 && rect.x > row.getBoundingClientRect().right - 50);
      });
      row.classList.toggle("codex-archive-confirm-visible", hasArchiveConfirm);
    });
  }

  function openDeleteConfirmForRow(row, button, ref, event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    releaseDeleteFocus(row, button);
    confirmDelete(ref.title).then(async (confirmed) => {
      if (!confirmed) return;
      releaseDeleteFocus(row, button);
      const result = await postJson("/delete", ref);
      if (result.status === "server_deleted" || result.status === "local_deleted") {
        removeDeletedRow(row, button, ref);
        showToast(result.message || t("deleteSuccess"), result.undo_token);
      } else {
        showToast(result.message || t("deleteFailed"), null);
      }
    });
  }

  async function exportMarkdown(ref) {
    const result = await postJson("/export-markdown", ref);
    if (result.status === "exported" && result.filename && typeof result.markdown === "string") {
      const saveResult = await saveMarkdown(result.filename, result.markdown);
      if (saveResult?.status === "cancelled") {
        showToast(saveResult.message || t("exportCancelled"), null);
      } else {
        showToast(result.message || t("exportSuccess"), null);
      }
      return;
    }
    showToast(result.message || t("exportFailed"), null);
  }

  function sortStateFromMoveResult(result, ref, row) {
    const trustedSortMs = timestampMsFromPayload(result);
    return { sortMs: trustedSortMs || rowSortMs(row, ref), sortMsTrusted: !!trustedSortMs };
  }

  function finishProjectMove(row, button, ref, target, message) {
    releaseDeleteFocus(row, button);
    button.disabled = false;
    button.textContent = t("moveLabel");
    saveProjectMoveProjection(ref, target, target.sortMs || rowSortMs(row, ref, target));
    if (target.kind === "projectless") moveRowToChats(row, target);
    refreshAfterProjectMove();
    showToast(message, null);
  }

  async function applyProjectMove(row, button, ref, target) {
    button.disabled = true;
    button.textContent = t("movingLabel");
    try {
      if (target.kind === "projectless") {
        const result = await moveSessionToProjectless(ref);
        finishProjectMove(row, button, ref, { ...target, ...sortStateFromMoveResult(result, ref, row) }, t("moveToChatsSuccess", { title: ref.title || ref.session_id }));
      } else {
        const result = await moveSessionToProject(ref, target);
        finishProjectMove(row, button, ref, { ...target, ...sortStateFromMoveResult(result, ref, row) }, t("moveToProjectSuccess", { project: target.label, title: ref.title || ref.session_id }));
      }
    } catch (error) {
      button.disabled = false;
      button.textContent = t("moveLabel");
      showToast(t("moveFailed", { message: error?.message || error }), null);
    }
  }

  async function openProjectMoveMenuForRow(row, button, ref, event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    releaseDeleteFocus(row, button);
    document.querySelectorAll(`.${projectMoveOverlayClass}`).forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = projectMoveOverlayClass;
    overlay.innerHTML = `
      <div class="codex-project-move-panel" role="dialog" aria-modal="true" aria-label="${codexPlusEscapedText("moveDialogLabel")}">
        <div class="codex-project-move-header">
          <div class="codex-project-move-title">${codexPlusEscapedText("moveDialogTitle", { title: ref.title || ref.session_id })}</div>
        </div>
        <div class="codex-project-move-list"><div class="codex-project-move-empty">${codexPlusEscapedText("moveTargetsLoading")}</div></div>
      </div>
    `;
    const panel = overlay.querySelector(".codex-project-move-panel");
    const rect = button.getBoundingClientRect();
    const panelWidth = Math.min(360, Math.max(240, window.innerWidth - 32));
    panel.style.left = `${Math.max(16, Math.min(window.innerWidth - panelWidth - 16, rect.right - panelWidth))}px`;
    panel.style.top = `${Math.max(16, Math.min(window.innerHeight - 120, rect.bottom + 6))}px`;
    const close = () => overlay.remove();
    overlay.addEventListener("click", (clickEvent) => {
      if (clickEvent.target === overlay) close();
    }, true);
    overlay.addEventListener("keydown", (keyEvent) => {
      if (keyEvent.key === "Escape") {
        keyEvent.preventDefault();
        close();
      }
    }, true);
    document.body.appendChild(overlay);
    try {
      const targets = projectMoveTargets();
      const list = overlay.querySelector(".codex-project-move-list");
      if (!list) return;
      list.innerHTML = "";
      if (targets.length === 0) {
        list.innerHTML = `<div class="codex-project-move-empty">${codexPlusEscapedText("moveTargetsEmpty")}</div>`;
        return;
      }
      for (const target of targets) {
        const item = document.createElement("button");
        item.type = "button";
        item.className = "codex-project-move-item";
        item.innerHTML = `
          <div class="codex-project-move-item-title">${escapeHtml(target.label)}</div>
          <div class="codex-project-move-item-path">${escapeHtml(target.description)}</div>
        `;
        item.addEventListener("click", async (selectEvent) => {
          selectEvent.preventDefault();
          selectEvent.stopPropagation();
          close();
          await applyProjectMove(row, button, ref, target);
        }, true);
        list.appendChild(item);
      }
      list.querySelector("button")?.focus();
    } catch (error) {
      close();
      showToast(t("moveTargetsLoadFailed", { message: error?.message || error }), null);
    }
  }

  function installDeleteButtonEventDelegation() {
    document.removeEventListener("pointerup", window.__codexSessionDeleteDocumentDeleteHandler, true);
    document.removeEventListener("click", window.__codexSessionDeleteDocumentDeleteHandler, true);
    const handler = (event) => {
      const button = event.target?.closest?.(`.${buttonClass}`);
      const row = button?.closest?.("[data-app-action-sidebar-thread-id]");
      if (!button || !row) return;
      const ref = sessionRefFromRow(row);
      if (!ref.session_id) return;
      openDeleteConfirmForRow(row, button, ref, event);
    };
    window.__codexSessionDeleteDocumentDeleteHandler = handler;
    document.addEventListener("pointerup", handler, true);
    document.addEventListener("click", handler, true);
  }

  function actionGroupFromRow(row) {
    return row.querySelector(`.${actionGroupClass}`);
  }

  function nativeActionButtonsFromRow(row) {
    return [...row.querySelectorAll('button,[role="button"],a')]
      .filter((node) => !node.closest(`.${actionGroupClass}`))
      .filter((node) => {
        const rect = node.getBoundingClientRect();
        if (rect.width < 12 || rect.height < 12) return false;
        const label = [
          node.getAttribute("aria-label"),
          node.getAttribute("title"),
          node.dataset?.state,
          node.textContent,
        ]
          .filter(Boolean)
          .join(" ")
          .toLowerCase();
        if (/(pin|archive|置顶|归档)/i.test(label)) return true;
        const rowRect = row.getBoundingClientRect();
        return rect.left > rowRect.left + rowRect.width * 0.68;
      });
  }

  function syncActionGroupLayout(row, group) {
    if (!row || !group) return;
    if (group.dataset.codexActionLayoutStable === "true") return;
    const rowRect = row.getBoundingClientRect();
    const nativeButtons = nativeActionButtonsFromRow(row);
    const leftmostNative = nativeButtons
      .map((button) => button.getBoundingClientRect())
      .filter((rect) => rect.width > 0 && rect.height > 0)
      .sort((a, b) => a.left - b.left)[0];
    const gap = 8;
    const fallbackRight = 28;
    const right = leftmostNative
      ? Math.max(fallbackRight, Math.round(rowRect.right - leftmostNative.left + gap))
      : fallbackRight;
    const groupWidth = Math.ceil(group.getBoundingClientRect().width || 96);
    const titleNode = row.querySelector(selectors.threadTitle);
    const titleRect = titleNode?.getBoundingClientRect();
    const titleLeft = titleRect?.left || rowRect.left + 40;
    const maxTitleWidth = Math.max(24, Math.round(rowRect.width - (titleLeft - rowRect.left) - right - groupWidth - 14));
    group.style.setProperty("--codex-session-actions-right", `${right}px`);
    row.style.setProperty("--codex-session-title-mask", `${right + groupWidth + 12}px`);
    row.style.setProperty("--codex-session-title-max-width", `${maxTitleWidth}px`);
    group.dataset.codexActionLayoutStable = "true";
  }

  function syncActionGroupsLayout() {
    sessionRows().forEach((row) => {
      const group = actionGroupFromRow(row);
      if (group) syncActionGroupLayout(row, group);
    });
  }

  function removeActionGroups(row) {
    row.querySelectorAll(`.${actionGroupClass}`).forEach((group) => group.remove());
  }

  function stopActionButtonEvent(row, button, event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    releaseDeleteFocus(row, button);
  }

  function installActionButtonEvents(row, button, onActivate) {
    ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
      button.addEventListener(eventName, (event) => stopActionButtonEvent(row, button, event), true);
    });
    button.addEventListener("pointerenter", () => showActionButtonTooltip(button));
    button.addEventListener("pointerleave", hideActionButtonTooltip);
    button.addEventListener("focus", () => showActionButtonTooltip(button));
    button.addEventListener("blur", hideActionButtonTooltip);
    button.addEventListener("pointerup", onActivate, true);
    button.addEventListener("click", (event) => {
      hideActionButtonTooltip();
      onActivate(event);
    }, true);
  }

  function installMoreButtonEvents(row, button, onActivate) {
    ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
      button.addEventListener(eventName, (event) => stopActionButtonEvent(row, button, event), true);
    });
    button.addEventListener("pointerenter", () => showActionButtonTooltip(button));
    button.addEventListener("pointerleave", hideActionButtonTooltip);
    button.addEventListener("focus", () => showActionButtonTooltip(button));
    button.addEventListener("blur", hideActionButtonTooltip);
    button.addEventListener("pointerup", onActivate, true);
    button.addEventListener("click", (event) => {
      hideActionButtonTooltip();
      stopActionButtonEvent(row, button, event);
    }, true);
  }

  function hideActionButtonTooltip() {
    document.querySelectorAll(`.${actionTooltipClass}`).forEach((node) => node.remove());
  }

  function closeSessionMoreMenus(exceptMenu = null) {
    document.querySelectorAll(`.${moreMenuClass}`).forEach((menu) => {
      if (menu !== exceptMenu) {
        menu.hidden = true;
        menu.closest?.("[data-codex-delete-row]")?.classList.remove("codex-session-more-open");
        menu.__codexSessionMoreRow?.classList?.remove("codex-session-more-open");
      }
    });
  }

  function toggleSessionMoreMenu(row, button, menu) {
    const nextHidden = !menu.hidden;
    closeSessionMoreMenus(menu);
    menu.hidden = nextHidden;
    row.classList.toggle("codex-session-more-open", !menu.hidden);
    button.setAttribute("aria-expanded", String(!menu.hidden));
  }

  function installSessionMoreMenuAutoClose(row, menu) {
    const group = menu.__codexSessionMoreGroup || menu.closest?.(`.${actionGroupClass}`);
    const closeIfOutside = () => {
      window.setTimeout(() => {
        if (menu.hidden) return;
        const active = document.activeElement;
        if (group?.matches?.(":hover") || menu.matches?.(":hover") || menu.contains(active)) return;
        menu.hidden = true;
        row.classList.remove("codex-session-more-open");
        group?.querySelector?.(`.${moreButtonClass}`)?.setAttribute("aria-expanded", "false");
      }, 80);
    };
    group?.addEventListener("pointerleave", closeIfOutside, true);
    menu.addEventListener("pointerleave", closeIfOutside, true);
    menu.addEventListener("focusout", closeIfOutside, true);
  }

  function updateSessionMoreMenuDirection(button, menu) {
    menu.classList.remove("codex-session-more-menu-open-up");
    const buttonRect = button.getBoundingClientRect();
    const estimatedMenuHeight = Math.max(80, menu.getBoundingClientRect().height || 76);
    if (buttonRect.bottom + 30 + estimatedMenuHeight > window.innerHeight - 8) {
      menu.classList.add("codex-session-more-menu-open-up");
    }
  }

  function positionSessionMoreMenu(button, menu) {
    const rect = button.getBoundingClientRect();
    const menuWidth = Math.max(140, menu.getBoundingClientRect().width || 140);
    const left = Math.min(window.innerWidth - menuWidth - 8, Math.max(8, rect.right - menuWidth));
    menu.style.left = `${left}px`;
    menu.style.top = `${Math.max(8, rect.bottom + 4)}px`;
  }

  function createSessionMoreMenuItem(label, icon, onActivate) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "codex-session-more-menu-item";
    item.setAttribute("role", "menuitem");
    item.innerHTML = `<span class="codex-session-more-menu-icon">${icon}</span><span>${escapeHtml(label)}</span>`;
    item.addEventListener("click", onActivate, true);
    return item;
  }

  function showActionButtonTooltip(button) {
    const label = button.dataset.codexActionLabel || button.getAttribute("aria-label") || "";
    if (!label) return;
    hideActionButtonTooltip();
    const tooltip = document.createElement("div");
    tooltip.className = actionTooltipClass;
    tooltip.textContent = label;
    document.body.appendChild(tooltip);
    const buttonRect = button.getBoundingClientRect();
    const tooltipRect = tooltip.getBoundingClientRect();
    const gap = 8;
    const left = Math.min(
      window.innerWidth - tooltipRect.width - 8,
      Math.max(8, buttonRect.left + buttonRect.width / 2 - tooltipRect.width / 2),
    );
    const preferredTop = buttonRect.top - tooltipRect.height - gap;
    const fallbackTop = buttonRect.bottom + gap;
    const top = preferredTop >= 8
      ? preferredTop
      : Math.min(window.innerHeight - tooltipRect.height - 8, fallbackTop);
    tooltip.style.left = `${left}px`;
    tooltip.style.top = `${Math.max(8, top)}px`;
  }

  function refreshActionButton(originalButton, row, onActivate) {
    if (!originalButton.isConnected) return;
    const replacement = originalButton.cloneNode(true);
    installActionButtonEvents(row, replacement, onActivate);
    originalButton.replaceWith(replacement);
    return replacement;
  }

  function configureActionButton(button, label, icon) {
    button.setAttribute("aria-label", label);
    button.dataset.codexActionLabel = label;
    button.removeAttribute("title");
    button.textContent = icon;
  }

  function moreIconSvg() {
    return `
      <svg width="20" height="20" viewBox="0 0 20 20" aria-hidden="true" focusable="false" fill="none" xmlns="http://www.w3.org/2000/svg">
        <circle cx="5" cy="10" r="1.35" fill="currentColor"></circle>
        <circle cx="10" cy="10" r="1.35" fill="currentColor"></circle>
        <circle cx="15" cy="10" r="1.35" fill="currentColor"></circle>
      </svg>
    `;
  }

  function trashIconSvg() {
    return `
      <svg width="20" height="20" viewBox="0 0 20 20" aria-hidden="true" focusable="false" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M7.25 3.5C7.25 3.08579 7.58579 2.75 8 2.75H12C12.4142 2.75 12.75 3.08579 12.75 3.5V4.25H16.25C16.6642 4.25 17 4.58579 17 5C17 5.41421 16.6642 5.75 16.25 5.75H15.556L14.875 15.284C14.7828 16.5746 13.7088 17.575 12.4149 17.575H7.58508C6.29116 17.575 5.2172 16.5746 5.12501 15.284L4.44401 5.75H3.75C3.33579 5.75 3 5.41421 3 5C3 4.58579 3.33579 4.25 3.75 4.25H7.25V3.5ZM8.75 4.25H11.25V4.25H8.75ZM5.948 5.75L6.62119 15.1772C6.65734 15.6833 7.07854 16.075 7.58508 16.075H12.4149C12.9215 16.075 13.3427 15.6833 13.3788 15.1772L14.052 5.75H5.948Z" fill="currentColor"></path>
        <path d="M8.5 8.25C8.91421 8.25 9.25 8.58579 9.25 9V13.25C9.25 13.6642 8.91421 14 8.5 14C8.08579 14 7.75 13.6642 7.75 13.25V9C7.75 8.58579 8.08579 8.25 8.5 8.25Z" fill="currentColor"></path>
        <path d="M11.5 8.25C11.9142 8.25 12.25 8.58579 12.25 9V13.25C12.25 13.6642 11.9142 14 11.5 14C11.0858 14 10.75 13.6642 10.75 13.25V9C10.75 8.58579 11.0858 8.25 11.5 8.25Z" fill="currentColor"></path>
      </svg>
    `;
  }

  function exportIconSvg() {
    return `
      <svg width="16" height="16" viewBox="0 0 20 20" aria-hidden="true" focusable="false" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M9.25 3.75C9.25 3.33579 9.58579 3 10 3C10.4142 3 10.75 3.33579 10.75 3.75V10.1893L12.4697 8.46967C12.7626 8.17678 13.2374 8.17678 13.5303 8.46967C13.8232 8.76256 13.8232 9.23744 13.5303 9.53033L10.5303 12.5303C10.2374 12.8232 9.76256 12.8232 9.46967 12.5303L6.46967 9.53033C6.17678 9.23744 6.17678 8.76256 6.46967 8.46967C6.76256 8.17678 7.23744 8.17678 7.53033 8.46967L9.25 10.1893V3.75Z" fill="currentColor"></path>
        <path d="M4.75 12.75C5.16421 12.75 5.5 13.0858 5.5 13.5V14.25C5.5 14.3881 5.61193 14.5 5.75 14.5H14.25C14.3881 14.5 14.5 14.3881 14.5 14.25V13.5C14.5 13.0858 14.8358 12.75 15.25 12.75C15.6642 12.75 16 13.0858 16 13.5V14.25C16 15.2165 15.2165 16 14.25 16H5.75C4.7835 16 4 15.2165 4 14.25V13.5C4 13.0858 4.33579 12.75 4.75 12.75Z" fill="currentColor"></path>
      </svg>
    `;
  }

  function moveIconSvg() {
    return `
      <svg width="16" height="16" viewBox="0 0 20 20" aria-hidden="true" focusable="false" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M11.25 4.75C11.25 4.33579 11.5858 4 12 4H15.25C15.6642 4 16 4.33579 16 4.75V8C16 8.41421 15.6642 8.75 15.25 8.75C14.8358 8.75 14.5 8.41421 14.5 8V6.56066L9.53033 11.5303C9.23744 11.8232 8.76256 11.8232 8.46967 11.5303C8.17678 11.2374 8.17678 10.7626 8.46967 10.4697L13.4393 5.5H12C11.5858 5.5 11.25 5.16421 11.25 4.75Z" fill="currentColor"></path>
        <path d="M5.75 5.5C5.61193 5.5 5.5 5.61193 5.5 5.75V14.25C5.5 14.3881 5.61193 14.5 5.75 14.5H14.25C14.3881 14.5 14.5 14.3881 14.5 14.25V11.75C14.5 11.3358 14.8358 11 15.25 11C15.6642 11 16 11.3358 16 11.75V14.25C16 15.2165 15.2165 16 14.25 16H5.75C4.7835 16 4 15.2165 4 14.25V5.75C4 4.7835 4.7835 4 5.75 4H8.25C8.66421 4 9 4.33579 9 4.75C9 5.16421 8.66421 5.5 8.25 5.5H5.75Z" fill="currentColor"></path>
      </svg>
    `;
  }

  function configureSvgActionButton(button, label, svg) {
    button.setAttribute("aria-label", label);
    button.dataset.codexActionLabel = label;
    button.removeAttribute("title");
    button.innerHTML = svg;
  }

  function attachButton(row) {
    const settings = codexPlusSettings();
    if (!settings.sessionDelete && !settings.markdownExport && !settings.projectMove) {
      removeActionGroups(row);
      row.dataset.codexDeleteRow = "false";
      row.dataset.codexProjectMoveRow = "false";
      return;
    }
    const existingGroup = actionGroupFromRow(row);
    const existingDeleteButton = existingGroup?.querySelector(`.${buttonClass}`);
    const existingMoreButton = existingGroup?.querySelector(`.${moreButtonClass}`);
    const existingExportButton = existingGroup?.querySelector(`.${exportButtonClass}`);
    const existingMoveButton = existingGroup?.querySelector(`.${projectMoveButtonClass}`);
    const needsMoreMenu = settings.markdownExport || settings.projectMove;
    const hasUnexpectedDelete = !settings.sessionDelete && !!existingDeleteButton;
    const hasUnexpectedMore = !needsMoreMenu && !!existingMoreButton;
    const hasUnexpectedExport = !!existingExportButton;
    const hasUnexpectedMove = !!existingMoveButton;
    const missingDelete = settings.sessionDelete && !existingDeleteButton;
    const missingMore = needsMoreMenu && !existingMoreButton;
    const deleteReady = !settings.sessionDelete || existingDeleteButton?.dataset.codexDeleteVersion === codexDeleteVersion;
    const groupReady = existingGroup?.dataset.codexActionGroupVersion === codexActionGroupVersion;
    if (groupReady && deleteReady && !hasUnexpectedDelete && !hasUnexpectedMore && !hasUnexpectedExport && !hasUnexpectedMove && !missingDelete && !missingMore) {
      return;
    }
    removeActionGroups(row);
    row.dataset.codexDeleteRow = "false";
    row.dataset.codexProjectMoveRow = "false";
    const ref = sessionRefFromRow(row);
    if (!ref.session_id) return;
    row.dataset.codexDeleteRow = "true";
    row.dataset.codexProjectMoveRow = String(!!settings.projectMove);
    const group = document.createElement("div");
    group.className = actionGroupClass;
    group.dataset.codexActionGroupVersion = codexActionGroupVersion;
    if (settings.markdownExport || settings.projectMove) {
      const moreButton = document.createElement("button");
      moreButton.type = "button";
      moreButton.className = `${actionButtonClass} ${moreButtonClass}`;
      moreButton.setAttribute("aria-haspopup", "menu");
      moreButton.setAttribute("aria-expanded", "false");
      configureSvgActionButton(moreButton, t("sidebarMoreActions"), moreIconSvg());
      const moreMenu = document.createElement("div");
      moreMenu.className = moreMenuClass;
      moreMenu.setAttribute("role", "menu");
      moreMenu.hidden = true;
      if (settings.markdownExport) {
        moreMenu.appendChild(createSessionMoreMenuItem(t("sidebarExport"), exportIconSvg(), (event) => {
          stopActionButtonEvent(row, moreButton, event);
          closeSessionMoreMenus();
          exportMarkdown(ref);
        }));
      }
      if (settings.projectMove) {
        moreMenu.appendChild(createSessionMoreMenuItem(t("sidebarMove"), moveIconSvg(), (event) => {
          stopActionButtonEvent(row, moreButton, event);
          closeSessionMoreMenus();
          openProjectMoveMenuForRow(row, moreButton, ref, event);
        }));
      }
      const openMoreMenu = (event) => {
        stopActionButtonEvent(row, moreButton, event);
        hideActionButtonTooltip();
        toggleSessionMoreMenu(row, moreButton, moreMenu);
        if (!moreMenu.hidden) {
          positionSessionMoreMenu(moreButton, moreMenu);
          updateSessionMoreMenuDirection(moreButton, moreMenu);
        }
      };
      installMoreButtonEvents(row, moreButton, openMoreMenu);
      group.appendChild(moreButton);
      moreMenu.__codexSessionMoreRow = row;
      moreMenu.__codexSessionMoreGroup = group;
      document.body.appendChild(moreMenu);
      installSessionMoreMenuAutoClose(row, moreMenu);
    }
    if (settings.sessionDelete) {
      const deleteButton = document.createElement("button");
      deleteButton.type = "button";
      deleteButton.className = `${actionButtonClass} ${buttonClass}`;
      deleteButton.dataset.codexDeleteVersion = codexDeleteVersion;
      configureSvgActionButton(deleteButton, t("sidebarDelete"), trashIconSvg());
      const openDeleteConfirm = (event) => openDeleteConfirmForRow(row, deleteButton, ref, event);
      installActionButtonEvents(row, deleteButton, openDeleteConfirm);
      group.appendChild(deleteButton);
      setTimeout(() => refreshActionButton(deleteButton, row, openDeleteConfirm), 0);
    }
    row.appendChild(group);
    syncActionGroupLayout(row, group);
  }

  function tryAttachButton(row) {
    try {
      attachButton(row);
    } catch (error) {
      window.__codexSessionDeleteAttachButtonFailures = window.__codexSessionDeleteAttachButtonFailures || [];
      window.__codexSessionDeleteAttachButtonFailures.push(String(error?.stack || error));
    }
  }

  function reactArchivedThreadFromNode(node) {
    const reactKey = Object.keys(node).find((key) => key.startsWith("__reactFiber$") || key.startsWith("__reactInternalInstance$"));
    let fiber = reactKey ? node[reactKey] : null;
    for (let depth = 0; fiber && depth < 20; depth += 1, fiber = fiber.return) {
      const props = fiber.memoizedProps || fiber.pendingProps || {};
      if (props.archivedThread?.id) return props.archivedThread;
      const childThread = props.children?.props?.archivedThread;
      if (childThread?.id) return childThread;
    }
    return null;
  }

  function archivedThreadFromRow(row) {
    for (const node of [row, ...row.querySelectorAll("*")]) {
      const thread = reactArchivedThreadFromNode(node);
      if (thread?.id || thread?.sessionId) return thread;
    }
    return null;
  }

  function archivedRefFromRow(row) {
    const archivedThread = archivedThreadFromRow(row);
    if (archivedThread?.id || archivedThread?.sessionId) {
      return { session_id: archivedThread.id || archivedThread.sessionId, title: archivedThread.title || row.querySelector(".truncate.text-base")?.textContent?.trim() || "Untitled session" };
    }
    const sidebarRef = sessionRefFromRow(row);
    if (sidebarRef.session_id) return sidebarRef;
    const titleNode = row.querySelector(".truncate.text-base, [data-thread-title], a, div");
    const title = ((titleNode || row).textContent || "Untitled session")
      .replace("取消归档", "")
      .replace("删除", "")
      .replace(/\d{4}年\d{1,2}月\d{1,2}日.*$/, "")
      .replace(/\s+·\s+.*$/, "")
      .trim()
      .slice(0, 160);
    return { session_id: "", title };
  }

  async function resolveArchivedThread(row) {
    const ref = archivedRefFromRow(row);
    if (ref.session_id) return ref;
    const resolved = await postJson("/archived-thread", { title: ref.title });
    return resolved?.session_id ? resolved : ref;
  }

  function stopArchivedButtonEvent(event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
  }

  function isArchiveTitleText(value) {
    return value === "已归档对话" || value === "Archived conversations";
  }

  function archiveTitleContainer() {
    const heading = Array.from(document.querySelectorAll("h1, h2, h3"))
      .find((element) => isArchiveTitleText((element.textContent || "").trim()));
    if (heading) return heading;
    return Array.from(document.querySelectorAll("h1, h2, h3, div, span"))
      .find((element) => isArchiveTitleText((element.textContent || "").trim()) && element.getBoundingClientRect().x > 350);
  }

  function attachArchivedPageDeleteButton(row) {
    const settings = codexPlusSettings();
    row.querySelectorAll("[data-codex-archive-row-action]").forEach((button) => button.remove());
    row.dataset.codexArchiveDeleteRow = "false";
    if (!settings.sessionDelete && !settings.markdownExport) return;
    const unarchiveButton = Array.from(row.querySelectorAll("button")).find((button) => (button.textContent || "").trim() === "取消归档");
    if (!unarchiveButton) return;
    row.dataset.codexArchiveDeleteRow = "true";
    row.dataset.codexArchiveRowActionsVersion = codexArchiveRowActionsVersion;
    let insertionPoint = unarchiveButton;
    if (settings.markdownExport) {
      const exportButton = document.createElement("button");
      exportButton.type = "button";
      exportButton.className = `codex-archive-delete-all codex-archive-row-button ${exportButtonClass}`;
      exportButton.dataset.codexArchiveRowAction = "export";
      exportButton.textContent = "导出";
      ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
        exportButton.addEventListener(eventName, stopArchivedButtonEvent, true);
      });
      exportButton.addEventListener("click", async (event) => {
        stopArchivedButtonEvent(event);
        const ref = await resolveArchivedThread(row);
        if (!ref.session_id) {
          showToast(t("exportArchivedMissingId"), null);
          return;
        }
        await exportMarkdown(ref);
      }, true);
      insertionPoint.insertAdjacentElement("afterend", exportButton);
      insertionPoint = exportButton;
    }
  }

  function conversationRoot() {
    return document.querySelector(".thread-scroll-container") || document.querySelector("main") || document.querySelector('[role="main"]');
  }

  function nodeOrAncestorLooksLikeCodexUserBubble(node) {
    if (node.nodeType !== 1) return false;
    const className = String(node.className || "");
    if (className.includes("bg-token-foreground/5") && node.parentElement?.classList?.contains("items-end")) return true;
    const bubble = node.closest?.("[class*='bg-token-foreground/5']");
    return !!bubble?.parentElement?.classList?.contains("items-end");
  }

  function nodeLooksLikeCodexUserBubble(node) {
    if (nodeOrAncestorLooksLikeCodexUserBubble(node)) return true;
    return !!node.querySelector?.(".group.flex.w-full.flex-col.items-end.justify-end.gap-1 > [class*='bg-token-foreground/5']");
  }

  function scrollerViewportTop(scroller) {
    if (scroller === document.scrollingElement || scroller === document.documentElement || scroller === document.body) return 0;
    return scroller.getBoundingClientRect().top;
  }

  function nearestScrollableAncestor(node) {
    for (let current = node?.parentElement; current; current = current.parentElement) {
      const style = getComputedStyle(current);
      if (/(auto|scroll)/.test(style.overflowY) && current.scrollHeight > current.clientHeight) return current;
    }
    return document.querySelector(".thread-scroll-container") || document.scrollingElement || document.documentElement;
  }

  const conversationViewContentClasses = [
    "mx-auto",
    "w-full",
    "max-w-(--thread-content-max-width)",
    "px-toolbar",
    "relative",
    "flex",
    "shrink-0",
    "flex-col",
    "pb-8",
  ];
  const conversationViewComposerClasses = [
    "relative",
    "z-10",
    "flex",
    "flex-col",
    "mx-auto",
    "w-full",
    "max-w-(--thread-content-max-width)",
    "px-toolbar",
  ];
  const conversationViewState = {
    contentEl: null,
    composerEl: null,
    rafId: 0,
    settleFramesLeft: 0,
    mo: null,
    ro: null,
    pollId: 0,
    moObserved: false,
    observed: new WeakSet(),
    elements: new Set(),
  };

  function conversationViewTokenSet(el) {
    return new Set(String(el?.className || "").split(/\s+/).filter(Boolean));
  }

  function conversationViewHasAllClasses(el, classes) {
    const set = conversationViewTokenSet(el);
    return classes.every((cls) => set.has(cls));
  }

  function conversationViewFindByClasses(classes) {
    return Array.from(document.querySelectorAll("div")).find((el) => conversationViewHasAllClasses(el, classes)) || null;
  }

  function conversationViewFindContentEl() {
    return conversationViewFindByClasses(conversationViewContentClasses);
  }

  function conversationViewFindComposerEl() {
    return conversationViewFindByClasses(conversationViewComposerClasses);
  }

  function codexServiceTierBadgeVisibleElement(element) {
    if (!(element instanceof HTMLElement) || !element.isConnected) return false;
    const style = getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden") return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }

  function codexServiceTierBadgeText(element) {
    return String(element?.textContent || "").replace(/\s+/g, " ").trim();
  }

  function codexServiceTierKnownProviderNames() {
    return uniqueValues([
      codexModelCatalog.provider_name,
      codexModelCatalog.model_provider,
    ]).map((value) => value.toLowerCase());
  }

  function codexServiceTierLooksLikeProviderButton(button, providerNames) {
    const text = codexServiceTierBadgeText(button);
    if (!text || text.length > 32) return false;
    const lower = text.toLowerCase();
    if (providerNames.includes(lower)) return true;
    if (/\s/.test(text)) return false;
    if (!/[a-z]/i.test(text)) return false;
    if (!/^[a-z0-9][a-z0-9._-]{1,31}$/i.test(text)) return false;
    if (/^(local|remote|cloud|standard|default|fast|worktree|new|send|stop|codex)$/i.test(text)) return false;
    if (/^(gpt|o[1-9]|claude|gemini|deepseek|qwen|kimi|moonshot|mistral|llama|sonnet|opus|haiku)[a-z0-9._-]*$/i.test(text)) return false;
    return true;
  }

  function codexServiceTierBadgeButtonCandidates(composer) {
    const composerRect = composer.getBoundingClientRect();
    return Array.from(composer.querySelectorAll("button, [role='button']"))
      .filter((button) => !button.closest?.(`[data-codex-service-tier-badge="true"]`))
      .filter(codexServiceTierBadgeVisibleElement)
      .filter((button) => {
        const rect = button.getBoundingClientRect();
        return rect.bottom >= composerRect.top + composerRect.height * 0.35;
      })
      .sort((left, right) => {
        const leftRect = left.getBoundingClientRect();
        const rightRect = right.getBoundingClientRect();
        return (rightRect.bottom - leftRect.bottom) || (leftRect.left - rightRect.left);
      });
  }

  function codexServiceTierVisibleComposerFooters(root = document) {
    const footers = [
      ...(root?.matches?.(".composer-footer") ? [root] : []),
      ...Array.from(root?.querySelectorAll?.(".composer-footer") || []),
    ];
    return footers
      .filter(codexServiceTierBadgeVisibleElement)
      .sort((left, right) => {
        const leftRect = left.getBoundingClientRect();
        const rightRect = right.getBoundingClientRect();
        return (rightRect.bottom - leftRect.bottom) || (rightRect.width - leftRect.width);
      });
  }

  function codexServiceTierComposerScore(composer) {
    const text = codexServiceTierBadgeText(composer).toLowerCase();
    const providerNames = codexServiceTierKnownProviderNames();
    let score = 0;
    if (providerNames.some((name) => name && text.includes(name))) score += 40;
    if (/完全访问权限|full access|model|超高|high|sub2api|provider/i.test(text)) score += 20;
    if (/本地模式|local mode|worktree|branch|codex\//i.test(text)) score -= 30;
    if (composer.matches?.(".composer-footer")) score += 4;
    if (composer.querySelector?.(".composer-footer")) score += 8;
    const buttons = Array.from(composer.querySelectorAll?.("button, [role='button']") || []).filter(codexServiceTierBadgeVisibleElement);
    if (buttons.some((button) => codexServiceTierLooksLikeProviderButton(button, providerNames))) score += 30;
    score += Math.min(10, buttons.length);
    return score;
  }

  function codexServiceTierComposerCandidates() {
    const candidates = new Set();
    const threadComposer = conversationViewFindComposerEl();
    if (threadComposer && codexServiceTierBadgeVisibleElement(threadComposer)) candidates.add(threadComposer);
    codexServiceTierVisibleComposerFooters().forEach((footer) => {
      candidates.add(footer);
      let node = footer.parentElement;
      for (let depth = 0; node instanceof HTMLElement && depth < 6; depth += 1, node = node.parentElement) {
        if (codexServiceTierBadgeVisibleElement(node)) candidates.add(node);
      }
    });
    return Array.from(candidates);
  }

  function codexServiceTierBestComposerFooter(root = document) {
    return codexServiceTierVisibleComposerFooters(root)
      .map((footer, index) => ({ footer, index, score: codexServiceTierComposerScore(footer) }))
      .sort((left, right) => (right.score - left.score) || (left.index - right.index))[0]?.footer || null;
  }

  function codexServiceTierFindComposerEl() {
    return codexServiceTierComposerCandidates()
      .map((composer, index) => ({ composer, index, score: codexServiceTierComposerScore(composer) }))
      .sort((left, right) => (right.score - left.score) || (left.index - right.index))[0]?.composer || null;
  }

  function codexServiceTierBadgeAnchor(composer) {
    const providerNames = codexServiceTierKnownProviderNames();
    const buttons = codexServiceTierBadgeButtonCandidates(composer);
    const exact = buttons.find((button) => providerNames.includes(codexServiceTierBadgeText(button).toLowerCase()));
    if (exact) return exact;
    const composerRect = composer.getBoundingClientRect();
    return buttons.find((button) => {
      const rect = button.getBoundingClientRect();
      return rect.left >= composerRect.left + composerRect.width * 0.42 && codexServiceTierLooksLikeProviderButton(button, providerNames);
    }) || null;
  }

  function codexServiceTierComposerFooter(composer) {
    if (composer?.matches?.(".composer-footer")) return composer;
    return codexServiceTierBestComposerFooter(composer) || codexServiceTierBestComposerFooter() || null;
  }

  function codexServiceTierBadgeFooterGroup(composer) {
    const footer = codexServiceTierComposerFooter(composer);
    if (!footer) return null;
    const children = Array.from(footer.children).filter(codexServiceTierBadgeVisibleElement);
    if (!children.length) return footer;
    const providerNames = codexServiceTierKnownProviderNames();
    const providerGroup = children.find((child) => {
      const text = codexServiceTierBadgeText(child).toLowerCase();
      return providerNames.some((name) => name && text.includes(name));
    });
    return providerGroup || children[children.length - 1] || footer;
  }

  function codexServiceTierBadgePlacement(composer) {
    const anchor = composer ? codexServiceTierBadgeAnchor(composer) : null;
    if (anchor?.parentElement) return { parent: anchor.parentElement, before: anchor };
    const group = composer ? codexServiceTierBadgeFooterGroup(composer) : null;
    if (group) return { parent: group, before: group.firstChild };
    return null;
  }

  function wireCodexServiceTierBadge(badge) {
    if (!badge || badge.dataset.codexServiceTierBadgeWired === codexServiceTierBadgeVersion) return;
    badge.dataset.codexServiceTierBadgeWired = codexServiceTierBadgeVersion;
    badge.setAttribute("role", "button");
    badge.setAttribute("tabindex", "0");
    badge.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (codexServiceTierState.status === "loading") return;
      toggleCodexServiceTierFromBadge();
    });
    badge.addEventListener("keydown", (event) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      event.stopPropagation();
      if (codexServiceTierState.status === "loading") return;
      toggleCodexServiceTierFromBadge();
    });
  }

  function installCodexServiceTierBadge() {
    if (!codexPlusSettings().serviceTierControls) {
      removeCodexServiceTierBadges();
      return;
    }
    const composer = codexServiceTierFindComposerEl();
    const placement = composer ? codexServiceTierBadgePlacement(composer) : null;
    const existingBadges = Array.from(document.querySelectorAll(`[data-codex-service-tier-badge="true"]`));
    if (!composer || !placement?.parent) {
      existingBadges.forEach((badge) => badge.remove());
      return;
    }
    let badge = existingBadges.find((node) => node.closest?.(".composer-footer") || node.closest?.("button") == null) || existingBadges[0];
    existingBadges.forEach((node) => {
      if (node !== badge) node.remove();
    });
    if (!badge || badge.dataset.codexServiceTierBadgeVersion !== codexServiceTierBadgeVersion) {
      badge?.remove();
      badge = document.createElement("span");
      badge.className = codexServiceTierBadgeClass;
      badge.dataset.codexServiceTierBadge = "true";
      badge.dataset.codexServiceTierBadgeVersion = codexServiceTierBadgeVersion;
    }
    wireCodexServiceTierBadge(badge);
    const before = placement.before?.parentElement === placement.parent ? placement.before : null;
    if (badge.parentElement !== placement.parent || badge.nextSibling !== before) {
      placement.parent.insertBefore(badge, before);
    }
    refreshCodexServiceTierBadges();
  }

  function removeCodexServiceTierBadges() {
    document.querySelectorAll(`[data-codex-service-tier-badge="true"]`).forEach((badge) => badge.remove());
  }

  function conversationViewRememberOriginals(el) {
    if (!el) return;
    conversationViewState.elements.add(el);
    const original = {
      width: el.style.width || "",
      maxWidth: el.style.maxWidth || "",
      marginLeft: el.style.marginLeft || "",
      marginRight: el.style.marginRight || "",
      left: el.style.left || "",
      transform: el.style.transform || "",
      boxSizing: el.style.boxSizing || "",
    };
    if (!("codexPlusConversationViewOriginalWidth" in el.dataset)) el.dataset.codexPlusConversationViewOriginalWidth = original.width;
    if (!("codexPlusConversationViewOriginalMaxWidth" in el.dataset)) el.dataset.codexPlusConversationViewOriginalMaxWidth = original.maxWidth;
    if (!("codexPlusConversationViewOriginalMarginLeft" in el.dataset)) el.dataset.codexPlusConversationViewOriginalMarginLeft = original.marginLeft;
    if (!("codexPlusConversationViewOriginalMarginRight" in el.dataset)) el.dataset.codexPlusConversationViewOriginalMarginRight = original.marginRight;
    if (!("codexPlusConversationViewOriginalLeft" in el.dataset)) el.dataset.codexPlusConversationViewOriginalLeft = original.left;
    if (!("codexPlusConversationViewOriginalTransform" in el.dataset)) el.dataset.codexPlusConversationViewOriginalTransform = original.transform;
    if (!("codexPlusConversationViewOriginalBoxSizing" in el.dataset)) el.dataset.codexPlusConversationViewOriginalBoxSizing = original.boxSizing;
  }

  function conversationViewRestoreElement(el) {
    if (!el) return;
    if ("codexPlusConversationViewOriginalWidth" in el.dataset) {
      el.style.width = el.dataset.codexPlusConversationViewOriginalWidth;
      delete el.dataset.codexPlusConversationViewOriginalWidth;
    }
    if ("codexPlusConversationViewOriginalMaxWidth" in el.dataset) {
      el.style.maxWidth = el.dataset.codexPlusConversationViewOriginalMaxWidth;
      delete el.dataset.codexPlusConversationViewOriginalMaxWidth;
    }
    if ("codexPlusConversationViewOriginalMarginLeft" in el.dataset) {
      el.style.marginLeft = el.dataset.codexPlusConversationViewOriginalMarginLeft;
      delete el.dataset.codexPlusConversationViewOriginalMarginLeft;
    }
    if ("codexPlusConversationViewOriginalMarginRight" in el.dataset) {
      el.style.marginRight = el.dataset.codexPlusConversationViewOriginalMarginRight;
      delete el.dataset.codexPlusConversationViewOriginalMarginRight;
    }
    if ("codexPlusConversationViewOriginalLeft" in el.dataset) {
      el.style.left = el.dataset.codexPlusConversationViewOriginalLeft;
      delete el.dataset.codexPlusConversationViewOriginalLeft;
    }
    if ("codexPlusConversationViewOriginalTransform" in el.dataset) {
      el.style.transform = el.dataset.codexPlusConversationViewOriginalTransform;
      delete el.dataset.codexPlusConversationViewOriginalTransform;
    }
    if ("codexPlusConversationViewOriginalBoxSizing" in el.dataset) {
      el.style.boxSizing = el.dataset.codexPlusConversationViewOriginalBoxSizing;
      delete el.dataset.codexPlusConversationViewOriginalBoxSizing;
    }
  }

  function conversationViewResetOwnOffset(el) {
    if (!el) return;
    const originalTransform = el.dataset.codexPlusConversationViewOriginalTransform || "";
    const originalLeft = el.dataset.codexPlusConversationViewOriginalLeft || "";
    if (el.style.left !== originalLeft) el.style.left = originalLeft;
    if (el.style.transform !== originalTransform) el.style.transform = originalTransform;
    const transform = String(el.style.transform || "").trim();
    if (/^(translateX\([^)]*\)\s*)+$/i.test(transform)) {
      el.style.transform = "";
    }
  }

  function conversationViewApplyNativeWidth(el) {
    conversationViewRememberOriginals(el);
    const maxWidth = `${conversationViewWidth()}px`;
    if (el.style.boxSizing !== "border-box") el.style.boxSizing = "border-box";
    if (el.style.width !== "100%") el.style.width = "100%";
    if (el.style.maxWidth !== maxWidth) el.style.maxWidth = maxWidth;
    if (el.style.marginLeft !== "auto") el.style.marginLeft = "auto";
    if (el.style.marginRight !== "auto") el.style.marginRight = "auto";
  }

  function conversationViewSessionRectFor(el) {
    return el?.parentElement?.getBoundingClientRect() || null;
  }

  function conversationViewHtmlCenter() {
    const rect = document.documentElement.getBoundingClientRect();
    return rect.left + rect.width / 2;
  }

  function conversationViewHasRoomForHtmlCenter(nativeRect, bounds) {
    if (!nativeRect || !bounds) return false;
    const targetLeft = conversationViewHtmlCenter() - nativeRect.width / 2;
    const targetRight = targetLeft + nativeRect.width;
    return targetLeft >= bounds.left - 0.5 && targetRight <= bounds.right + 0.5;
  }

  function conversationViewAlignElement(el) {
    if (!el?.isConnected) return;
    conversationViewApplyNativeWidth(el);
    conversationViewResetOwnOffset(el);
    const nativeRect = el.getBoundingClientRect();
    const bounds = conversationViewSessionRectFor(el);
    if (!conversationViewHasRoomForHtmlCenter(nativeRect, bounds)) return;
    const targetLeft = conversationViewHtmlCenter() - nativeRect.width / 2;
    const delta = targetLeft - nativeRect.left;
    if (Math.abs(delta) > 0.5) {
      const nextLeft = `${delta.toFixed(2)}px`;
      if (el.style.left !== nextLeft) el.style.left = nextLeft;
    }
  }

  function conversationViewObserveIfNeeded(el) {
    if (!el || !conversationViewState.ro || conversationViewState.observed.has(el)) return;
    conversationViewState.observed.add(el);
    conversationViewState.ro.observe(el);
  }

  function conversationViewResolveTargets() {
    if (!conversationViewState.contentEl?.isConnected) conversationViewState.contentEl = conversationViewFindContentEl();
    if (!conversationViewState.composerEl?.isConnected) conversationViewState.composerEl = conversationViewFindComposerEl();
    [
      document.documentElement,
      document.body,
      conversationViewState.contentEl,
      conversationViewState.contentEl?.parentElement,
      conversationViewState.contentEl?.parentElement?.parentElement,
      conversationViewState.composerEl,
      conversationViewState.composerEl?.parentElement,
      conversationViewState.composerEl?.parentElement?.parentElement,
    ].forEach(conversationViewObserveIfNeeded);
  }

  function conversationViewAlignNow() {
    if (!codexPlusSettings().conversationView) return;
    conversationViewResolveTargets();
    conversationViewAlignElement(conversationViewState.contentEl);
    conversationViewAlignElement(conversationViewState.composerEl);
  }

  function scheduleConversationViewAlign(frames = 16) {
    conversationViewState.settleFramesLeft = Math.max(conversationViewState.settleFramesLeft, frames);
    if (conversationViewState.rafId) return;
    const tick = () => {
      conversationViewState.rafId = 0;
      conversationViewAlignNow();
      conversationViewState.settleFramesLeft -= 1;
      if (conversationViewState.settleFramesLeft > 0) {
        conversationViewState.rafId = requestAnimationFrame(tick);
      }
    };
    conversationViewState.rafId = requestAnimationFrame(tick);
  }

  function cleanupConversationView() {
    if (conversationViewState.rafId) cancelAnimationFrame(conversationViewState.rafId);
    if (conversationViewState.pollId) clearInterval(conversationViewState.pollId);
    conversationViewState.rafId = 0;
    conversationViewState.pollId = 0;
    conversationViewState.mo?.disconnect();
    conversationViewState.ro?.disconnect();
    conversationViewState.mo = null;
    conversationViewState.ro = null;
    conversationViewState.moObserved = false;
    conversationViewState.observed = new WeakSet();
    conversationViewState.elements.forEach(conversationViewRestoreElement);
    conversationViewState.elements.clear();
    conversationViewState.contentEl = null;
    conversationViewState.composerEl = null;
  }

  window.__codexPlusConversationViewCleanup = cleanupConversationView;

  function ensureConversationViewRuntime() {
    if (conversationViewState.ro && conversationViewState.mo && conversationViewState.pollId) return;
    conversationViewState.ro = conversationViewState.ro || new ResizeObserver(() => scheduleConversationViewAlign());
    conversationViewState.mo = conversationViewState.mo || new MutationObserver(() => scheduleConversationViewAlign());
    if (document.body && !conversationViewState.moObserved) {
      conversationViewState.mo.observe(document.body, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ["class", "hidden", "data-state", "aria-hidden"],
      });
      conversationViewState.moObserved = true;
    }
    conversationViewState.pollId = conversationViewState.pollId || window.setInterval(() => scheduleConversationViewAlign(2), 350);
  }

  function refreshConversationView() {
    if (!codexPlusSettings().conversationView) {
      cleanupConversationView();
      return;
    }
    ensureConversationViewRuntime();
    scheduleConversationViewAlign();
  }

  function scanLightweight() {
    installStyle();
    installCodexServiceTierDispatcherPatch();
    installCodexPlusMenu();
    localizeCodexMenus();
    scheduleBackendHeartbeat();
    installDeleteButtonEventDelegation();
    updateThreadScrollHandlers();
    installThreadScrollProgrammaticScrollGuard();
    installThreadScrollNavigationCapture();
    installThreadScrollUserIntentCapture();
    installThreadScrollRouteHooks();
    scheduleThreadScrollSync(true);
    refreshCodexServiceTierControls();
  }

  let zedRemoteStatusPromise = null;
  const zedRemoteMissingHostMessage = "Cannot determine remote SSH host for this file";

  function showZedRemoteToast(message) {
    document.querySelectorAll(`.${zedRemoteToastClass}`).forEach((node) => node.remove());
    const toast = document.createElement("div");
    toast.className = zedRemoteToastClass;
    toast.textContent = message;
    document.body.appendChild(toast);
    setTimeout(() => toast.remove(), 3200);
  }

  async function loadZedRemoteStatus() {
    zedRemoteStatusPromise = zedRemoteStatusPromise || postJson("/zed-remote/status", {});
    return zedRemoteStatusPromise;
  }

  async function resolveZedRemoteHost(hostId) {
    const result = await postJson("/zed-remote/resolve-host", { hostId });
    return result?.status === "ok" && result.ssh ? result.ssh : null;
  }

  function zedRemoteIsRemoteHostId(hostId) {
    return zedRemoteString(hostId).startsWith("remote-ssh-");
  }

  function zedRemoteProjectIdFromRow(row) {
    const projectList = row?.closest?.("[data-app-action-sidebar-project-list-id]");
    const projectId = zedRemoteString(projectList?.getAttribute?.("data-app-action-sidebar-project-list-id"));
    if (projectId) return projectId;
    const projectRow = row?.closest?.("[data-app-action-sidebar-project-id]");
    return zedRemoteString(projectRow?.getAttribute?.("data-app-action-sidebar-project-id"));
  }

  function zedRemoteWorkspaceRootFromObject(source) {
    if (!source || typeof source !== "object") return "";
    for (const key of ["remoteWorkspaceRoot", "workspaceRoot", "displayCwd", "cwd", "rootPath", "workingDirectory", "workingDir"]) {
      const workspaceRoot = zedRemoteString(source[key]);
      if (workspaceRoot.startsWith("/") && !/\/\.codex$/.test(workspaceRoot)) return workspaceRoot;
    }
    const hostConfig = source.hostConfig || source.sshHostConfig || source.remoteHostConfig || source.ssh || {};
    for (const key of ["remoteWorkspaceRoot", "workspaceRoot", "rootPath", "cwd"]) {
      const workspaceRoot = zedRemoteString(hostConfig[key]);
      if (workspaceRoot.startsWith("/") && !/\/\.codex$/.test(workspaceRoot)) return workspaceRoot;
    }
    return "";
  }

  function zedRemoteWorkspaceRootFromElement(element) {
    for (const key of zedRemoteReactKeys(element)) {
      const workspaceRoot = zedRemoteWalkObject(element[key], zedRemoteWorkspaceRootFromObject, { maxDepth: 10, maxNodes: 320 });
      if (workspaceRoot) return workspaceRoot;
    }
    return "";
  }

  function zedRemoteWorkspaceRootFromRow(row) {
    for (let node = row; node && node !== document.body; node = node.parentElement) {
      const workspaceRoot = zedRemoteWorkspaceRootFromElement(node);
      if (workspaceRoot) return workspaceRoot;
    }
    return "";
  }

  function zedRemoteActiveThreadRow() {
    const rows = sessionRows(true).filter((row) => row instanceof HTMLElement);
    return rows.find((row) => row.getAttribute("data-app-action-sidebar-thread-active") === "true")
      || rows.find((row) => row.getAttribute("aria-current") === "page" || row.getAttribute("aria-current") === "true")
      || null;
  }

  function zedRemoteCurrentFallbackPayload() {
    const row = zedRemoteActiveThreadRow();
    const ref = row ? sessionRefFromRow(row) : currentSessionRef();
    const threadId = ref.session_id || locationThreadId();
    const hostId = zedRemoteString(row?.getAttribute?.("data-app-action-sidebar-thread-host-id"));
    const isRemoteHost = zedRemoteIsRemoteHostId(hostId);
    const payload = {};
    if (threadId) payload.threadId = threadId;
    if (hostId && hostId !== "local") payload.hostId = hostId;
    if (!isRemoteHost) return payload;
    const remoteWorkspaceRoot = zedRemoteWorkspaceRootFromRow(row);
    const remoteProjectId = zedRemoteProjectIdFromRow(row);
    if (remoteWorkspaceRoot) payload.remoteWorkspaceRoot = remoteWorkspaceRoot;
    if (remoteProjectId) payload.remoteProjectId = remoteProjectId;
    return payload;
  }

  function zedRemoteCurrentThreadId() {
    return zedRemoteCurrentFallbackPayload().threadId || "";
  }

  async function resolveZedRemoteFallbackRequest() {
    const payload = zedRemoteCurrentFallbackPayload();
    if (!zedRemoteIsRemoteHostId(payload.hostId)) return null;
    const result = await postJson("/zed-remote/fallback-request", payload);
    return result?.status === "ok" && result.request ? result.request : null;
  }

  function zedRemoteOpenStrategy() {
    const strategy = zedRemoteString(codexPlusBackendSettings.zedRemoteOpenStrategy);
    return ["addToFocusedWorkspace", "reuseWindow", "newWindow", "default"].includes(strategy)
      ? strategy
      : "addToFocusedWorkspace";
  }

  function zedRemoteString(value) {
    return typeof value === "string" || typeof value === "number" ? String(value).trim() : "";
  }

  function zedRemoteTruthy(value) {
    if (value === true) return true;
    if (typeof value === "string") return /^(true|1|yes|enabled|ssh)$/i.test(value.trim());
    return false;
  }

  function zedRemoteHasTrustedSshSignal(source, hostConfig) {
    return zedRemoteTruthy(source?.supportsSsh) || zedRemoteTruthy(hostConfig?.supportsSsh);
  }

  function zedRemoteContextFromObject(source) {
    if (!source || typeof source !== "object") return null;
    const hostConfig = source.hostConfig || source.sshHostConfig || source.remoteHostConfig || source.ssh || {};
    const host = zedRemoteString(source.remoteHost || source.sshHost || source.host || source.hostname || source.hostName || hostConfig.host || hostConfig.hostname || hostConfig.hostName || hostConfig.sshHost);
    const hostId = zedRemoteString(source.hostId);
    const cwd = zedRemoteString(source.cwd || source.workspaceRoot || source.rootPath || source.remoteWorkspaceRoot || hostConfig.remoteWorkspaceRoot || hostConfig.workspaceRoot || hostConfig.rootPath);
    if ((!host || !zedRemoteHasTrustedSshSignal(source, hostConfig)) && !(hostId.startsWith("remote-ssh-") && cwd.startsWith("/"))) return null;
    const user = zedRemoteString(source.remoteUser || source.sshUser || source.user || source.username || hostConfig.user || hostConfig.username || hostConfig.sshUser);
    const port = zedRemoteString(source.remotePort || source.sshPort || source.port || hostConfig.port || hostConfig.sshPort);
    const workspaceRoot = cwd;
    return { hostId, ssh: { user, host, port }, workspaceRoot };
  }

  function zedRemoteWalkObject(root, visitor, options = {}) {
    const maxDepth = options.maxDepth || 6;
    const maxNodes = options.maxNodes || 180;
    const visited = new WeakSet();
    const stack = [{ value: root, depth: 0 }];
    let scanned = 0;
    while (stack.length && scanned < maxNodes) {
      const { value, depth } = stack.pop();
      if (!value || typeof value !== "object" || visited.has(value) || depth > maxDepth) continue;
      visited.add(value);
      scanned += 1;
      const result = visitor(value);
      if (result) return result;
      if (value instanceof Element || value === window || value === document || value === document.body || value === document.documentElement) continue;
      for (const key of Object.keys(value).slice(0, 80)) {
        if (key === "ownerDocument" || key === "parentElement" || key === "parentNode" || key === "children" || key === "childNodes") continue;
        let child;
        try {
          child = value[key];
        } catch {
          continue;
        }
        if (child && typeof child === "object") stack.push({ value: child, depth: depth + 1 });
      }
    }
    return null;
  }

  function zedRemoteReactKeys(element) {
    return Object.keys(element).filter((key) => key.startsWith("__reactFiber") || key.startsWith("__reactInternalInstance") || key.startsWith("__reactProps"));
  }

  function zedRemoteContextFromElement(element) {
    for (const key of zedRemoteReactKeys(element)) {
      const context = zedRemoteWalkObject(element[key], zedRemoteContextFromObject);
      if (context) return context;
    }
    return null;
  }

  function zedRemoteContextForElement(element) {
    for (let node = element; node && node !== document.body; node = node.parentElement) {
      const context = zedRemoteContextFromElement(node);
      if (context) return context;
    }
    return null;
  }

  function zedRemoteHostIdFromText(text) {
    const source = String(text || "");
    const match = source.match(/\bremote-ssh-[A-Za-z0-9:_-]+\b/);
    return match ? match[0] : "";
  }

  function zedRemoteWorkspaceRootForPath(path) {
    const source = String(path || "").trim();
    const projects = Array.from(document.querySelectorAll(selectors.sidebarThread))
      .map((row) => ({
        label: (row.textContent || "").replace(/\s+/g, " ").trim(),
        selected: row.getAttribute("aria-current") === "page" || row.getAttribute("data-selected") === "true" || row.getAttribute("data-active") === "true" || row.className.includes("selected"),
      }))
      .filter((row) => row.label);
    const selected = projects.find((row) => row.selected)?.label || "";
    for (const label of [selected, ...projects.map((row) => row.label)]) {
      const name = label.match(/^([A-Za-z0-9._-]+)/)?.[1];
      if (name && source.includes(`/repo/${name}/`)) return source.slice(0, source.indexOf(`/repo/${name}/`) + `/repo/${name}`.length);
    }
    const repoIndex = source.indexOf("/bin/repo/");
    if (repoIndex >= 0) {
      const afterRepo = source.slice(repoIndex + "/bin/repo/".length);
      const project = afterRepo.split("/")[0];
      if (project) return source.slice(0, repoIndex + "/bin/repo/".length + project.length);
    }
    return source;
  }

  function zedRemoteFallbackContextForElement(element) {
    const pathText = (element.textContent || "").trim();
    if (!pathText.startsWith("/")) return null;
    const root = element.closest("main") || document.body;
    const hostId = zedRemoteHostIdFromText(root?.textContent || "") || "remote-ssh-codex-managed:remote";
    return { hostId, ssh: { user: "", host: "", port: "" }, workspaceRoot: zedRemoteWorkspaceRootForPath(pathText) };
  }

  function zedRemoteContextFromSerializedState(text) {
    const source = String(text || "");
    if (!source.includes("hostConfig") || !source.includes("supportsSsh") || !source.includes("remoteWorkspaceRoot")) return null;
    const trimmed = source.trim();
    if (/^[{[]/.test(trimmed)) {
      try {
        const parsed = JSON.parse(trimmed);
        const context = zedRemoteWalkObject(parsed, zedRemoteContextFromObject, { maxDepth: 10, maxNodes: 300 });
        if (context) return context;
      } catch {
      }
    }
    if (!/['"]supportsSsh['"]\s*:\s*true/.test(source)) return null;
    const fieldValue = (name) => {
      const match = source.match(new RegExp(`["']${name}["']\\s*:\\s*["']([^"']+)["']`));
      return match ? match[1] : "";
    };
    const host = fieldValue("host") || fieldValue("hostname") || fieldValue("hostName") || fieldValue("sshHost") || fieldValue("remoteHost");
    if (!host) return null;
    return {
      ssh: {
        user: fieldValue("user") || fieldValue("username") || fieldValue("sshUser") || fieldValue("remoteUser"),
        host,
        port: fieldValue("port") || fieldValue("sshPort") || fieldValue("remotePort"),
      },
      workspaceRoot: fieldValue("remoteWorkspaceRoot") || fieldValue("workspaceRoot") || fieldValue("rootPath"),
    };
  }

  const zedRemoteContextCacheTtlMs = 1200;
  let zedRemoteContextCache = { scope: null, at: 0, value: null };

  function zedRemoteScopedElements(scope, selector) {
    const root = scope?.querySelectorAll ? scope : document;
    const nodes = [];
    if (scope instanceof HTMLElement && scope.matches?.(selector)) nodes.push(scope);
    root.querySelectorAll?.(selector).forEach((node) => nodes.push(node));
    return Array.from(new Set(nodes));
  }

  function zedRemoteContextFromDataset(node) {
    if (!(node instanceof HTMLElement)) return null;
    const data = node.dataset;
    return zedRemoteContextFromObject({
      hostConfig: data.hostConfig ? { host: data.hostConfig, supportsSsh: true } : {},
      supportsSsh: data.supportsSsh || data.supportsSshRemote,
      sshHost: data.sshHost,
      remoteHost: data.remoteHost,
      host: data.host,
      sshUser: data.sshUser,
      remoteUser: data.remoteUser,
      user: data.user,
      sshPort: data.sshPort,
      remotePort: data.remotePort,
      port: data.port,
      remoteWorkspaceRoot: data.remoteWorkspaceRoot,
      workspaceRoot: data.workspaceRoot,
    });
  }

  function zedRemoteContextUncached(scope = document) {
    const explicitSelector = "[data-host-config], [data-ssh-host], [data-remote-host], [data-remote-workspace-root], [data-supports-ssh]";
    for (const node of zedRemoteScopedElements(scope, explicitSelector)) {
      if (isExtensionUiNode(node)) continue;
      const context = zedRemoteContextFromDataset(node);
      if (context) return context;
    }
    const reactSelector = "[data-remote-path], [data-file-path], [data-path], [data-open-in-targets], [data-open-file], [data-codex-open-file], [role='menuitem']";
    const reactNodes = zedRemoteScopedElements(scope, reactSelector);
    if (scope instanceof HTMLElement && !isExtensionUiNode(scope)) reactNodes.unshift(scope);
    for (const node of Array.from(new Set(reactNodes)).slice(0, 60)) {
      if (!(node instanceof HTMLElement) || isExtensionUiNode(node)) continue;
      const context = zedRemoteContextFromElement(node);
      if (context) return context;
    }
    if (scope !== document) return null;
    const scripts = Array.from(document.querySelectorAll("script[type='application/json'], script[data-state], script#__NEXT_DATA__, script:not([src])"));
    for (const script of scripts.slice(0, 20)) {
      const context = zedRemoteContextFromSerializedState(script.textContent || "");
      if (context) return context;
    }
    return null;
  }

  function zedRemoteContext(scope = document) {
    const settings = codexPlusSettings();
    if (!settings.zedRemoteOpen) return null;
    const now = Date.now();
    if (zedRemoteContextCache.scope === scope && now - zedRemoteContextCache.at < zedRemoteContextCacheTtlMs) {
      return zedRemoteContextCache.value;
    }
    const value = zedRemoteContextUncached(scope);
    zedRemoteContextCache = { scope, at: now, value };
    return value;
  }

  function zedRemoteAbsolutePath(value, workspaceRoot) {
    const text = String(value || "").trim();
    if (!text) return "";
    if (text.startsWith("/")) return text;
    if (workspaceRoot && !text.includes("://") && !text.startsWith("~")) {
      return `${workspaceRoot.replace(/\/+$/, "")}/${text.replace(/^\.\//, "")}`;
    }
    return "";
  }

  function zedRemoteMetadataRemotePath(source) {
    if (!source || typeof source !== "object") return "";
    return zedRemoteString(source.remotePath || source.remote_path || source.path || source.filePath || source.file_path || source.openFile?.remotePath || source.openFile?.path);
  }

  function zedRemotePathFromElementMetadata(element) {
    const dataPath = element.dataset.remotePath || element.dataset.filePath || element.dataset.path || "";
    if (dataPath) return dataPath;
    for (const key of zedRemoteReactKeys(element)) {
      const path = zedRemoteWalkObject(element[key], zedRemoteMetadataRemotePath, { maxDepth: 6, maxNodes: 120 });
      if (path) return path;
    }
    return "";
  }

  function zedRemoteInlinePathFromElement(element, context) {
    if (!context?.hostId && !context?.ssh?.host) return "";
    const text = (element.textContent || "").trim();
    if (!text || text.length > 600 || !text.startsWith("/")) return "";
    const path = zedRemoteAbsolutePath(text, context.workspaceRoot || "");
    if (!path) return "";
    if (context.workspaceRoot && !path.startsWith(`${context.workspaceRoot.replace(/\/+$/, "")}/`) && path !== context.workspaceRoot) return "";
    return path;
  }

  function zedRemoteAnchorHasOpenFileMetadata(anchor) {
    if (!(anchor instanceof HTMLAnchorElement)) return false;
    if (anchor.dataset.remotePath || anchor.dataset.filePath || anchor.dataset.path || anchor.dataset.openInTargets || anchor.dataset.openFile || anchor.dataset.codexOpenFile) return true;
    const label = `${anchor.getAttribute("aria-label") || ""} ${anchor.getAttribute("data-testid") || ""} ${anchor.getAttribute("rel") || ""}`;
    return /open[-_\s]?file|open-in-targets|remote/i.test(label) && !!zedRemotePathFromElementMetadata(anchor);
  }

  function zedRemoteFileCandidates(context, scope = document) {
    const candidates = [];
    const seen = new Set();
    const addCandidate = (node, candidateContext, rawPath) => {
      if (!candidateContext?.ssh?.host && !candidateContext?.hostId) return;
      const path = zedRemoteAbsolutePath(rawPath, candidateContext.workspaceRoot || "");
      if (!path || seen.has(path)) return;
      seen.add(path);
      candidates.push({ node, request: { ssh: candidateContext.ssh, hostId: candidateContext.hostId || "", path } });
    };
    const selectors = "[data-remote-path], [data-file-path], [data-path], [data-open-in-targets], [data-open-file], [data-codex-open-file], a[data-remote-path], a[data-file-path], a[data-path]";
    zedRemoteScopedElements(scope, selectors).forEach((node) => {
      if (!(node instanceof HTMLElement) || isExtensionUiNode(node)) return;
      if (node instanceof HTMLAnchorElement && !zedRemoteAnchorHasOpenFileMetadata(node)) return;
      addCandidate(node, zedRemoteContextForElement(node) || context, zedRemotePathFromElementMetadata(node));
    });
    if (scope !== document) {
      zedRemoteScopedElements(scope, "span.inline-markdown, code, [class*='inlineMarkdown']").forEach((node) => {
        if (!(node instanceof HTMLElement) || isExtensionUiNode(node)) return;
        const candidateContext = zedRemoteContextForElement(node) || context || zedRemoteFallbackContextForElement(node);
        if (!candidateContext?.hostId && !candidateContext?.ssh?.host) return;
        const path = zedRemoteInlinePathFromElement(node, candidateContext);
        if (path) addCandidate(node, candidateContext, path);
      });
    }
    return candidates;
  }

  function zedRemoteBestOpenRequest(scope = document, context = zedRemoteContext(scope) || zedRemoteContext(document) || {}) {
    const candidates = zedRemoteFileCandidates(context, scope);
    if (candidates.length) return candidates[0].request;
    return null;
  }

  async function openZedRemote(request) {
    let nextRequest = request;
    if (!nextRequest?.ssh?.host && nextRequest?.hostId) {
      const ssh = await resolveZedRemoteHost(nextRequest.hostId);
      nextRequest = ssh ? { ...nextRequest, ssh } : nextRequest;
    }
    if (!nextRequest?.ssh?.host) {
      showZedRemoteToast(zedRemoteMissingHostMessage);
      return;
    }
    nextRequest = {
      ...nextRequest,
      strategy: nextRequest.strategy || zedRemoteOpenStrategy(),
      remember: codexPlusBackendSettings.zedRemoteProjectRegistryEnabled !== false,
    };
    try {
      const result = await postJson("/zed-remote/open", nextRequest);
      if (result?.status === "ok") {
        showZedRemoteToast("Opened in Zed Remote");
        return;
      }
      showZedRemoteToast(result?.message || "Cannot open this file in Zed Remote");
    } catch (error) {
      showZedRemoteToast(error?.message || "Cannot open this file in Zed Remote");
    }
  }

  async function openBestZedRemoteTarget() {
    const request = zedRemoteBestOpenRequest(document) || await resolveZedRemoteFallbackRequest();
    if (!request) {
      showZedRemoteToast("Cannot find a remote workspace or file for Zed");
      return;
    }
    openZedRemote(request);
  }

  function attachZedRemoteButton(candidate) {
    const anchor = candidate.node;
    if (anchor.dataset.codexZedRemoteVersion === zedRemoteOpenVersion) return;
    anchor.dataset.codexZedRemoteVersion = zedRemoteOpenVersion;
    const button = document.createElement("button");
    button.type = "button";
    button.className = zedRemoteButtonClass;
    button.textContent = "Open in Zed Remote";
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      openZedRemote(candidate.request);
    }, true);
    anchor.insertAdjacentElement("afterend", button);
  }

  function removeZedRemoteButtons() {
    document.querySelectorAll(`[data-codex-zed-remote-version]`).forEach((node) => {
      delete node.dataset.codexZedRemoteVersion;
    });
    document.querySelectorAll(`.${zedRemoteButtonClass}`).forEach((node) => node.remove());
  }

  function createZedRemoteOpenInMenuItem(referenceItem) {
    const item = document.createElement("div");
    item.className = referenceItem?.className || "no-drag text-token-foreground outline-hidden rounded-lg px-[var(--padding-row-x)] py-[var(--padding-row-y)] text-sm group hover:bg-token-list-hover-background focus:bg-token-list-hover-background cursor-interaction flex flex-col";
    item.classList.add(zedRemoteOpenInMenuItemClass);
    item.setAttribute("role", referenceItem?.getAttribute("role") || "menuitem");
    item.setAttribute("tabindex", referenceItem?.getAttribute("tabindex") || "-1");
    item.setAttribute("data-orientation", referenceItem?.getAttribute("data-orientation") || "vertical");
    item.innerHTML = `
      <div class="flex w-full items-center gap-1.5">
        <span class="inline-flex size-[18px] items-center justify-center leading-none shrink-0 opacity-75 group-focus:opacity-100 group-hover:opacity-100">
          <img alt="" class="codex-zed-open-in-menu-icon icon-sm" src="apps/zed.png">
        </span>
        <span class="flex-1 min-w-0 truncate">Zed</span>
      </div>
    `;
    bindZedRemoteOpenInMenuItem(item, "injected");
    return item;
  }

  function zedRemoteOpenInMenuActivationIsDuplicate(target) {
    if (!(target instanceof HTMLElement)) return false;
    const now = Date.now();
    const activatedAt = Number(target.dataset.codexZedOpenInMenuActivatedAt || 0);
    if (activatedAt && now - activatedAt < zedRemoteOpenInMenuActivationWindowMs) return true;
    target.dataset.codexZedOpenInMenuActivatedAt = String(now);
    return false;
  }

  async function activateZedRemoteOpenInMenuItem(event) {
    if (!codexPlusSettings().zedRemoteOpen) return;
    if (event?.type === "keydown" && !["Enter", " "].includes(event.key)) return;
    const scope = event?.currentTarget?.closest?.('[role="menu"], [data-radix-popper-content-wrapper]') || event?.currentTarget || document;
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
    if (zedRemoteOpenInMenuActivationIsDuplicate(event?.currentTarget)) return;
    const request = zedRemoteBestOpenRequest(scope) || await resolveZedRemoteFallbackRequest();
    if (!request) {
      showZedRemoteToast("Cannot find a remote workspace or file for Zed");
      return;
    }
    openZedRemote(request);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true }));
  }

  function bindZedRemoteOpenInMenuItem(item, source) {
    item.setAttribute("data-codex-zed-open-in-menu", source);
    if (item.dataset.codexZedOpenInMenuBound === zedRemoteOpenInMenuVersion) return;
    item.dataset.codexZedOpenInMenuBound = zedRemoteOpenInMenuVersion;
    item.dataset.codexZedOpenInMenuVersion = zedRemoteOpenInMenuVersion;
    item.addEventListener("pointerup", activateZedRemoteOpenInMenuItem, true);
    item.addEventListener("click", activateZedRemoteOpenInMenuItem, true);
    item.addEventListener("keydown", activateZedRemoteOpenInMenuItem, true);
  }

  function removeZedRemoteOpenInMenuItems(scope = document) {
    const root = scope?.querySelectorAll ? scope : document;
    root.querySelectorAll(`.${zedRemoteOpenInMenuItemClass}, [data-codex-zed-open-in-menu="injected"]`).forEach((node) => node.remove());
  }

  function zedRemoteOpenInMenuScopes(scope = document) {
    const root = scope?.querySelectorAll ? scope : document;
    const menus = [];
    if (scope instanceof HTMLElement && scope.matches?.('[role="menu"]')) menus.push(scope);
    root.querySelectorAll?.('[role="menu"]').forEach((menu) => menus.push(menu));
    return Array.from(new Set(menus));
  }

  function refreshZedRemoteOpenInMenus(scope = document) {
    removeZedRemoteOpenInMenuItems(scope);
    if (!codexPlusSettings().zedRemoteOpen) return;
    const fallbackPayload = zedRemoteCurrentFallbackPayload();
    zedRemoteOpenInMenuScopes(scope).forEach((menu) => {
      if (!(menu instanceof HTMLElement) || isExtensionUiNode(menu)) return;
      const items = Array.from(menu.querySelectorAll('[role="menuitem"]')).filter((item) => !isExtensionUiNode(item));
      const menuText = items.map((item) => (item.textContent || "").trim()).join(" ");
      if (!/\b(VS Code|Cursor|Antigravity)\b/.test(menuText)) return;
      if (!zedRemoteBestOpenRequest(menu) && !zedRemoteIsRemoteHostId(fallbackPayload.hostId)) return;
      const existingZedItem = items.find((item) => (item.textContent || "").trim() === "Zed");
      if (existingZedItem) {
        bindZedRemoteOpenInMenuItem(existingZedItem, "native");
        return;
      }
      const referenceItem = items.find((item) => /^(VS Code|Cursor|Antigravity)$/.test((item.textContent || "").trim()));
      if (!referenceItem) return;
      referenceItem.parentElement?.appendChild(createZedRemoteOpenInMenuItem(referenceItem));
    });
  }

  async function refreshZedRemoteOpenControls(scope = document) {
    if (!codexPlusSettings().zedRemoteOpen) {
      removeZedRemoteButtons();
      removeZedRemoteOpenInMenuItems();
      return;
    }
    try {
      const status = await loadZedRemoteStatus();
      if (!status?.platformSupported || (!status.zedAppFound && !status.zedCliFound)) {
        removeZedRemoteButtons();
        removeZedRemoteOpenInMenuItems();
        return;
      }
    } catch (_) {
      removeZedRemoteButtons();
      removeZedRemoteOpenInMenuItems();
      return;
    }
    refreshZedRemoteOpenInMenus(scope);
  }

  function runScheduledZedRemoteMenuRefresh() {
    window.__codexZedRemoteMenuRefreshPending = false;
    clearTimeout(window.__codexZedRemoteMenuRefreshTimer);
    window.__codexZedRemoteMenuRefreshTimer = null;
    refreshZedRemoteOpenControls().catch(() => {
      removeZedRemoteOpenInMenuItems();
    });
  }

  function shouldRefreshZedRemoteMenus(mutations) {
    if (!codexPlusSettings().zedRemoteOpen) return false;
    if (!mutations) return true;
    return mutations.some((mutation) => {
      const target = mutation.target;
      if (isExtensionUiNode(target)) return false;
      if (target?.nodeType === 1 && target.matches?.('[role="menu"], [data-radix-popper-content-wrapper]')) return true;
      return [...Array.from(mutation.addedNodes), ...Array.from(mutation.removedNodes)].some((node) => node.nodeType === 1 && (
        node.matches?.('[role="menu"], [data-radix-popper-content-wrapper]') ||
        node.querySelector?.('[role="menu"], [data-radix-popper-content-wrapper]')
      ));
    });
  }

  function scheduleZedRemoteMenuRefresh(mutations) {
    if (!shouldRefreshZedRemoteMenus(mutations)) return;
    if (window.__codexZedRemoteMenuRefreshPending) return;
    window.__codexZedRemoteMenuRefreshPending = true;
    window.__codexZedRemoteMenuRefreshTimer = setTimeout(runScheduledZedRemoteMenuRefresh, 50);
  }

  function scanDeferred() {
    if (pluginPatchDisabledInRelayMode()) {
      clearPluginPatchArtifacts();
      refreshForcePluginInstallUnlockLoop();
    } else {
      const pluginUnlockStrategy = codexPluginUnlockStrategy();
      const settings = codexPlusSettings();
      logCodexPluginUnlockStrategy(pluginUnlockStrategy);
      if ((pluginUnlockStrategy === "modern" || pluginUnlockStrategy === "unknown") && settings.pluginMarketplaceUnlock) {
        const marketplaceRequestPatchStrategy = codexPluginMarketplaceRequestPatchStrategy();
        installPluginBuildFlavorFilterPatch();
        if (marketplaceRequestPatchStrategy === "bridge") {
          installPluginMarketplaceBridgePatch();
        } else if (marketplaceRequestPatchStrategy === "client") {
          installPluginMarketplaceRequestPatch();
        } else {
          installPluginMarketplaceWindowEventPatchOnly();
          installPluginMarketplaceBridgePatch();
          installPluginMarketplaceRequestPatch();
        }
      }
      unblockPluginInstallButtons();
      refreshForcePluginInstallUnlockLoop();
    }
    refreshThreadIdBadges();
    sessionRows().forEach(tryAttachButton);
    updateDeleteButtonOffsets();
    scheduleProjectMoveProjection();
    scheduleChatsSortCorrection();
    archivedPageRows().forEach(attachArchivedPageDeleteButton);
    refreshConversationView();
    installCodexServiceTierBadge();
    scheduleThreadScrollSync();
    refreshCodexModelWhitelistFromScan(window.__codexSessionDeleteLastMutations);
    schedulePluginAutoExpand();
  }

  function runScanStep(step) {
    try {
      step();
    } catch (error) {
      window.__codexSessionDeleteScanFailures = window.__codexSessionDeleteScanFailures || [];
      window.__codexSessionDeleteScanFailures.push(String(error?.stack || error));
    }
  }

  function scan() {
    runScanStep(scanLightweight);
    requestAnimationFrame(() => runScanStep(scanDeferred));
  }

  function isExtensionUiNode(node) {
    return !!node?.closest?.(`.codex-delete-toast, .codex-delete-confirm-overlay, .codex-plus-modal-overlay, .${projectMoveOverlayClass}, .${codexServiceTierBadgeClass}, .codex-zed-remote-button, .codex-zed-remote-toast, #codex-plus-menu`);
  }

  function scanRelevantSelector() {
    return [
      selectors.sidebarThread,
      '[data-app-action-sidebar-section-heading="Chats"]',
      '[data-app-action-sidebar-section-heading="Projects"]',
      '[data-codex-project-move-row="true"]',
      '[data-codex-archive-page-row="true"]',
      "[data-codex-archive-delete-all]",
      '[data-message-author-role]',
      '[data-testid="conversation-turn"]',
      '[class*="user-message"]',
      '[class*="UserMessage"]',
      ".composer-footer",
      selectors.appHeader,
      selectors.archiveNav,
      codexMenuLocalizationScopeSelector(),
      ...(pluginPatchDisabledInRelayMode() ? [] : [selectors.disabledInstallButton]),
    ].join(", ");
  }

  function nodeSelfOrAncestorMatchesScanRelevance(node) {
    if (node.nodeType !== 1) return false;
    if (isExtensionUiNode(node)) return false;
    const relevantSelector = scanRelevantSelector();
    return !!node.matches?.(relevantSelector) ||
      !!node.closest?.(relevantSelector) ||
      nodeOrAncestorLooksLikeCodexUserBubble(node);
  }

  function isScanRelevantNode(node) {
    if (node.nodeType !== 1) return false;
    if (isExtensionUiNode(node)) return false;
    return nodeSelfOrAncestorMatchesScanRelevance(node) || !!node.querySelector?.(scanRelevantSelector()) || nodeLooksLikeCodexUserBubble(node);
  }

  function isChatContentMutation(mutation) {
    const target = mutation.target;
    if (!target?.closest?.('[data-message-author-role], [data-testid="conversation-turn"], main .prose')) return false;
    return !Array.from(mutation.addedNodes).some((node) => node.nodeType === 1 && isScanRelevantNode(node)) &&
      !Array.from(mutation.removedNodes).some((node) => node.nodeType === 1 && isScanRelevantNode(node));
  }

  function shouldScheduleScan(mutations) {
    if (!mutations) return true;
    return mutations.some((mutation) => {
      if (isChatContentMutation(mutation)) return false;
      const target = mutation.target;
      if (isExtensionUiNode(target)) return false;
      if (target?.nodeType === 1 && nodeSelfOrAncestorMatchesScanRelevance(target)) return true;
      const changedNodes = [...Array.from(mutation.addedNodes), ...Array.from(mutation.removedNodes)];
      return changedNodes.some((node) => node.nodeType === 1 && isScanRelevantNode(node));
    });
  }

  function runScheduledScan() {
    window.__codexSessionDeleteScanPending = false;
    clearTimeout(window.__codexSessionDeleteScanTimer);
    window.__codexSessionDeleteScanTimer = null;
    scan();
  }

  function scheduleScan(mutations) {
    window.__codexSessionDeleteLastMutations = mutations;
    scheduleCodexPlusMenuRepair(mutations);
    scheduleZedRemoteMenuRefresh(mutations);
    schedulePluginAutoExpand();
    if (!shouldScheduleScan(mutations)) return;
    if (window.__codexSessionDeleteScanPending) return;
    window.__codexSessionDeleteScanPending = true;
    window.__codexSessionDeleteScanTimer = setTimeout(runScheduledScan, 200);
  }

  void loadBackendSettingsForStartup();
  void loadCodexServiceTierState();
  installUpstreamBranchDropdownAdapter();
  installUpstreamWorktreeNativeAdapter();
  scan();
  window.__codexProjectMoveApplyProjection = applyProjectMoveProjection;
  window.__codexProjectMoveReadProjection = readProjectMoveProjection;
  window.__codexProjectMoveTargets = projectMoveTargets;
  window.__codexProjectMoveSortChats = applyChatsSortCorrection;
  window.removeEventListener("resize", window.__codexPlusResizeHandler);
  let codexPlusResizeRafId = 0;
  window.__codexPlusResizeHandler = () => {
    cancelAnimationFrame(codexPlusResizeRafId);
    codexPlusResizeRafId = requestAnimationFrame(() => {
      sessionRows().forEach((row) => {
        const group = actionGroupFromRow(row);
        if (group) delete group.dataset.codexActionLayoutStable;
      });
      syncActionGroupsLayout();
      updateFloatingCodexPlusMenuPosition(document.getElementById(codexPlusMenuId));
      runScanStep(refreshConversationView);
    });
  };
  window.addEventListener("resize", window.__codexPlusResizeHandler);
  window.__codexSessionDeleteObserver?.disconnect();
  window.__codexSessionDeleteObserver = new MutationObserver(scheduleScan);
  window.__codexSessionDeleteObserver.observe(document.body || document.documentElement, { childList: true, subtree: true });
})();

// === 粘贴修复 (CodexPlusPlus 页面增强) ===
// 控制开关：window.__CODEX_PLUS_PASTE_FIX__ = { enabled: <bool> }
// 由 CodexPlusPlus 在启动时根据 settings.codexAppPasteFix 注入。
// 关闭时不进入 if 体，行为与原 Codex 完全一致；开启时在 document 捕获阶段
// 拦截 paste，若 text/plain 非空则阻止默认行为并调用 execCommand('insertText')
// 插入纯文本，避免 Codex 把 Word 复制的内容识别为附件。
// SENTINEL 保证多次执行（页面刷新、脚本重注入）只装一次 handler。
if (window.__CODEX_PLUS_PASTE_FIX__ && window.__CODEX_PLUS_PASTE_FIX__.enabled === true) {
  (() => {
    const SENTINEL = '__codexPasteFixInstalled__';
    if (window[SENTINEL]) return;
    window[SENTINEL] = true;

    const TAG = '[PasteFix]';

    const handler = (e) => {
      const cd = e.clipboardData;
      if (!cd) return;

      const text = cd.getData('text/plain');
      if (typeof text !== 'string' || text.length === 0) return;

      e.preventDefault();
      e.stopImmediatePropagation();

      let ok = false;
      try {
        ok = document.execCommand('insertText', false, text);
      } catch (err) {
        console.warn(TAG, 'execCommand threw:', err && err.message);
      }
      if (!ok) {
        console.warn(TAG, 'execCommand failed; please paste again');
      }
    };

    document.addEventListener('paste', handler, { capture: true });
    console.log(TAG, 'paste handler installed (capture phase)');
  })();
}
