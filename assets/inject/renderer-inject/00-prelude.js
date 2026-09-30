(() => {
  // The launcher targets the Codex app page, but keep a renderer-side guard
  // so this bundle cannot create UI in embedded browser documents.
  const codexPlusIsNodeTestHarness = typeof process === "object" && !!process.versions?.node;
  if (!codexPlusIsNodeTestHarness && (window.top !== window || window.self !== window || !window.electronBridge || !/^app:\/\/\-\//i.test(window.location.href))) return;
  const codexPlusIsWindowsPlatform = /\bWindows\b/i.test(navigator.userAgent || "");

  function installCodexPlusFastStartup() {
    const config = window.__CODEX_PLUS_FAST_STARTUP__;
    if (!config || config.enabled !== true) return;
    if (window.__codexPlusFastStartupInstalled === "1") return;
    window.__codexPlusFastStartupInstalled = "1";
    const timeoutMs = Math.max(100, Math.min(Number(config.statsigTimeoutMs) || 800, 3000));
    const statsigHosts = new Set([
      "ab.chatgpt.com",
      "featureassets.org",
      "prodregistryv2.org",
      "api.statsigcdn.com",
      "statsigapi.net",
      "cloudflare-dns.com",
    ]);

    const isStatsigUrl = (input) => {
      try {
        const url = new URL(typeof input === "string" ? input : input?.url ?? "", window.location.href);
        return statsigHosts.has(url.hostname);
      } catch {
        return false;
      }
    };

    const timeoutSignal = (signal) => {
      const controller = new AbortController();
      const timer = window.setTimeout(() => controller.abort(), timeoutMs);
      const clear = () => window.clearTimeout(timer);
      if (signal) {
        if (signal.aborted) controller.abort();
        else signal.addEventListener("abort", () => controller.abort(), { once: true });
      }
      return { signal: controller.signal, clear };
    };

    const patchFetch = () => {
      if (typeof window.fetch !== "function" || window.fetch.__codexPlusFastStartupPatched) return;
      const originalFetch = window.fetch.bind(window);
      const patchedFetch = (input, init = undefined) => {
        if (!isStatsigUrl(input)) return originalFetch(input, init);
        const { signal, clear } = timeoutSignal(init?.signal);
        const nextInit = { ...(init || {}), signal };
        return originalFetch(input, nextInit).finally(clear);
      };
      patchedFetch.__codexPlusFastStartupPatched = true;
      window.fetch = patchedFetch;
    };

    const markStatsigReady = (client) => {
      if (!client || typeof client !== "object" || client.__codexPlusFastStartupReadyPatched) return;
      client.__codexPlusFastStartupReadyPatched = true;
      const markReady = () => {
        try {
          if (client.loadingStatus && client.loadingStatus !== "Ready") client.loadingStatus = "Ready";
        } catch {
        }
        try {
          if (typeof client.$emt === "function") client.$emt({ name: "values_updated" });
        } catch {
        }
      };
      if (typeof client.initializeAsync === "function") {
        const originalInitializeAsync = client.initializeAsync.bind(client);
        client.initializeAsync = (...args) => Promise.race([
          originalInitializeAsync(...args).catch(() => null),
          new Promise((resolve) => window.setTimeout(() => resolve(null), timeoutMs)),
        ]).finally(markReady);
      }
      markReady();
    };

    const statsigClients = () => {
      const root = window.__STATSIG__ || globalThis.__STATSIG__;
      if (!root || typeof root !== "object") return [];
      const clients = [root.firstInstance, typeof root.instance === "function" ? root.instance() : null];
      if (root.instances && typeof root.instances === "object") clients.push(...Object.values(root.instances));
      return clients.filter((client, index, array) => client && typeof client === "object" && array.indexOf(client) === index);
    };

    const patchStatsigRoot = () => statsigClients().forEach(markStatsigReady);

    patchFetch();
    patchStatsigRoot();
    const startedAt = Date.now();
    const timer = window.setInterval(() => {
      patchFetch();
      patchStatsigRoot();
      if (Date.now() - startedAt > 5000) window.clearInterval(timer);
    }, 50);
  }

  function installCodexPlusForceChineseLocale() {
    const config = window.__CODEX_PLUS_FORCE_CHINESE_LOCALE__;
    if (!config) return;
    const enabled = config.enabled === true;
    const locale = typeof config.locale === "string" && config.locale ? config.locale : "zh-CN";
    const installationKey = `2:${enabled ? "on" : "off"}:${locale}`;
    if (window.__codexPlusForceChineseLocaleInstalled === installationKey) return;
    window.__codexPlusForceChineseLocaleInstalled = installationKey;
    const languages = [locale, "zh", "en-US", "en"];
    const managedLocaleStorageKey = "codexPlus.forceChineseLocale.managed.v1";
    const localeReloadStorageKey = "codexPlus.forceChineseLocale.reload.v1";

    const readManagedLocale = () => {
      try {
        const value = JSON.parse(window.localStorage.getItem(managedLocaleStorageKey) || "null");
        return value && typeof value === "object" ? value : null;
      } catch {
        return null;
      }
    };

    const writeManagedLocale = (value) => {
      try {
        if (value) {
          window.localStorage.setItem(managedLocaleStorageKey, JSON.stringify(value));
        } else {
          window.localStorage.removeItem(managedLocaleStorageKey);
        }
      } catch {
      }
    };

    const waitForElectronBridge = () => new Promise((resolve) => {
      const startedAt = Date.now();
      const check = () => {
        const bridge = window.electronBridge;
        if (bridge && typeof bridge.sendMessageFromView === "function") {
          resolve(bridge);
          return;
        }
        if (Date.now() - startedAt >= 5000) {
          resolve(null);
          return;
        }
        window.setTimeout(check, 50);
      };
      check();
    });

    const callCodexSettingApi = (bridge, method, params) => new Promise((resolve, reject) => {
      const requestId = typeof crypto?.randomUUID === "function"
        ? crypto.randomUUID()
        : `codex-plus-locale-${Date.now()}-${Math.random().toString(16).slice(2)}`;
      let timeout;
      const cleanup = () => {
        window.clearTimeout(timeout);
        window.removeEventListener("message", onMessage);
      };
      const onMessage = (event) => {
        const message = event?.data;
        if (!message || message.type !== "fetch-response" || message.requestId !== requestId) return;
        cleanup();
        if (message.responseType !== "success") {
          reject(new Error(message.error || `Codex ${method} failed`));
          return;
        }
        try {
          resolve(JSON.parse(message.bodyJsonString || "null"));
        } catch (error) {
          reject(error);
        }
      };
      window.addEventListener("message", onMessage);
      timeout = window.setTimeout(() => {
        cleanup();
        reject(new Error(`Codex ${method} timed out`));
      }, 5000);
      const message = {
        type: "fetch",
        requestId,
        method: "POST",
        url: `vscode://codex/${method}`,
        body: JSON.stringify({ params }),
      };
      Promise.resolve(bridge.sendMessageFromView(message)).catch((error) => {
        cleanup();
        reject(error);
      });
    });

    const reloadAfterLocaleChange = (value) => {
      const marker = JSON.stringify(value);
      try {
        if (window.sessionStorage.getItem(localeReloadStorageKey) === marker) return;
        window.sessionStorage.setItem(localeReloadStorageKey, marker);
        // 标记写不进去就不要刷新，否则下次加载读不到标记，会再次刷新。
        if (window.sessionStorage.getItem(localeReloadStorageKey) !== marker) return;
      } catch {
        return;
      }
      window.location.reload();
    };

    const clearLocaleReloadMarker = () => {
      try {
        window.sessionStorage.removeItem(localeReloadStorageKey);
      } catch {
      }
    };

    const syncOfficialLocaleSetting = async () => {
      const managed = readManagedLocale();
      if (!enabled && !managed) return;
      const bridge = await waitForElectronBridge();
      if (!bridge) return;
      const response = await callCodexSettingApi(bridge, "get-setting", { key: "localeOverride" });
      const currentValue = response?.value ?? null;

      if (enabled) {
        if (currentValue === locale) {
          clearLocaleReloadMarker();
          return;
        }
        if (!managed) {
          writeManagedLocale({ appliedLocale: locale, previousValue: currentValue });
        }
        await callCodexSettingApi(bridge, "set-setting", { key: "localeOverride", value: locale });
        reloadAfterLocaleChange(locale);
        return;
      }

      if (currentValue !== managed.appliedLocale) {
        writeManagedLocale(null);
        clearLocaleReloadMarker();
        return;
      }
      const previousValue = managed.previousValue ?? null;
      await callCodexSettingApi(bridge, "set-setting", {
        key: "localeOverride",
        value: previousValue,
      });
      writeManagedLocale(null);
      reloadAfterLocaleChange(previousValue);
    };

    syncOfficialLocaleSetting().catch(() => {});
    if (!enabled) return;

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

  installCodexPlusFastStartup();
  installCodexPlusForceChineseLocale();

  const helperBase = window.__CODEX_SESSION_DELETE_HELPER__ || "http://127.0.0.1:57321";
  const buttonClass = "codex-delete-button";
  const exportButtonClass = "codex-export-button";
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
  const sessionCopyMenuItemClass = "codex-session-copy-menu-item";
  const sessionCopyMenuItemVersion = "1";
  const sessionCopyMenuActivationTimeoutMs = 12000;
  const sessionShareButtonClass = "codex-session-share-button";
  const sessionShareButtonVersion = "1";
  const codexPlusShareBaseUrl = "https://share.codexpp.cc";
  const codexPlusShareFallbackBaseUrl = "https://codexpp-share.pages.dev";
  const codexPlusShareMaxCharacters = 900000;
  const sessionAutoRenameTimeoutMs = 20000;
  const zedRemoteToastClass = "codex-zed-remote-toast";
  const upstreamWorktreeDialogClass = "codex-upstream-worktree-dialog";
  const upstreamBranchOptionAttribute = "data-codex-upstream-branch-option";
  const upstreamBranchSelectionKey = "codexUpstreamBranchSelection";
  const upstreamProjectContextKey = "codexUpstreamProjectContext";
  const zedRemoteOpenInMenuVersion = "1";
  const zedRemoteOpenInMenuActivationWindowMs = 600;
  const styleId = "codex-delete-style";
  // 改 10-style.js 里的任何 CSS 都要把它 +1：installStyle 靠这个版本号判断
  // 页面里已有的 <style> 是否过期，不升的话新样式在旧标签存在时会被直接跳过。
  const codexDeleteStyleVersion = "23";
  const codexPlusMenuId = "codex-plus-menu";
  const codexPlusMenuFloatingClass = "codex-plus-menu-floating";
  const codexPlusSidebarNavId = "codex-plus-sidebar-nav";
  const codexPlusPageClass = "codex-plus-page-overlay";
  // 新版 Codex 在最左侧多出一条导航图标栏（navigation rail）。
  // 三个入口分别挂进去：Codex++ 主页、「拓展」（原用户脚本）和「推荐内容」。
  // 三者各自是一个独立页面，不再作为弹窗里的二级 tab。
  const codexPlusRailNavId = "codex-plus-rail-nav";
  const codexPlusRailExtensionsId = "codex-plus-rail-extensions";
  const codexPlusRailSponsorId = "codex-plus-rail-sponsor";
  const codexPlusRailSelector = "nav[data-app-navigation-rail]";
  const codexPlusRailDestinationSelector = "[data-sidebar-destination]";
  const codexPlusExtensionsTab = "extensions";
  const codexPlusSponsorTab = "sponsor";
  // Codex 的界面缩放是给内层布局节点设 CSS zoom，不是改 documentElement。
  // 我们的 overlay 挂在 body 下、落在那棵缩放子树之外，只能自己读这个变量跟随。
  const codexPlusWindowZoomVar = "--codex-window-zoom";
  const codexDeleteVersion = "7";
  const codexExportVersion = "1";
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
  let lastSessionActionTrigger = null;
  const codexPlusSettingsKey = "codexPlusSettings";
  const codexThreadScrollKey = "codexThreadScroll";
  const codexThreadServiceTierKey = "codexThreadServiceTierOverrides";
  const codexThreadServiceTierMaxEntries = 120;
  const codexThreadServiceTierDraftBindWindowMs = 60 * 1000;
  const codexServiceTierRequestOverrideVersion = "9";
  const codexAppServerModelRequestPatchVersion = "9";
  const codexAppServerClientCaptureMarker = "AppServerRequestClient is missing a message dispatcher";
  const codexAppServerClientCaptureAnchor = "async sendRequest(";
  const codexRemoteSessionRecoveryVersion = "5";
  const codexPluginMarketplaceUnlockVersion = "15";
  const codexThreadScrollMaxEntries = 120;
  const codexThreadScrollSaveThrottleMs = 120;
  const codexThreadScrollRestoreWindowMs = 3200;
  const codexThreadScrollRestoreDelaysMs = [0, 80, 220, 500, 1000, 1800, 2800];
  const codexThreadScrollUserIntentWindowMs = 1200;
  const codexThreadScrollProgrammaticGuardVersion = "dispatcher:2";
  const codexThreadScrollRouteHooksVersion = "dispatcher:2";
  const codexThreadScrollListenerVersion = "4";
  const codexThreadScrollUserIntentVersion = "dispatcher:2";
  const codexPlusImageOverlayId = "codex-plus-image-overlay";
  const codexPlusDreamSkinStyleId = "codex-dream-skin-style";
  const codexPlusDreamSkinPlatform = String(window.__CODEX_PLUS_DREAM_SKIN_PLATFORM__ || "macos");
  const codexPlusDreamSkinRevision = String(window.__CODEX_PLUS_DREAM_SKIN_REVISION__ || "1");
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
    const fitMode = ["fill", "fit", "stretch", "tile", "center"].includes(config.fitMode)
      ? config.fitMode
      : "fit";
    const fitStyles = {
      fill: { size: "cover", position: "center center", repeat: "no-repeat" },
      fit: { size: "contain", position: "center center", repeat: "no-repeat" },
      stretch: { size: "100% 100%", position: "center center", repeat: "no-repeat" },
      tile: { size: "auto", position: "left top", repeat: "repeat" },
      center: { size: "auto", position: "center center", repeat: "no-repeat" },
    }[fitMode];
    const overlay = existing?.tagName === "DIV" ? existing : document.createElement("div");
    if (existing && existing !== overlay) existing.remove();
    overlay.id = codexPlusImageOverlayId;
    overlay.setAttribute("aria-hidden", "true");
    Object.assign(overlay.style, {
      position: "fixed",
      inset: "0",
      width: "100vw",
      height: "100vh",
      backgroundImage: `url("${source.replace(/"/g, "%22")}")`,
      backgroundSize: fitStyles.size,
      backgroundPosition: fitStyles.position,
      backgroundRepeat: fitStyles.repeat,
      opacity: String(opacity),
      pointerEvents: "none",
      zIndex: "2147483646",
      userSelect: "none",
    });
    if (!overlay.parentElement) root.appendChild(overlay);
    sendCodexPlusDiagnostic("image_overlay_installed", {
      opacity,
      fitMode,
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
    appHeader: '[class*="ApplicationMenuTopBar"], .app-header-tint',
    archiveNav: 'button[aria-label="已归档对话"], button[aria-label="Archived conversations"]',
    disabledInstallButton: 'button:disabled, button[aria-disabled="true"], [role="button"][aria-disabled="true"], button[data-disabled], [role="button"][data-disabled], button.cursor-not-allowed, [role="button"].cursor-not-allowed, button.pointer-events-none, [role="button"].pointer-events-none',
    pluginNavButton: 'nav[role="navigation"] button.h-token-nav-row.w-full',
    pluginSvgPath: 'svg path[d^="M7.94562 14.0277"]',
  };
  const headerContextButtonClass = "border-token-border user-select-none no-drag cursor-interaction flex items-center gap-1 border whitespace-nowrap focus:outline-none disabled:cursor-not-allowed disabled:opacity-40 rounded-lg border-token-border text-token-button-tertiary-foreground bg-token-bg-fog enabled:hover:bg-token-list-hover-background data-[state=open]:bg-token-list-hover-background border h-token-button-composer px-2 py-0 text-base leading-[18px]";

