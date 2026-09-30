  /**
   * 拓展注册中心。
   *
   * 第三方用户脚本通过挂到 window 上的 `codexPlus` 对象注册 UI 项，注册结果
   * 落在这里。注册中心只存数据与回调，不做任何渲染——渲染由各消费方在合适的
   * 时机读表完成。这样「内置项」和「第三方项」不会产生两条代码路径。
   *
   * 生命周期约定（很重要）：
   *   注册表持久，DOM 瞬态。
   *
   * Codex++ 的 UI 宿主会被反复重建（overlay 每次打开都清空重建、会话行按钮在
   * 版本号变化时整组重建），所以任何消费方都不能缓存 DOM 引用，必须每次从注册
   * 表读数据全量重建。反过来说，第三方脚本不需要关心 DOM 何时被销毁。
   *
   * 注意：注册中心本身不持有 DOM，也不在模块顶层读 DOM，因此可以安全地放在
   * prelude 之后的最前面——此时常量已声明，而所有顶层启动语句都还没执行。
   */
  const codexPlusRegistry = {
    rowActions: new Map(),
    navEntries: new Map(),
    pages: new Map(),
    menuItems: new Map(),
  };

  /** 每个脚本最多注册多少项、全局最多多少项，防止劣质拓展把扫描拖慢。 */
  const codexPlusExtensionPerScriptLimit = 16;
  const codexPlusExtensionGlobalLimit = 64;

  /**
   * 必须被扫描调度忽略的选择器。
   *
   * 这些节点由 Codex++ 自己（或拓展）插入到 Codex 的容器里，而容器本身是
   * scan-relevant 的。如果不排除，就会形成「写入 → 观察到自己的写入 → 200ms
   * 后再 scan → 再写入」的自喂循环：空闲时也每秒全量扫描五次，macOS 上足以
   * 吃满一个核（issue #1960）。
   *
   * 内置项在这里，拓展项通过 registerCodexPlusExtensionSelector 动态加入。
   * 拓展自带的选择器一律是 `[data-codex-plus-ext="<脚本 key>"]`，由接口层在
   * 注册时自动加上，拓展作者不需要也不应该自己维护这个列表。
   */
  const codexPlusExtensionSelectors = new Set();
  let codexPlusExtensionSelectorCache = null;

  function registerCodexPlusExtensionSelector(selector) {
    if (typeof selector !== "string" || !selector.trim()) return false;
    if (codexPlusExtensionSelectors.has(selector)) return true;
    if (codexPlusExtensionSelectors.size >= codexPlusExtensionGlobalLimit) {
      return false;
    }
    // 提前验证选择器语法：非法选择器会在 closest() 里抛错，而 closest() 跑在
    // 每次 mutation 上，一个坏选择器能把整个页面卡死。
    try {
      document.createDocumentFragment().querySelector(selector);
    } catch {
      return false;
    }
    codexPlusExtensionSelectors.add(selector);
    codexPlusExtensionSelectorCache = null;
    return true;
  }

  /**
   * 拼给 closest() 用的选择器串。Set 变化时重建、否则复用——closest() 传一个
   * 逗号串比逐个调用快得多，而这里每次 DOM 变更都会走一遍。
   */
  function codexPlusExtensionSelector() {
    if (codexPlusExtensionSelectorCache !== null) return codexPlusExtensionSelectorCache;
    codexPlusExtensionSelectorCache = [...codexPlusExtensionSelectors].join(", ");
    return codexPlusExtensionSelectorCache;
  }

  function isCodexPlusExtensionNode(node) {
    const selector = codexPlusExtensionSelector();
    if (!selector) return false;
    return !!node?.closest?.(selector);
  }

  /**
   * 注册一项通用扩展数据。返回 dispose 函数。
   *
   * 所有类别共用同一套校验与配额，免得每个 register* 各写一遍。`kind` 只用于
   * 诊断与配额统计，不参与渲染。
   */
  function registerCodexPlusExtension(kind, registry, id, definition, scriptKey) {
    if (typeof id !== "string" || !id.trim()) {
      throw new Error("拓展项 id 不能为空");
    }
    if (registry.has(id)) {
      throw new Error(`拓展项 id 已被占用：${id}`);
    }
    const owned = [...registry.values()].filter((item) => item.scriptKey === scriptKey).length;
    if (owned >= codexPlusExtensionPerScriptLimit) {
      throw new Error(`每个脚本最多注册 ${codexPlusExtensionPerScriptLimit} 项`);
    }
    if (registry.size >= codexPlusExtensionGlobalLimit) {
      throw new Error(`拓展项总数已达上限 ${codexPlusExtensionGlobalLimit}`);
    }
    // 每个类别至少要有一个可调用的钩子，否则注册进来也渲染不出东西。
    // 菜单项的开关形态是 onChange，页面/入口是 render，其余是 onActivate。
    const callbacks = ["render", "onActivate", "onChange", "onCleanup"];
    if (definition && !callbacks.some((name) => typeof definition[name] === "function")) {
      throw new Error(`拓展项 ${id} 必须提供 ${callbacks.join(" / ")} 之一`);
    }
    const order = Number.isFinite(definition?.order) ? Number(definition.order) : 0;
    // 内置项占用 0~999，第三方从 1000 起，避免插到内置项前面破坏既有布局。
    const normalized = { ...definition, kind, id, order: Math.max(1000, order), scriptKey };
    registry.set(id, normalized);
    codexPlusRegistryDiagnostics(kind, "register", id, scriptKey);
    return () => {
      if (registry.get(id) === normalized) {
        registry.delete(id);
        codexPlusRegistryDiagnostics(kind, "dispose", id, scriptKey);
      }
    };
  }

  /** 按 order 排序的注册项快照。消费方每次渲染时取，不要缓存结果。 */
  function codexPlusExtensionItems(registry) {
    return [...registry.values()].sort((left, right) => left.order - right.order);
  }

  function codexPlusRegistryDiagnostics(kind, action, id, scriptKey) {
    const entry = {
      kind,
      action,
      id,
      script_key: scriptKey || "",
      at: Date.now(),
      // 不抛错：诊断通道本身出问题时不该影响注册。
    };
    window.__codexPlusRegistryLog = window.__codexPlusRegistryLog || [];
    window.__codexPlusRegistryLog.push(entry);
    if (window.__codexPlusRegistryLog.length > 200) window.__codexPlusRegistryLog.shift();
    try {
      window.__codexSessionDeleteBridge?.("/diagnostics/log", {
        event: "extension_registry",
        detail: entry,
      })?.catch?.(() => {});
    } catch {}
  }

  /**
   * 带着归属信息执行拓展提供的回调。
   *
   * 拓展代码可能抛错、也可能返回坏数据。这里统一兜住：错误记进该脚本的状态
   * 通道（和用户脚本自身的失败上报同一个字段），并由调用方决定如何降级展示。
   * 返回值约定：成功返回 { ok: true, value }，失败返回 { ok: false, error }。
   */
  function runCodexPlusExtensionCallback(scriptKey, label, callback) {
    try {
      return { ok: true, value: callback() };
    } catch (error) {
      const message = String(error?.stack || error?.message || error);
      codexPlusMarkExtensionFailure(scriptKey, `${label}: ${message}`);
      return { ok: false, error: message };
    }
  }

  /**
   * 把拓展的失败写进用户脚本运行时状态。
   *
   * 复用 wrap_script 已经建立的上报通道：管理页读的就是
   * window.__codexPlusUserScripts.scripts[key].error。这样拓展的 UI 错误和
   * 脚本本身抛错在用户看来是同一件事，不需要第二套排查入口。
   */
  function codexPlusMarkExtensionFailure(scriptKey, message) {
    if (!scriptKey) return;
    const record = window.__codexPlusUserScripts?.scripts?.[scriptKey];
    if (record) {
      record.error = message;
      // 不覆盖 status：脚本本身可能已成功加载，失败的只是它注册的某一项 UI。
      record.extensionError = message;
    }
    window.__codexPlusExtensionFailures = window.__codexPlusExtensionFailures || [];
    window.__codexPlusExtensionFailures.push({ script_key: scriptKey, message, at: Date.now() });
    if (window.__codexPlusExtensionFailures.length > 100) window.__codexPlusExtensionFailures.shift();
  }
