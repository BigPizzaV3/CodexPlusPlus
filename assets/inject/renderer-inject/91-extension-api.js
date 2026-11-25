  /**
   * 对外接口层：window.codexPlus
   *
   * 第三方用户脚本不认识 Codex++ 内部的闭包函数，只能通过这个对象调用能力。
   * 设计要点：
   *
   *   1. 只挂一个全局名。之前 42 个 window.__codexPlus* 里绝大多数是补丁哨兵，
   *      对外没有价值；新能力统一收进这里，避免命名空间继续发散。
   *   2. 注册表持久、DOM 瞬态。每个 register* 只把数据写进注册中心，渲染由消费方
   *      负责。第三方不需要关心宿主何时重建（overlay 重开、会话行重建）。
   *   3. 失败隔离。第三方回调一律经 runCodexPlusExtensionCallback 包一层，抛错
   *      记进该脚本的状态通道，不会让 Codex++ 自己的 UI 白屏。
   *
   * 这个分片必须在 renderer-inject 内部的所有 UI 消费方之前执行，因为它只做定义、
   * 不读 DOM；实际挂载发生在 99-tail 之前，那时所有依赖函数都已可用。
   */
  const codexPlusExtensionApiVersion = 1;
  const codexPlusExtensionAdapterVersion = "1.2.0";

  /** 类名与属性契约。一旦发布不再更名，新增用新名字。 */
  const codexPlusExtensionConstants = {
    pageClass: codexPlusPageClass,
    pageNavAttribute: "data-codex-plus-page-nav",
    railSelector: codexPlusRailSelector,
    railDestinationSelector: codexPlusRailDestinationSelector,
    actionGroupClass,
    moreMenuClass,
    toastClass: "codex-delete-toast",
    // 拓展自己插入的节点必须带这个属性，值是该脚本的 key。
    // 扫描调度靠它把拓展的写入排除在自喂循环之外（issue #1960）。
    extensionAttribute: "data-codex-plus-ext",
  };

  /**
   * 路由白名单。未在此声明的路由即使后端支持也不允许拓展调用。
   *
   * 刻意做成白名单而不是黑名单：新增路由时默认不可用，需要显式决定是否开放，
   * 避免内部路由（例如 `/settings/set`、`/manager/open-transient`）被顺手暴露出去。
   */
  const codexPlusExtensionRoutes = new Set([
    "/diagnostics/log",
    "/thread-usage-history",
    "/usage/history",
    "/usage/executions",
    "/usage/execution-detail",
    // Daily 固定 schema 的消费者封存；写入只影响其独立数据库。
    "/usage/archive",
    "/archived-thread",
    "/export-markdown",
    "/user-scripts/list",
    // 显式开放：给拓展注入 overlay 等独立 Codex 页面，不暴露任意 CDP 方法。
    "/cdp/targets",
    "/cdp/evaluate",
    // 只读：系统鼠标位置，写「桌宠跟随鼠标」这类功能用。
    "/system/cursor-position",
    // 只读：最近已完成的模型请求（模型、耗时、供应商、token、成败），写「请求提示」这类功能用。
    "/request-log/recent",
    // 只读：进行中的模型请求（阶段、首字耗时、重试），与内置请求状态条同源。
    "/proxy-status",
    // 只读：当前会话最近的 token 事件（读 rollout 尾部，可秒级轮询），与内置用量状态栏同源。
    "/thread-usage-live",
    // 只读：Codex++ 版本与连接状态。
    "/backend/status",
    // 只读：设置的公开子集（当前供应商的名称、模式、模型、上下文窗口），不含地址、Key 与配置内容。
    "/settings/public",
    // 只读：网关供应商的配置价，不含地址、Key 或其他设置。
    "/provider-prices/public",
    // 只读：模型目录（模型列表与每模型元数据，含上下文窗口），去掉了来源地址与本机路径。
    "/model-catalog/public",
    // Agent 桌宠（官方拓展包）的数据服务：图片 / 音效下载、大肥鱼的素材、挂件数据、本机用量。
    // 只有启用中的拓展在清单里声明了才干活；供应商 Key 只在 Codex++ 里用。
    "/agent-pet/catalog",
    "/agent-pet/image",
    "/agent-pet/sound",
    "/agent-pet/import-sound",
    "/whale/media",
    "/whale/full",
    "/whale/history",
    "/whale/session",
    // 背景图（官方拓展包）的读图路由：只读这个拓展配置里的那张本机图片。
    "/image-overlay/image",
  ]);

  /** 单次调用的默认超时，略短于桥接自身的 26s，让拓展先拿到可读的错误。 */
  const codexPlusExtensionCallTimeoutMs = 26000;

  /**
   * 只读轮询路由的合并窗口（毫秒）。
   *
   * 拓展面板普遍用 setInterval 轮询这几个只读接口，每个面板各轮一次就是一次独立的桥
   * 往返，而后端每条请求都要走一遍 JSON 编解码。实测 /request-log/recent 58 次/分钟、
   * /thread-usage-live 37 次/分钟，几乎全是同一 payload 的重复请求。
   * 窗口内同参调用复用同一次桥请求：拓展拿到的仍是真实数据，最多晚一个窗口的延迟，
   * 后端与桥的往返次数按窗口长度下降一个量级。
   * 共用的只是那一次请求：结果每个调用方各拿一份拷贝（一方原地 sort / reverse 改不到别人），
   * 超时各算各的（先到者的 options.timeout 不套给后来的）。
   */
  const codexPlusExtensionCoalesceWindowMs = new Map([
    // 实测这些面板的轮询间隔在 1~1.5 秒，窗口必须比它长才有合并效果；
    // 对「最近请求 / 实时用量」这类只读展示，两秒的滞后看不出来。
    ["/request-log/recent", 2000],
    ["/thread-usage-live", 2000],
    ["/proxy-status", 2000],
    ["/backend/status", 2200],
    ["/thread-usage-history", 3000],
    ["/settings/public", 5000],
    ["/model-catalog/public", 30000],
    ["/cdp/targets", 5000],
  ]);

  /**
   * route + payload 指纹 → { at, windowMs, request }。request 是那一次桥请求（不带超时），只在窗口内复用，过期即重发。
   * 过期的条目到点删掉；payload 带变化值（时间戳、游标）时指纹各不相同，另设一个条数上限，超了先丢最早的。
   */
  const codexPlusExtensionCallCache = new Map();
  const codexPlusExtensionCallCacheLimit = 64;

  function codexPlusExtensionCallCacheKey(route, payload) {
    try {
      return `${route}\u0000${JSON.stringify(payload ?? {})}`;
    } catch {
      return "";
    }
  }

  /** 只在缓存里还是这一条时才删：同一个指纹可能已经换成了新一次请求。 */
  function codexPlusExtensionCallCacheDrop(cacheKey, entry) {
    if (codexPlusExtensionCallCache.get(cacheKey) === entry) codexPlusExtensionCallCache.delete(cacheKey);
  }

  function codexPlusExtensionCallCacheStore(cacheKey, entry) {
    // 后台标签页里定时器会被钳得很慢：放新条目时顺手把已经过期的清掉。
    for (const [key, cached] of codexPlusExtensionCallCache) {
      if (entry.at - cached.at >= cached.windowMs) codexPlusExtensionCallCache.delete(key);
    }
    // 先删再放：Map 按插入顺序排，超上限时丢的才是最早的那条。
    codexPlusExtensionCallCache.delete(cacheKey);
    codexPlusExtensionCallCache.set(cacheKey, entry);
    while (codexPlusExtensionCallCache.size > codexPlusExtensionCallCacheLimit) {
      codexPlusExtensionCallCache.delete(codexPlusExtensionCallCache.keys().next().value);
    }
    // 轮询停了以后不会再有新条目来触发清理，所以每条到点自己删，响应体不会一直留着。
    const timer = setTimeout(() => codexPlusExtensionCallCacheDrop(cacheKey, entry), entry.windowMs);
    // Node 测试环境里不让这个定时器拖住进程；浏览器的定时器 id 是数字，没有 unref。
    if (typeof timer?.unref === "function") timer.unref();
  }

  /** 交给调用方的结果拷贝。桥的结果都是 JSON 形状；万一拷不了，宁可共用也不让调用失败。 */
  function codexPlusExtensionCopyResult(value) {
    if (!value || typeof value !== "object") return value;
    try {
      return typeof structuredClone === "function" ? structuredClone(value) : JSON.parse(JSON.stringify(value));
    } catch {
      return value;
    }
  }

  /**
   * 调用后端。
   *
   * 直接暴露 __codexSessionDeleteBridge 有三个问题：名字语义错位（它早就不只用于
   * 会话删除）、没有超时、错误风格不统一（路由层返回 {status:"failed"}，浮层面板
   * 返回 {error}）。这里统一成 Promise reject，让拓展用 try/catch。
   */
  function codexPlusExtensionCall(route, payload = {}, options = {}) {
    if (typeof route !== "string" || !codexPlusExtensionRoutes.has(route)) {
      return Promise.reject(new Error(`未开放的路由：${route}`));
    }
    const bridge = window.__codexSessionDeleteBridge;
    if (typeof bridge !== "function") {
      return Promise.reject(new Error("Codex 页面尚未连接，请稍后重试"));
    }
    const windowMs = codexPlusExtensionCoalesceWindowMs.get(route);
    const cacheKey = windowMs ? codexPlusExtensionCallCacheKey(route, payload) : "";
    if (!windowMs || !cacheKey) return codexPlusExtensionCallOnce(route, payload, options, bridge);
    const now = Date.now();
    let entry = codexPlusExtensionCallCache.get(cacheKey);
    if (!entry || now - entry.at >= windowMs) {
      const fresh = { at: now, windowMs, request: codexPlusExtensionBridgeRequest(route, payload, bridge) };
      entry = fresh;
      codexPlusExtensionCallCacheStore(cacheKey, fresh);
      // 失败不留在缓存里：否则一次偶发错误会在一个窗口内被放大给所有轮询者。
      fresh.request.catch(() => codexPlusExtensionCallCacheDrop(cacheKey, fresh));
    }
    const shared = entry;
    return codexPlusExtensionAwaitRequest(shared.request, route, options, {
      // 有人等到超时说明这次请求卡住了：不再让后来的调用方接着等它，下一次调用重新发。
      onTimeout: () => codexPlusExtensionCallCacheDrop(cacheKey, shared),
    });
  }

  /** 发一次桥请求，不带超时。路由层的 {status:"failed"}、浮层面板的 {error} 统一成 reject。 */
  function codexPlusExtensionBridgeRequest(route, payload, bridge) {
    return Promise.resolve(bridge(route, payload)).then(
      (result) => {
        if (result?.status === "failed" || result?.error) {
          throw new Error(result.message || result.error || `${route} 调用失败`);
        }
        return result;
      },
      (error) => {
        throw error instanceof Error ? error : new Error(String(error));
      },
    );
  }

  /**
   * 按调用方自己的超时等一次桥请求。给了 shared（这次请求是合并缓存里共用的）时，结果拷一份再交出去，
   * 超时那一刻回调 shared.onTimeout。
   */
  function codexPlusExtensionAwaitRequest(request, route, options, shared = null) {
    const timeout = Number.isFinite(options?.timeout) ? Number(options.timeout) : codexPlusExtensionCallTimeoutMs;
    return new Promise((resolve, reject) => {
      let timedOut = false;
      // 桥接协议没有 cancel 通道，超时只能放弃等待，服务端任务仍会跑完。
      const timer = setTimeout(() => {
        timedOut = true;
        shared?.onTimeout();
        reject(new Error(`调用 ${route} 超时`));
      }, timeout);
      request.then(
        (result) => {
          // 已经超时放弃的调用方不用再为它拷一份。
          if (timedOut) return;
          clearTimeout(timer);
          resolve(shared ? codexPlusExtensionCopyResult(result) : result);
        },
        (error) => {
          clearTimeout(timer);
          reject(error);
        },
      );
    });
  }

  /** 真正发起一次桥调用（不做合并）。 */
  function codexPlusExtensionCallOnce(route, payload, options, bridge) {
    return codexPlusExtensionAwaitRequest(codexPlusExtensionBridgeRequest(route, payload, bridge), route, options);
  }

  /**
   * 当前脚本的 key。
   *
   * wrap_script 只在脚本初始化期间把 currentKey 设为脚本 key，异步回调里就是 null。
   * 而拓展完全可能在 await 之后才注册 UI，所以不能只看 currentKey——那样这些项会
   * 丢失归属，出问题时无法定位到是哪个脚本，扫描调度也认不出它的节点。
   * 这里保留 options.scriptKey 作为显式覆盖，默认回退到 currentKey。
   */
  function codexPlusCurrentExtensionScriptKey(options) {
    return options?.scriptKey || window.__codexPlusUserScripts?.currentKey || "";
  }

  /** 注册一个带 order 的拓展项，统一处理 id 前缀与失败上报。 */
  function codexPlusRegisterExtensionItem(kind, registry, definition, options = {}) {
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const id = `${scriptKey}:${kind}:${options.id || codexPlusExtensionIdSeed()}`;
    const dispose = registerCodexPlusExtension(kind, registry, id, definition, scriptKey);
    // 注册后立刻让所有入口重画一次，否则用户要等下一次 scan 才看得到新项。
    codexPlusRefreshExtensionHosts();
    return () => {
      dispose();
      codexPlusRefreshExtensionHosts();
    };
  }

  let codexPlusExtensionIdCounter = 0;
  function codexPlusExtensionIdSeed() {
    codexPlusExtensionIdCounter += 1;
    return `item-${codexPlusExtensionIdCounter}`;
  }

  /**
   * 通知各消费方重画。
   *
   * 每个消息都可能有消费方尚未初始化（例如浮层面板按需注入、overlay 未打开），
   * 所以逐项 try/catch，任何一个不存在或抛错都不影响其余。
   */
  function codexPlusRefreshExtensionHosts() {
    codexPlusRefreshExtensionHostsNow();
    scheduleCodexPlusExtensionHostsCatchUp();
  }

  /**
   * 页面刚加载时拓展往往先于 Codex 的界面注册：注册那一刻输入框、侧栏、顶栏还没渲染，立即刷新没有地方挂。
   * 首页静止时扫描也不会再跑，所以注册 / 撤销后再在 0.5、2、5 秒各补刷一次。连续注册共用同一组定时器；
   * 各刷新按指纹幂等，界面没变时不动 DOM。
   */
  let codexPlusExtensionHostsCatchUpTimers = [];
  function scheduleCodexPlusExtensionHostsCatchUp() {
    for (const timer of codexPlusExtensionHostsCatchUpTimers) clearTimeout(timer);
    codexPlusExtensionHostsCatchUpTimers = [500, 2000, 5000].map((delay) => setTimeout(codexPlusRefreshExtensionHostsNow, delay));
  }

  function codexPlusRefreshExtensionHostsNow() {
    for (const refresh of [
      refreshCodexPlusPageNav,
      refreshCodexPlusRailNavigation,
      refreshCodexPlusComposerActions,
      refreshCodexPlusMessageSlots,
      refreshCodexPlusSidePanels,
      refreshCodexPlusUiSlots,
      refreshExtensionSessionRows,
      refreshCodexPlusExtensionMenu,
    ]) {
      try {
        refresh?.();
      } catch {}
    }
  }

  /**
   * 已打开的菜单里补上／摘掉拓展项。
   *
   * 菜单是打开时一次性构建的 innerHTML，注册发生在它打开之后时不会自动出现。
   * 这里只处理「已打开」这一种情况：整块替换掉带 data-codex-plus-ext-menu 的容器。
   * 菜单没打开时什么都不做——下次打开自然会带上。
   */
  function refreshCodexPlusExtensionMenu() {
    const overlay = document.querySelector(".codex-plus-modal-overlay, .codex-plus-page-overlay");
    if (!overlay) return;
    const panel = overlay.querySelector('[data-codex-plus-panel="home"]');
    if (!panel) return;
    panel.querySelector("[data-codex-plus-ext-menu]")?.remove();
    const markup = renderCodexPlusExtensionMenuRows();
    // 设置页按分组分页后，拓展项放在「拓展设置」那一组里；有没有拓展项决定左栏里有没有这一组。
    const host = panel.querySelector("[data-codex-plus-ext-menu-host]") || panel;
    if (markup) host.insertAdjacentHTML("beforeend", markup);
    if (typeof refreshCodexPlusSettingsNav === "function") refreshCodexPlusSettingsNav();
  }

  /** 会话行按钮重画：让扫描在下一轮把这些行重建，从而带上拓展的项。 */
  function refreshExtensionSessionRows() {
    try {
      sessionRows().forEach((row) => {
        const group = actionGroupFromRow(row);
        if (group) delete group.dataset.codexActionLayoutStable;
        // 「…」按钮只在有菜单项时出现：拓展注册 / 注销会话行操作后立刻按新指纹
        // 重建（或摘掉）这一行的按钮组，不用等下一次扫描。
        tryAttachButton(row);
      });
      archivedPageRows().forEach(attachArchivedPageDeleteButton);
    } catch {}
  }

  /**
   * 注册一个整页视图，并自动配一个图标栏入口。
   *
   * 内置的三个页面（Codex++ / 拓展 / 推荐内容）都是「rail 入口 + 整页」的形态，
   * 第三方页面沿用同一种形态，用户才不会在弹窗里找入口。抽成独立函数是因为
   * codexPlus.registerPage 与插件内核的 ctx.registerPage 共用这一套逻辑。
   */
  function codexPlusRegisterExtensionPage(definition, options = {}) {
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const pageId = `${scriptKey}:page:${options.id || codexPlusExtensionIdSeed()}`;
    const disposePage = registerCodexPlusExtension("page", codexPlusRegistry.pages, pageId, definition, scriptKey);
    // 入口与页面成对存在：页面没了，入口也该消失，否则点了没有任何反应。
    const entry = {
      ...definition,
      label: options.navLabel || definition.navLabel || definition.title || pageId,
      icon: options.icon || definition.icon,
      pageId,
      order: Math.max(1000, Number.isFinite(options.order) ? Number(options.order) : 0),
    };
    const navId = `${scriptKey}:navEntry:${pageId}`;
    let disposeNav = null;
    try {
      // options.nav === false：只要页面、不要图标栏入口（例如从 Codex 设置页里打开的拓展分区）。
      if (options.nav !== false) disposeNav = registerCodexPlusExtension("navEntry", codexPlusRegistry.navEntries, navId, entry, scriptKey);
    } catch {
      // 入口注册失败（配额满）时页面本身仍可用，不要回滚已成功的页面注册。
    }
    codexPlusRefreshExtensionHosts();
    return () => {
      try {
        disposeNav?.();
      } catch {}
      disposePage();
      codexPlusRefreshExtensionHosts();
    };
  }

  /**
   * 构建对外对象。
   *
   * 拆成函数而不是直接字面量，是为了让 99-tail 之前的挂载点能按顺序装配：
   * 依赖的函数都已在同一闭包里，此处只做引用。
   */
  function buildCodexPlusExtensionApi() {
    return {
      version: codexPlusExtensionAdapterVersion,
      apiVersion: codexPlusExtensionApiVersion,
      /**
       * 当前宿主。同一份拓展脚本也能跑在 Claude Desktop 里（清单的 claudeDesktop.renderer），那边是 "claudeDesktop"。
       * 老版本没有这个成员，判断时写 `codexPlus.host ?? "codex"`。
       */
      host: "codex",
      /**
       * 某个接口 / 挂载点在当前宿主可不可用，例如 supports("registerComposerAction")、supports("registerSlot:composer.above")。
       * Codex 里公开接口都有；Claude Desktop 里有些挂载点没有对应位置，先问一下再注册。
       */
      supports(name) {
        const [member, detail] = String(name || "").split(":");
        if (member === "registerSlot" && detail !== undefined) return codexPlusUiSlotNames.includes(detail);
        // 从挂好的对象上查，不靠 this：拓展可能把 supports 解构出来单独调。
        return member !== "dshClient" && typeof window.codexPlus?.[member] === "function";
      },
      constants: codexPlusExtensionConstants,
      // 用 getter 而不是快照：脚本初始化结束后再读也能拿到自己的 key。
      get script() {
        const key = codexPlusCurrentExtensionScriptKey();
        return { key, manifest: codexPlusExtensionManifest(key) };
      },

      /** 创建一个拓展实例：ctx 上的注册都随实例卸载自动撤销，见 91-extension-runtime.js。 */
      extension: codexPlusCreateExtension,
      /** 早期名称。已有拓展（包括对话里生成的）在用，保留为别名。 */
      plugin: codexPlusCreateExtension,

      /** 当前已提供的服务：[{ name, owner }]。取值请在拓展里 inject 或 ctx.get。 */
      services() {
        return [...codexPlusServices.entries()].map(([name, entry]) => ({ name, owner: entry.owner }));
      },

      /**
       * 在拓展实例里运行 dsh 前端插件（由 Codex++ 生成的 dsh 配套拓展包调用）：modules 是截住的
       * __ModuleLoader__.load 参数，dsh 插槽映射到 Codex++ 的插槽，见 96-dsh-client.js。
       */
      dshClient(ctx, modules, options) {
        return codexPlusRunDshClient(ctx, modules, options);
      },

      /** 当前打开的 Codex 会话 id；首页、新对话还没有 id 时为空串。 */
      currentSessionId() {
        return codexPlusCurrentSessionId();
      },
      /**
       * 当前打开的 Codex 会话：{ id, title, hostId, remote }。hostId 是会话所在的主机（本机为 "local"），
       * remote 表示远程 / SSH 会话（本机会话日志里没有它的用量）。没有会话时 id 为空串。
       */
      currentSession() {
        const ref = typeof currentSessionRef === "function" ? currentSessionRef() || {} : {};
        const hostId = String(ref.host_id || "local");
        return Object.freeze({ id: String(ref.session_id || ""), title: String(ref.title || ""), hostId, remote: hostId !== "local" });
      },

      /** 内置拓展的现状：[{ id, name, active, disabled, replacedBy }]。见 96-builtin-extensions.js。 */
      builtins() {
        return codexPlusBuiltinExtensionList();
      },

      /** 显示提示。type: info | success | warn | error */
      toast(message, options) {
        return runCodexPlusExtensionCallback(
          codexPlusCurrentExtensionScriptKey(),
          "toast",
          () => showToast(String(message ?? ""), options || {}),
        );
      },

      /** 调用后端白名单路由，失败时 reject。 */
      call: codexPlusExtensionCall,

      /**
       * 注册斜杠命令：在输入框输入「/name 参数」回车时调用 run(args, { editor, insertText, setText, toast })，
       * 输入框随后清空；输入「/」时在输入框上方提示匹配的拓展命令，Tab 补全。
       */
      registerCommand(definition, options = {}) {
        return codexPlusRegisterExtensionCommand(definition, options);
      },

      /**
       * 在输入区（发送按钮所在那一行）加一个图标按钮。
       * onActivate({ editor, insertText, close }) 中 insertText(text) 把文字插入当前输入框。
       */
      registerComposerAction(definition, options = {}) {
        return codexPlusRegisterExtensionItem("composerAction", codexPlusRegistry.composerActions, definition, options);
      },

      /**
       * 在每条对话消息的操作行（复制、编辑按钮旁）加一个图标按钮。
       * roles 限定 "user" / "assistant"；visible(message) 返回 false 时这条消息不显示；
       * onActivate({ message, close }) 里 message = { role, id, turnId, conversationId, text, element }。
       */
      registerMessageAction(definition, options = {}) {
        return codexPlusRegisterExtensionItem("messageAction", codexPlusRegistry.messageActions, definition, options);
      },

      /**
       * 在每条对话消息的正文下方挂一块内容：render({ message, container }) 往 container 里画。
       * 消息文字变化（流式输出）时会清空 container 重新调用；container 留空则不占位。
       */
      registerMessageDecoration(definition, options = {}) {
        return codexPlusRegisterExtensionItem("messageDecoration", codexPlusRegistry.messageDecorations, definition, options);
      },

      /**
       * 注册右侧面板：顶栏右侧出现一个开关按钮，打开时 Codex 主区域往左让出面板宽度。
       * render({ container, close }) 每次打开都调用，可返回清理函数（关闭时调用）。
       * 返回的撤销函数上带 open() / close() / toggle()。
       */
      registerSidePanel(definition, options = {}) {
        return codexPlusRegisterExtensionSidePanel(definition, options);
      },

      /**
       * 往 Codex 界面的固定位置放内容：slot 为 composer.above、thread.top、thread.bottom、sidebar.top、
       * sidebar.bottom、titlebar。render({ container, slot }) 在容器新建时调用（Codex 重建 DOM 后会再调），
       * 可返回清理函数。返回的撤销函数上带 refresh() 强制重画。
       */
      registerInputTrigger(definition, options = {}) {
        return codexPlusRegisterInputTrigger(definition, options);
      },
      registerShortcut(definition, options = {}) {
        return codexPlusRegisterShortcut(definition, options);
      },
      registerTheme(definition, options = {}) {
        return codexPlusRegisterTheme(definition, options);
      },
      ask(question) {
        return codexPlusAsk(question);
      },
      registerSlot(definition, options = {}) {
        return codexPlusRegisterExtensionSlot(definition, options);
      },
      /** 右下角悬浮球面板里加一个页签：render({ container, close })，可返回清理函数。 */
      registerFloatingTab(definition, options = {}) {
        return codexPlusRegisterFloatingTab(definition, options);
      },
      /** 对话里的工具调用 / 命令条目末尾挂内容：tools / kinds 过滤，render({ container, call })。 */
      registerToolView(definition, options = {}) {
        return codexPlusRegisterToolView(definition, options);
      },
      /** Codex++ 设置页左栏加一组，选中时 render({ container, close }) 画右边内容，可返回清理函数（切走或关页时调用）。 */
      registerSettingsSection(definition, options = {}) {
        return codexPlusRegisterSettingsSection(definition, options);
      },

      /**
       * 「拓展」页面里本拓展详情下方的设置面板（每个拓展一个）：render({ container, config, saveConfig }) 每次打开详情都调用，
       * 可返回清理函数。saveConfig(values) 只存本拓展清单 config 里声明过的项，回生效配置。
       */
      registerSettingsPanel(definition, options = {}) {
        return codexPlusRegisterSettingsPanel(definition, options);
      },

      /** 在会话行「更多操作」里加一项。 */
      registerRowAction(definition, options = {}) {
        return codexPlusRegisterExtensionItem("rowAction", codexPlusRegistry.rowActions, definition, options);
      },

      /** 加一个图标栏入口（点击后走 registerPage 注册的页面）。 */
      registerNavEntry(definition, options = {}) {
        return codexPlusRegisterExtensionItem("navEntry", codexPlusRegistry.navEntries, definition, options);
      },

      /**
       * 在 Codex++ 菜单的「主页」面板里加一行。
       *
       * 两种形态，按 definition 里给的字段决定：
       *   - 开关：给 `onChange(next)`，可选 `toggleValue()` 提供当前值
       *   - 按钮：给 `onActivate({ close })`
       *
       * 这些是 Codex++ 自己的设置面板，改动会立刻反映到当前打开的菜单上；
       * 菜单重新打开时会从 `toggleValue()` 重新读一次状态。
       */
      registerMenuItem(definition, options = {}) {
        return codexPlusRegisterExtensionItem("menuItem", codexPlusRegistry.menuItems, definition, options);
      },

      /**
       * 注册一个整页视图。
       *
       * 同时自动配一个图标栏入口——内置的三个页面（Codex++ / 拓展 / 推荐内容）
       * 都是「rail 入口 + 整页」的形态，第三方页面沿用同一种形态，用户才不会
       * 在弹窗里找入口。`options.navLabel` / `options.icon` 控制入口外观。
       *
       * render 每次打开都被重新调用，不要缓存 DOM（见本文件顶部的生命周期约定）。
       */
      registerPage(definition, options = {}) {
        return codexPlusRegisterExtensionPage(definition, options);
      },

      /** 注册清理函数，热重载时逆序执行。 */
      onCleanup(cleanup) {
        return window.__codexPlusUserScripts?.registerCleanup?.(cleanup);
      },

      /** 主动上报失败，供异步阶段的错误使用（同步阶段由 wrap_script 捕获）。 */
      fail(error) {
        codexPlusMarkExtensionFailure(
          window.__codexPlusUserScripts?.currentKey,
          String(error?.stack || error?.message || error),
        );
      },
    };
  }
