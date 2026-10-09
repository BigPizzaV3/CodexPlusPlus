  function installCodexPlusNavigationEntries() {
    // 插件入口沿用原生按钮，清理旧注入留下的重复商店入口。
    document.getElementById(codexPlusRailPluginMarketId)?.remove();
    document.getElementById(codexPlusSidebarPluginMarketId)?.remove();
    if (installCodexPlusRailNavigation()) {
      detachCodexPlusSidebarNavigation();
      return;
    }
    removeCodexPlusRailNavigation();
    installCodexPlusSidebarNavigation();
  }

  const codexPluginRemoteOnlyMarketplaceKinds = new Set(["created-by-me-remote", "shared-with-me"]);

  function pluginMarketplaceRequestProfile(params) {
    const marketplaceKinds = Array.isArray(params?.marketplaceKinds)
      ? Array.from(new Set(params.marketplaceKinds.map((kind) => restorePluginMarketplaceName(kind))))
      : [];
    const hasRemoteOnlyKind = marketplaceKinds.some((kind) => codexPluginRemoteOnlyMarketplaceKinds.has(kind));
    const hasLocalKind = marketplaceKinds.includes("local");
    const hasOtherKind = marketplaceKinds.some(
      (kind) => !codexPluginRemoteOnlyMarketplaceKinds.has(kind) && kind !== "vertical"
    );
    return {
      marketplaceKinds,
      remoteOnly: hasRemoteOnlyKind && !hasLocalKind && !hasOtherKind,
    };
  }

  function patchPluginMarketplaceRequestParams(method, params) {
    if (method === "list-plugins") {
      if (!params || typeof params !== "object") return params;
    } else {
      return params;
    }
    const next = { ...params };
    const requestProfile = pluginMarketplaceRequestProfile(next);
    const requestCwds = Array.isArray(next.cwds)
      ? next.cwds.filter((cwd) => typeof cwd === "string" && cwd.trim())
      : [];
    if (requestCwds.length > 0) {
      window.__codexPluginMarketplaceLastCwds = Array.from(new Set(requestCwds));
    } else if (!requestProfile.remoteOnly && Array.isArray(window.__codexPluginMarketplaceLastCwds) && window.__codexPluginMarketplaceLastCwds.length > 0) {
      next.cwds = [...window.__codexPluginMarketplaceLastCwds];
    }
    const hadMarketplaceKinds = Object.prototype.hasOwnProperty.call(next, "marketplaceKinds");
    const broadCatalogRequest = codexPluginUsesBroadCatalogKinds()
      && (!hadMarketplaceKinds || next.marketplaceKinds == null);
    const remoteCatalogUnavailable = window.__codexPluginMarketplaceRemoteCatalogUnavailable === true;
    if (broadCatalogRequest && !remoteCatalogUnavailable) {
      sendCodexPlusDiagnostic("plugin_marketplace_request_expanded", {
        hadMarketplaceKinds,
        marketplaceKinds: hadMarketplaceKinds ? next.marketplaceKinds : null,
        broadCatalogPreserved: true,
        cwdCount: Array.isArray(next.cwds) ? next.cwds.length : 0,
        cwdRestored: requestCwds.length === 0 && Array.isArray(next.cwds) && next.cwds.length > 0,
        remoteCatalogUnavailable,
        remoteOnly: requestProfile.remoteOnly,
      });
      return next;
    }
    let nextKinds = Array.isArray(next.marketplaceKinds)
      ? next.marketplaceKinds.map((kind) => restorePluginMarketplaceName(kind))
      : ["local"];
    if (!requestProfile.remoteOnly && remoteCatalogUnavailable) {
      nextKinds = nextKinds.filter((kind) => kind !== "created-by-me-remote" && kind !== "shared-with-me");
    }
    if (!requestProfile.remoteOnly) {
      if (!nextKinds.includes("local")) nextKinds.push("local");
      if (!nextKinds.includes("vertical")) nextKinds.push("vertical");
    }
    next.marketplaceKinds = Array.from(new Set(nextKinds));
    sendCodexPlusDiagnostic("plugin_marketplace_request_expanded", {
      hadMarketplaceKinds,
      marketplaceKinds: next.marketplaceKinds,
      broadCatalogPreserved: false,
      cwdCount: Array.isArray(next.cwds) ? next.cwds.length : 0,
      cwdRestored: requestCwds.length === 0 && Array.isArray(next.cwds) && next.cwds.length > 0,
      remoteCatalogUnavailable,
      remoteOnly: requestProfile.remoteOnly,
    });
    return next;
  }

  function displayNameForPluginMarketplaceName(name, fallback) {
    if (name === "openai-bundled") return "OpenAI插件1(Codex++)";
    if (name === "openai-curated") return "OpenAI插件2(Codex++)";
    if (name === "openai-primary-runtime") return "OpenAI插件3(Codex++)";
    if (name === "openai-api-curated") return "OpenAI插件4(Codex++)";
    // 兼容原生返回的既有旧配置名称；这里不会注入、缓存或注册插件市场。
    if (name === "codex-plus-curated" || name === "openai-curated-remote") return "OpenAI插件5(Codex++)";
    return fallback;
  }

  function patchPluginMarketplaceObject(marketplace) {
    if (!marketplace || typeof marketplace !== "object" || marketplace.__codexPlusMarketplaceUnlockPatched) return false;
    // 上游已经给了显示名就用上游的（issue #692：此前无条件用上面的中文编号覆盖，
    // 用户看到的是「OpenAI插件1(Codex++)」而不是市场真实名字）。
    // 编号映射只在市场上游确实没给显示名时兜底；去重仍走 restorePluginMarketplaceName，
    // 与显示名无关，所以不会因此退回重复条目。
    const upstreamDisplayName = marketplace.displayName || marketplace.title || marketplace.label || "";
    const displayName = upstreamDisplayName
      || displayNameForPluginMarketplaceName(marketplace.name, marketplace.name);
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

  function restorePluginMarketplaceName(name) {
    if (name === "codex-plus-openai-bundled") return "openai-bundled";
    if (name === "codex-plus-openai-curated") return "openai-curated";
    if (name === "codex-plus-openai-primary-runtime") return "openai-primary-runtime";
    if (name === "codex-plus-openai-api-curated") return "openai-api-curated";
    if (name === "codex-plus-openai-curated-remote") return "openai-curated-remote";
    return name;
  }

  function codexPluginOfficialMarketplaceName(name) {
    const restored = restorePluginMarketplaceName(name);
    return restored === "openai-bundled" || restored === "openai-curated" || restored === "openai-primary-runtime" || restored === "openai-api-curated" || restored === "openai-curated-remote";
  }

  const codexPluginFilterSourceCache = new WeakMap();

  function codexPluginFilterCallbackSource(callback) {
    if (codexPluginFilterSourceCache.has(callback)) {
      return codexPluginFilterSourceCache.get(callback);
    }
    let source = "";
    try {
      source = Function.prototype.toString.call(callback);
    } catch {
    }
    codexPluginFilterSourceCache.set(callback, source);
    return source;
  }

  // 这三种 marketplace 过滤器由 Codex 打包后压缩，标识符每版都会换名
  // （历史形态: !u(e.marketplaceName)||e.marketplaceName===r / !ne(...) / !Eu(...) /
  // 26.928.31416 起: !Mj(e.marketplaceName)||e.marketplaceName===n）。
  // 所以按「结构」而不是按字面量识别，避免每次发版都要补一个新变体。
  const codexPluginBuildFlavorFilterSourcePattern =
    /!\s*([A-Za-z_$][\w$]*)\s*\(\s*([A-Za-z_$][\w$]*)\s*\.marketplaceName\s*\)\s*\|\|\s*\2\s*\.marketplaceName\s*===\s*[A-Za-z_$][\w$]*/;
  // featuredPluginIds 那条：{let t=Gj(e);return t==null||!Mj(t)||t===n}
  // 入参是 plugin id 字符串（不是对象），先取 marketplace 再判，形态和上面不同，需单独认。
  // 三个 X 必须同一个标识符，且 `===` 右边是标识符而非调用——否则会误伤 bundle 里
  // 形如 `t==null||r==null||!ds(r)||r===SFe(t)` 的无关函数（实测存在）。
  // 右值后面必须紧跟非标识符字符（`(?![\w$(])`），光写 `(?!\s*\()` 会被贪婪回溯绕过：
  // `===SFe(t)` 里 `SFe` 可退回 `SF`，后面 `e` 不是 `(`，前瞻就放行了。实测踩过。
  const codexPluginFeaturedFilterSourcePattern =
    /([A-Za-z_$][\w$]*)\s*==\s*null\s*\|\|\s*!\s*[A-Za-z_$][\w$]*\s*\(\s*\1\s*\)\s*\|\|\s*\1\s*===\s*[A-Za-z_$][\w$]*(?![\w$(])/;

  function isCodexPluginBuildFlavorFilter(callback, sample, filtered = null) {
    if (!Array.isArray(sample) || sample.length === 0 || typeof callback !== "function") return false;
    if (!sample.some((plugin) => codexPluginOfficialMarketplaceName(plugin?.marketplaceName))) return false;
    const source = codexPluginFilterCallbackSource(callback);
    if (!source) return false;
    if (!codexPluginBuildFlavorFilterSourcePattern.test(source)) return false;
    return sample.some((plugin) => codexPluginOfficialMarketplaceName(plugin?.marketplaceName)
      && (Array.isArray(filtered) ? !filtered.includes(plugin) : !callback(plugin)));
  }

  // featuredPluginIds 过滤：sample 是字符串数组，回调按 id 反查 marketplace 后剔除官方目录。
  // 外部拿不到 id→marketplace 的映射，所以这里只做「结构 + 确实过滤掉了元素」的判定。
  function isCodexPluginFeaturedFilter(callback, sample, filtered = null) {
    if (!Array.isArray(sample) || sample.length === 0 || typeof callback !== "function") return false;
    if (!sample.every((id) => typeof id === "string")) return false;
    const source = codexPluginFilterCallbackSource(callback);
    if (!source) return false;
    if (!codexPluginFeaturedFilterSourcePattern.test(source)) return false;
    if (Array.isArray(filtered) && filtered.length >= sample.length) return false;
    return true;
  }
