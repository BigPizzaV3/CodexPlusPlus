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

    // 语言包的加载 gate 读的是 Layer 而不是 DynamicConfig（issue #2329 根因 a）：
    // 应用用 useLayer('72216192').get('enable_i18n', false) 取开关，Layer 的 memo
    // 缓存键恒为 NoValues，于是即使 DynamicConfig 被补成 enable_i18n:true 也不生效。
    // 这里给 layer 对象补上同样的取值覆盖；__value 是 Statsig 存原始值的字段，
    // 一并 assign，避免应用直接从 __value 读时绕过 get。
    const patchI18nLayer = (layer) => {
      if (!layer || typeof layer !== "object") return layer;
      const value = layer.__value && typeof layer.__value === "object" ? layer.__value : {};
      const nextValue = {
        ...value,
        enable_i18n: true,
        locale_source: "SYSTEM",
      };
      try {
        layer.__value = nextValue;
      } catch {
      }
      try {
        layer.value = nextValue;
      } catch {
      }
      if (typeof layer.get === "function" && !layer.__codexPlusForceChineseLocaleLayerPatched) {
        const originalGet = layer.get.bind(layer);
        layer.get = (key, fallback) => {
          if (key === "enable_i18n") return true;
          if (key === "locale_source") return "SYSTEM";
          return originalGet(key, fallback);
        };
        layer.__codexPlusForceChineseLocaleLayerPatched = true;
      }
      return layer;
    };

    const patchStatsigClient = (client) => {
      if (!client || typeof client !== "object") return;
      if (typeof client.getLayer === "function" && !client.__codexPlusForceChineseLocaleLayerChannelPatched) {
        const originalGetLayer = client.getLayer.bind(client);
        client.getLayer = (name, options) => {
          const result = originalGetLayer(name, options);
          return name === "72216192" ? patchI18nLayer(result) : result;
        };
        client.__codexPlusForceChineseLocaleLayerChannelPatched = true;
      }
      if (typeof client._getLayerImpl === "function" && !client.__codexPlusForceChineseLocaleLayerImplPatched) {
        const originalGetLayerImpl = client._getLayerImpl.bind(client);
        client._getLayerImpl = function (name, ...rest) {
          const result = originalGetLayerImpl(name, ...rest);
          return name === "72216192" ? patchI18nLayer(result) : result;
        };
        client.__codexPlusForceChineseLocaleLayerImplPatched = true;
      }
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
      try {
        if (typeof client.getLayer === "function") {
          patchI18nLayer(client.getLayer("72216192", { disableExposureLog: true }));
        }
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
  const codexDeleteStyleVersion = "25";
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
  const codexPluginMarketplaceUnlockVersion = "16";
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
    window.__codexPlusImageOverlayCleanup?.();
    window.__codexPlusImageOverlayCleanup = null;
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
    overlay.setAttribute("data-codex-plus-ext", "image-overlay");
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
    installCodexPlusImageOverlayForeground();
    sendCodexPlusDiagnostic("image_overlay_installed", {
      opacity,
      fitMode,
      sourceKind: source.startsWith("data:") ? "data-uri" : "unknown",
    });
  }

  function installCodexPlusImageOverlayForeground() {
    // Keep upstream's single tint layer intact. Only actual media receives a
    // foreground plane; geometry is refreshed on bounded layout events.
    const mediaSelector = "img, video, canvas";
    const raisedSelector = '[role="dialog"]:has([data-testid="image-preview-dismiss-area"]), [data-browser-sidebar-webview]';
    const blockerSelector = '[role="menu"], [role="listbox"], [role="tooltip"], [role="dialog"]';
    const records = new Map();
    const nativeHosts = new Map();
    const raised = new Map();
    const owned = node => node?.closest?.('[data-codex-plus-ext="image-overlay"]');
    const blockers = new Set();
    const dirty = new Set();
    const visibleMedia = new Set();
    const watched = new Map();
    const layoutRoots = new Map();
    let frame = 0;
    let reobserveQueued = false;
    let disposed = false;
    let frameStyles = new Map();
    let frameRects = new Map();
    const readStyle = element => {
      if (!frameStyles.has(element)) frameStyles.set(element, getComputedStyle(element));
      return frameStyles.get(element);
    };
    const readRect = element => {
      if (!frameRects.has(element)) frameRects.set(element, element.getBoundingClientRect());
      return frameRects.get(element);
    };
    const resetGeometry = () => { frameStyles = new Map(); frameRects = new Map(); };
    const style = document.createElement("style");
    style.setAttribute("data-codex-plus-ext", "image-overlay");
    style.textContent = `
      .codex-plus-media-plane {
        position: fixed !important; inset: 0 !important; margin: 0 !important;
        padding: 0 !important; border: 0 !important; background: transparent !important;
        pointer-events: none !important; user-select: none !important;
        z-index: 2147483647 !important; display: block !important;
        width: 100vw; height: 100vh;
      }
      .codex-plus-media-copy {
        position: fixed !important; inset: auto;
        left: 0; top: 0; width: 0; height: 0;
        margin: 0 !important; padding: 0 !important; border: 0 !important;
        max-width: none !important; max-height: none !important;
        pointer-events: none !important;
      }
      .codex-plus-media-plane[hidden] { display: none !important; }
      [data-codex-plus-native-media]::backdrop { display: none !important; }
    `;
    document.documentElement.appendChild(style);
    const preserveStyle = (element, name, value) => {
      const previous = element.style.getPropertyValue(name);
      const priority = element.style.getPropertyPriority(name);
      element.style.setProperty(name, value, "important");
      const applied = element.style.getPropertyValue(name);
      return () => {
        if (element.style.getPropertyValue(name) !== applied ||
            element.style.getPropertyPriority(name) !== "important") return;
        if (previous) element.style.setProperty(name, previous, priority);
        else element.style.removeProperty(name);
      };
    };
    const stopStream = record => {
      if (record.player) {
        if (record.canvasFrame != null) record.player.cancelVideoFrameCallback(record.canvasFrame);
        record.player.pause();
        record.player.srcObject = null;
      }
      if (record.ownsStream) record.stream?.getTracks().forEach(track => track.stop());
      record.stream = null;
      record.ownsStream = false;
    };
    const display = record => {
      if (disposed || records.get(record.source) !== record || !record.plane.isConnected) return;
      const visible = record.visible && record.inViewport && !record.blocked;
      if (record.native) {
        record.plane.hidden = true;
        if (document.fullscreenElement?.contains(record.source)) return;
        if (visible && !record.native.host.matches(':popover-open')) record.native.host.showPopover();
        else if (!visible && record.native.host.matches(':popover-open')) record.native.host.hidePopover();
        return;
      }
      if (record.source.tagName === 'VIDEO' && record.source.controls) {
        // Unknown compound layouts keep their usable native player. Never
        // conceal its controls with a frame copy or reparent React-owned DOM.
        record.plane.hidden = true;
        return;
      }
      if (record.nextVideoFrame) {
        if (visible && record.videoFrame == null) record.videoFrame = record.source.requestVideoFrameCallback(record.nextVideoFrame);
        else if (!visible && record.videoFrame != null) {
          record.source.cancelVideoFrameCallback(record.videoFrame);
          record.videoFrame = null;
        }
      }
      if (!visible) {
        record.plane.hidden = true;
        if (record.stream) stopStream(record);
        return;
      }
      if (record.source.tagName === "IMG" && (!record.copy.complete || !record.copy.naturalWidth)) {
        record.plane.hidden = true;
        return;
      }
      if (record.source.tagName === "CANVAS" && !record.stream) {
        try {
          const { source, copy } = record;
          if (copy.width !== source.width) copy.width = source.width;
          if (copy.height !== source.height) copy.height = source.height;
          copy.getContext("2d").drawImage(source, 0, 0);
          record.stream = record.source.captureStream?.(15);
          record.ownsStream = true;
          if (record.stream) {
            const player = record.player || (record.player = document.createElement("video"));
            player.muted = true;
            player.srcObject = record.stream;
            const paint = () => {
              if (disposed || !record.stream || !records.has(source)) return;
              if (player.readyState >= 2) {
                if (copy.width !== source.width) copy.width = source.width;
                if (copy.height !== source.height) copy.height = source.height;
                copy.getContext("2d").drawImage(player, 0, 0, copy.width, copy.height);
              }
              record.canvasFrame = player.requestVideoFrameCallback(paint);
            };
            record.canvasFrame = player.requestVideoFrameCallback(paint);
            player.play().catch(() => {});
          }
        } catch (error) {
          sendCodexPlusDiagnostic("image_overlay_video_error", { message: String(error?.message || error) });
          stopStream(record);
          record.plane.hidden = true;
          return;
        }
      }
      record.plane.hidden = false;
    };
    const visibleRect = element => {
      if (!element.isConnected || element.hidden || element.getAttribute("data-state") === "closed") return null;
      if (!element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) return null;
      const rect = readRect(element);
      return rect.width > 0 && rect.height > 0 ? rect : null;
    };
    const intersects = (a, b) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
    const drawVideo = record => {
      const { source, copy } = record;
      if (source.readyState < 2 || !record.visible || !record.inViewport || record.blocked) return;
      const width = source.videoWidth;
      const height = source.videoHeight;
      if (!width || !height) return;
      if (copy.width !== width) copy.width = width;
      if (copy.height !== height) copy.height = height;
      copy.getContext("2d").drawImage(source, 0, 0, width, height);
    };
    const nativeVideo = record => {
      const source = record.source;
      const host = source.parentElement;
      if (!source.controls || !host || host.id === 'root' || host === document.body ||
          host.childElementCount !== 1 || host.textContent.trim() || host.hasAttribute('popover') || !host.showPopover) return;
      const placeholder = document.createElement(host.tagName);
      for (const name of ['class', 'style']) {
        if (host.hasAttribute(name)) placeholder.setAttribute(name, host.getAttribute(name));
      }
      const spacer = document.createElement('video');
      for (const name of ['class', 'style', 'width', 'height']) {
        if (source.hasAttribute(name)) spacer.setAttribute(name, source.getAttribute(name));
      }
      const css = getComputedStyle(source);
      spacer.style.width = css.width;
      spacer.style.height = css.height;
      spacer.style.maxWidth = '100%';
      placeholder.append(spacer);
      placeholder.setAttribute('data-codex-plus-ext', 'image-overlay');
      placeholder.setAttribute('aria-hidden', 'true');
      placeholder.style.setProperty('visibility', 'hidden', 'important');
      placeholder.style.setProperty('pointer-events', 'none', 'important');
      host.before(placeholder);
      record.native = { host, placeholder, spacer, saved: new Map(), applied: new Map() };
      nativeHosts.set(host, record);
      host.setAttribute('data-codex-plus-native-media', '');
      host.setAttribute('popover', 'manual');
      resizeObserver.observe(placeholder);
    };
    const setNativeStyle = (record, name, value) => {
      const { native } = record;
      const host = native.host;
      if (!native.saved.has(name)) native.saved.set(name, [host.style.getPropertyValue(name), host.style.getPropertyPriority(name)]);
      host.style.setProperty(name, value, 'important');
      native.applied.set(name, host.style.getPropertyValue(name));
      native.style = host.getAttribute('style');
    };
    const refresh = source => {
      if (disposed) return;
      if (source) dirty.add(source);
      else visibleMedia.forEach(element => dirty.add(element));
      if (frame || !dirty.size) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        resetGeometry();
        const pending = [...dirty];
        dirty.clear();
        pending.forEach(update);
      });
    };
    const update = source => {
      const record = records.get(source);
      if (!record) return;
      if (record.native && document.fullscreenElement?.contains(source)) return;
      const geometry = record.native?.placeholder || source;
      const css = readStyle(geometry);
      const copy = record.copy;
      copy.style.objectFit = css.objectFit;
      copy.style.objectPosition = css.objectPosition;
      copy.style.borderRadius = css.borderRadius;
      copy.style.backgroundColor = css.backgroundColor === "rgba(0, 0, 0, 0)"
        ? "var(--color-surface, var(--color-token-bg-primary, Canvas))" : css.backgroundColor;
      if (source.tagName === "IMG") {
        const src = source.currentSrc || source.src;
        if (copy.src !== src) copy.src = src;
      }
      const rect = readRect(geometry);
      if (rect.bottom <= 0 || rect.right <= 0 || rect.left >= innerWidth || rect.top >= innerHeight) {
        record.inViewport = false;
        display(record);
        return;
      }
      record.visible = Math.max(rect.width, rect.height) >= 48 && (record.native
        ? geometry.isConnected && !geometry.closest('[hidden]') && rect.width > 0 && rect.height > 0
        : !!visibleRect(source));
      if (record.native) {
        for (let node = source; node; node = node.parentElement) {
          const native = readStyle(node);
          if (native.visibility !== 'visible' || (native.display === 'none' && node !== record.native.host) || Number(native.opacity) === 0 ||
              node.hidden || node.inert || node.getAttribute('aria-hidden') === 'true') record.visible = false;
        }
      }
      if (source.tagName === "VIDEO" && !record.native && source.readyState < 2) record.visible = false;
      record.blocked = [...blockers].some(element => {
        if (element.contains(source)) return false;
        const box = visibleRect(element);
        return box && intersects(rect, box);
      });
      const clip = { left: Math.max(0, rect.left), top: Math.max(0, rect.top),
        right: Math.min(innerWidth, rect.right), bottom: Math.min(innerHeight, rect.bottom) };
      for (let parent = geometry.parentElement; parent && parent !== document.documentElement; parent = parent.parentElement) {
        const native = readStyle(parent);
        const clipsX = /^(auto|scroll|hidden|clip)$/.test(native.overflowX);
        const clipsY = /^(auto|scroll|hidden|clip)$/.test(native.overflowY);
        if (!clipsX && !clipsY) continue;
        const box = readRect(parent);
        if (clipsX) { clip.left = Math.max(clip.left, box.left); clip.right = Math.min(clip.right, box.right); }
        if (clipsY) { clip.top = Math.max(clip.top, box.top); clip.bottom = Math.min(clip.bottom, box.bottom); }
      }
      const thread = source.closest('.thread-scroll-container');
      if (thread) {
        const footer = document.querySelector('[data-thread-scroll-footer]');
        const box = footer && visibleRect(footer);
        if (box && intersects(rect, box)) clip.bottom = Math.min(clip.bottom, box.top);
      }
      record.inViewport = rect.width > 0 && rect.height > 0 &&
        clip.right > clip.left && clip.bottom > clip.top;
      record.plane.style.clipPath = `inset(${Math.max(0, clip.top)}px ${Math.max(0, innerWidth - clip.right)}px ${Math.max(0, innerHeight - clip.bottom)}px ${Math.max(0, clip.left)}px)`;
      Object.assign(copy.style, {
        left: `${rect.left}px`,
        top: `${rect.top}px`,
        width: `${rect.width}px`,
        height: `${rect.height}px`,
      });
      if (record.native) {
        const zoom = geometry.currentCSSZoom || 1;
        for (const [name, value] of Object.entries({position:'fixed', margin:'0', inset:'auto',
          left:`${rect.left / zoom}px`, top:`${rect.top / zoom}px`, width:`${rect.width / zoom}px`, height:`${rect.height / zoom}px`,
          'max-width':'none', 'max-height':'none', 'box-sizing':'border-box', padding:css.padding, border:css.border, 'background-color':css.backgroundColor,
          'clip-path':`inset(${Math.max(0, clip.top - rect.top) / zoom}px ${Math.max(0, rect.right - clip.right) / zoom}px ${Math.max(0, rect.bottom - clip.bottom) / zoom}px ${Math.max(0, clip.left - rect.left) / zoom}px)`})) setNativeStyle(record, name, value);
      } else if (source.tagName === "VIDEO") drawVideo(record);
      display(record);
    };
    const watch = element => {
      const path = [];
      const layoutRoot = element.closest('.thread-scroll-container, [role="tabpanel"], [data-testid="image-preview-dismiss-area"]');
      if (layoutRoot) layoutRoots.set(layoutRoot, (layoutRoots.get(layoutRoot) || 0) + 1);
      for (let node = element; node; node = node.parentElement) {
        path.push(node);
        watched.set(node, (watched.get(node) || 0) + 1);
        if (watched.get(node) !== 1 && node !== layoutRoot) continue;
        attributes.observe(node, { attributes: true, subtree: layoutRoots.has(node),
          attributeFilter: ["src", "srcset", "sizes", "class", "style", "hidden", "data-state", "aria-hidden"] });
        resizeObserver.observe(node);
      }
      return { path, layoutRoot };
    };
    const unwatch = ({ path, layoutRoot }) => {
      if (layoutRoot) {
        const count = layoutRoots.get(layoutRoot) - 1;
        if (count) layoutRoots.set(layoutRoot, count);
        else layoutRoots.delete(layoutRoot);
      }
      for (const node of path) {
        const count = watched.get(node) - 1;
        if (count) watched.set(node, count);
        else { watched.delete(node); resizeObserver.unobserve(node); }
      }
      if (disposed || reobserveQueued) return;
      reobserveQueued = true;
      queueMicrotask(() => {
        reobserveQueued = false;
        if (disposed) return;
        attributes.disconnect();
        watched.forEach((_, node) => attributes.observe(node, { attributes: true, subtree: layoutRoots.has(node),
          attributeFilter: ["src", "srcset", "sizes", "class", "style", "hidden", "data-state", "aria-hidden"] }));
      });
    };
    const add = source => {
      if (records.has(source) || owned(source) || !source.isConnected ||
          [...raised.keys()].some(element => element.contains(source)) || source.getAttribute("aria-hidden") === "true") return;
      const css = getComputedStyle(source);
      if (Math.max(parseFloat(css.width) || 0, parseFloat(css.height) || 0, source.naturalWidth || 0, source.naturalHeight || 0) < 48) return;
      const plane = document.createElement("div");
      plane.className = "codex-plus-media-plane";
      plane.setAttribute("data-codex-plus-ext", "image-overlay");
      plane.setAttribute("aria-hidden", "true");
      plane.hidden = true;
      const copy = document.createElement(source.tagName === "IMG" ? "img" : "canvas");
      copy.className = "codex-plus-media-copy";
      copy.setAttribute("aria-hidden", "true");
      copy.draggable = false;
      const record = {
        source, plane, copy, stream: null, ownsStream: false, visible: true,
        inViewport: false, listeners: [], videoFrame: null,
      };
      copy.addEventListener("load", () => display(record));
      plane.appendChild(copy);
      document.documentElement.appendChild(plane);
      records.set(source, record);
      if (source.tagName === 'VIDEO') nativeVideo(record);
      visibleMedia.add(source);
      intersectionObserver.observe(record.native?.placeholder || source);
      for (const event of ["load", "loadeddata", "loadedmetadata", "canplay", "play", "playing", "pause", "seeked", "emptied"]) {
        const listener = () => refresh(source);
        source.addEventListener(event, listener);
        record.listeners.push([event, listener]);
      }
      record.watched = watch(source);
      if (source.tagName === "VIDEO" && !source.controls && !record.native && source.requestVideoFrameCallback) {
        const nextFrame = () => {
          if (disposed || !records.has(source)) return;
          record.videoFrame = null;
          if (record.visible && record.inViewport && !record.blocked) {
            drawVideo(record);
            record.videoFrame = source.requestVideoFrameCallback(nextFrame);
          }
        };
        record.nextVideoFrame = nextFrame;
        record.videoFrame = source.requestVideoFrameCallback(nextFrame);
      }
      resetGeometry();
      update(source);
    };
    const remove = source => {
      const record = records.get(source);
      if (!record) return;
      records.delete(source);
      visibleMedia.delete(source);
      intersectionObserver.unobserve(record.native?.placeholder || source);
      dirty.delete(source);
      for (const [event, listener, target = source] of record.listeners) target.removeEventListener(event, listener);
      unwatch(record.watched);
      if (record.videoFrame !== null) source.cancelVideoFrameCallback(record.videoFrame);
      stopStream(record);
      if (record.native) {
        const host = record.native.host;
        if (host.matches(':popover-open')) host.hidePopover();
        host.removeAttribute('popover');
        host.removeAttribute('data-codex-plus-native-media');
        for (const [name, [value, priority]] of record.native.saved) {
          if (host.style.getPropertyValue(name) !== record.native.applied.get(name)) continue;
          if (value) host.style.setProperty(name, value, priority);
          else host.style.removeProperty(name);
        }
        nativeHosts.delete(host);
        resizeObserver.unobserve(record.native.placeholder);
        record.native.placeholder.remove();
      }
      record.plane.remove();
    };
    const raise = element => {
      if (raised.has(element) || owned(element)) return;
      let target = element;
      for (let parent = element.parentElement; parent && parent !== document.body; parent = parent.parentElement) {
        const css = getComputedStyle(parent);
        if (element.matches('[data-browser-sidebar-webview]')) {
          if (parent.id === "root" || parent === document.documentElement) return;
          if (css.position !== "static") target = parent;
          continue;
        }
        const stacking = css.transform !== "none" || css.isolation === "isolate" ||
          css.zIndex !== "auto" || Number(css.opacity) < 1 || /paint|layout/.test(css.contain) ||
          css.filter !== "none" || css.perspective !== "none" || /^(fixed|sticky)$/.test(css.position);
        if (!stacking) continue;
        // A native portal wrapper can be lifted with its child; an app surface
        // containing unrelated UI cannot. Its media keeps the copy fallback.
        if (parent.id === "root" || parent.childElementCount !== 1) return;
        target = parent;
      }
      if (getComputedStyle(target).position === "static") return;
      if ([...raised.keys()].some(node => node.contains(element))) return;
      raised.set(element, preserveStyle(target, "z-index", "2147483647"));
      element.querySelectorAll(mediaSelector).forEach(remove);
    };
    const visit = (node, callback, selector) => {
      if (node.nodeType !== 1 || owned(node)) return;
      if (node.matches(selector)) callback(node);
      if (node.childElementCount) node.querySelectorAll(selector).forEach(callback);
    };
    const resizeObserver = new ResizeObserver(() => refresh());
    const intersectionObserver = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const target = records.has(entry.target) ? entry.target : [...records.values()].find(record => record.native?.placeholder === entry.target)?.source;
        if (!target) continue;
        if (entry.isIntersecting) visibleMedia.add(target);
        else visibleMedia.delete(target);
        refresh(target);
      }
    });
    const attributes = new MutationObserver(mutations => {
      for (const mutation of mutations) {
        if (owned(mutation.target)) continue;
        const record = nativeHosts.get(mutation.target);
        if (record?.native && mutation.attributeName === 'style' && record.native.style === mutation.target.getAttribute('style')) continue;
        if (record?.native) {
          if (mutation.attributeName === 'class') record.native.placeholder.className = record.native.host.className;
          if (mutation.attributeName === 'style') {
            for (const [name, expected] of record.native.applied) {
              const value = record.native.host.style.getPropertyValue(name);
              if (value === expected) continue;
              const priority = record.native.host.style.getPropertyPriority(name);
              record.native.saved.set(name, [value, priority]);
              if (value) record.native.placeholder.style.setProperty(name, value, priority);
              else record.native.placeholder.style.removeProperty(name);
            }
          }
        }
        invalidateLayout(mutation.target);
      }
    });
    const blockerPaths = new Map();
    const addBlocker = element => {
      if (blockers.has(element)) return;
      blockers.add(element);
      blockerPaths.set(element, watch(element));
    };
    const removeBlocker = element => {
      if (element.isConnected || !blockers.delete(element)) return;
      unwatch(blockerPaths.get(element));
      blockerPaths.delete(element);
    };
    const layoutChanged = () => refresh();
    const invalidateLayout = target => {
      for (const source of visibleMedia) {
        const record = records.get(source);
        const root = record?.watched.layoutRoot || source.parentElement;
        if (root && (root.contains(target) || target.contains?.(root) || blockers.has(target))) refresh(source);
      }
    };
    window.addEventListener("resize", layoutChanged, { passive: true });
    document.addEventListener("scroll", layoutChanged, { passive: true, capture: true });
    document.addEventListener("transitionend", layoutChanged, true);
    document.addEventListener("fullscreenchange", layoutChanged, true);
    const mediaLoaded = event => { if (event.target.matches?.(mediaSelector)) { add(event.target); refresh(event.target); } };
    document.addEventListener("load", mediaLoaded, true);
    resizeObserver.observe(document.body);
    const changes = new MutationObserver(mutations => {
      for (const mutation of mutations) {
        if (owned(mutation.target)) continue;
        invalidateLayout(mutation.target);
        for (const node of mutation.removedNodes) {
          if (node.nodeType !== 1 || owned(node)) continue;
          visit(node, source => {
            remove(source);
          }, mediaSelector);
          visit(node, element => {
            if (element.isConnected) return;
            raised.get(element)?.();
            raised.delete(element);
          }, raisedSelector);
          visit(node, removeBlocker, blockerSelector);
        }
        for (const node of mutation.addedNodes) {
          if (node.nodeType !== 1 || owned(node)) continue;
          visit(node, raise, raisedSelector);
          visit(node, add, mediaSelector);
          visit(node, addBlocker, blockerSelector);
        }
      }
    });
    document.querySelectorAll(raisedSelector).forEach(raise);
    document.querySelectorAll(blockerSelector).forEach(addBlocker);
    document.querySelectorAll(mediaSelector).forEach(add);
    changes.observe(document.body, { childList: true, subtree: true, characterData: true });
    window.__codexPlusImageOverlayCleanup = () => {
      disposed = true;
      changes.disconnect();
      window.removeEventListener("resize", layoutChanged);
      document.removeEventListener("scroll", layoutChanged, true);
      document.removeEventListener("transitionend", layoutChanged, true);
      document.removeEventListener("fullscreenchange", layoutChanged, true);
      document.removeEventListener("load", mediaLoaded, true);
      cancelAnimationFrame(frame);
      [...records.keys()].forEach(remove);
      attributes.disconnect();
      resizeObserver.disconnect();
      intersectionObserver.disconnect();
      watched.clear();
      layoutRoots.clear();
      blockerPaths.clear();
      raised.forEach(restore => restore());
      raised.clear();
      style.remove();
    };
  }

  function scheduleCodexPlusImageOverlay() {
    window.__codexPlusImageOverlayReadyCleanup?.();
    window.__codexPlusImageOverlayReadyCleanup = null;
    if (document.readyState === "loading") {
      const onReady = () => {
        window.__codexPlusImageOverlayReadyCleanup = null;
        installCodexPlusImageOverlay();
      };
      document.addEventListener("DOMContentLoaded", onReady, { once: true });
      window.__codexPlusImageOverlayReadyCleanup = () =>
        document.removeEventListener("DOMContentLoaded", onReady);
      return;
    }
    installCodexPlusImageOverlay();
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
    // 会话视图对齐的目标锚点。全部走 data-* / 结构性写法，不绑 Codex 的哈希类名，
    // 见 90-action-groups.js 的候选链说明（issue #2258）。
    conversationViewScrollContainer: ".thread-scroll-container",
    conversationViewContentAnchor: "[data-thread-user-message-navigation-content]",
    conversationViewFooter: "[data-thread-scroll-footer]",
  };
  const headerContextButtonClass = "border-token-border user-select-none no-drag cursor-interaction flex items-center gap-1 border whitespace-nowrap focus:outline-none disabled:cursor-not-allowed disabled:opacity-40 rounded-lg border-token-border text-token-button-tertiary-foreground bg-token-bg-fog enabled:hover:bg-token-list-hover-background data-[state=open]:bg-token-list-hover-background border h-token-button-composer px-2 py-0 text-base leading-[18px]";
