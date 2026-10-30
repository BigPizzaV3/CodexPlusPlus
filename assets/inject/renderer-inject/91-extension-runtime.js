  /**
   * 拓展运行时内核：codexPlus.extension(apply)
   *
   * 在 register* 之上加一层「拓展实例」：ctx 上的每一次注册（UI 项、事件监听、
   * 中间件、DOM 监听、定时器、自定义副作用）都绑定到实例的寿命上，卸载时按注册
   * 逆序自动撤销。拓展作者不必再自己维护 dispose 列表，热重载也不会残留旧实例。
   *
   * 两种拓展间耦合方式：
   *   - 事件：ctx.on / ctx.emit，一对多通知，处理器抛错互不影响
   *   - waterfall：ctx.intercept / ctx.waterfall，中间件链，handler(value, next)
   *     调 next(v) 交给下一个，不调即短路；没有中间件时原样返回
   *
   * 宿主后续会在协议层、界面层的关键点调用 codexPlusRunWaterfall /
   * codexPlusEmitExtensionEvent，拓展用同一套 ctx 接口挂上去。设计见
   * docs/plans/2026-10-30-codexpp-extension-system-design.md。
   *
   * 和 91-extension-api.js 一样只做定义、不读 DOM，挂载在 99-startup。
   */

  /** 事件名 → 监听者集合。监听者带 scriptKey，出错时记到对应脚本。 */
  const codexPlusExtensionListeners = new Map();
  /** 中间件链名 → 中间件数组，按 order、再按注册先后排序。 */
  const codexPlusExtensionInterceptors = new Map();
  let codexPlusExtensionInterceptorSeq = 0;
  /** 存活的拓展实例，供诊断查看。 */
  const codexPlusExtensionInstances = new Set();

  /** 拓展包的清单（Rust 侧写进脚本状态记录）；单文件脚本没有清单。 */
  function codexPlusExtensionManifest(scriptKey) {
    const manifest = window.__codexPlusUserScripts?.scripts?.[scriptKey]?.manifest;
    if (!manifest || typeof manifest !== "object") return null;
    // 给拓展一份副本，避免它改到状态记录里的原件。
    try {
      return JSON.parse(JSON.stringify(manifest));
    } catch {
      return null;
    }
  }

  function codexPlusIsThenable(value) {
    return !!value && typeof value.then === "function";
  }

  /** 通知所有监听者。逐个隔离：一个处理器抛错不影响其余。 */
  function codexPlusEmitExtensionEvent(event, args = []) {
    const listeners = codexPlusExtensionListeners.get(event);
    if (!listeners) return;
    for (const listener of [...listeners]) {
      runCodexPlusExtensionCallback(listener.scriptKey, `on:${event}`, () => listener.handler(...args));
    }
  }

  /**
   * 跑一条中间件链。
   *
   * 中间件可以是异步的：返回 Promise 时整条链的结果也是 Promise。中间件抛错（或
   * reject）时跳过它——若它已经调过 next，沿用下游的结果，避免下游被执行两次。
   */
  function codexPlusRunWaterfall(name, value) {
    const chain = [...(codexPlusExtensionInterceptors.get(name) || [])];
    const step = (index, current) => {
      if (index >= chain.length) return current;
      const item = chain[index];
      let called = false;
      let downstream;
      const next = (nextValue = current) => {
        called = true;
        downstream = step(index + 1, nextValue);
        return downstream;
      };
      const recover = () => (called ? downstream : step(index + 1, current));
      const outcome = runCodexPlusExtensionCallback(item.scriptKey, `intercept:${name}`, () => item.handler(current, next));
      if (!outcome.ok) return recover();
      if (codexPlusIsThenable(outcome.value)) {
        return Promise.resolve(outcome.value).catch((error) => {
          codexPlusMarkExtensionFailure(item.scriptKey, `intercept:${name}: ${String(error?.stack || error?.message || error)}`);
          return recover();
        });
      }
      return outcome.value;
    };
    return step(0, value);
  }

  /**
   * Codex 会话事件：把 app-server 推给页面的通知转成拓展事件 `codex/<method>`。
   *
   * 复用 30-service-tier 找到的通知分发器（dispatcher.subscribe），只订阅一组固定的通知；
   * 没有拓展监听时 emit 只是一次 Map 查找。分发器换了（Codex 重建客户端）就重新订阅。
   */
  function installCodexExtensionEventBridge(dispatcher) {
    if (!dispatcher || typeof dispatcher.subscribe !== "function") return false;
    // 列表放在函数里：本函数可能在内核分片的常量初始化之前被调用（测试单独执行前面的分片），
    // 顶层 const 会落进暂时性死区。
    const events = [
      "thread/started",
      "turn/started",
      "turn/completed",
      "item/started",
      "item/completed",
      "item/agentMessage/delta",
      "error",
    ];
    try {
      const current = window.__codexPlusExtensionEventBridge;
      if (current?.dispatcher === dispatcher) return true;
      try {
        current?.unsubscribe?.();
      } catch {}
      const unsubscribers = events.map((method) => {
        try {
          return dispatcher.subscribe(method, (params) => {
            // 转发失败（例如内核尚未初始化）不能影响 Codex 自己的通知处理。
            try {
              codexPlusEmitExtensionEvent(`codex/${method}`, [params, { method }]);
            } catch {}
          });
        } catch {
          return null;
        }
      });
      window.__codexPlusExtensionEventBridge = {
        dispatcher,
        unsubscribe: () => unsubscribers.forEach((unsubscribe) => {
          try {
            if (typeof unsubscribe === "function") unsubscribe();
          } catch {}
        }),
      };
      return true;
    } catch {
      return false;
    }
  }

  /**
   * 发往 Codex app-server 的请求中间件：`codex/request` 链，值为 `{ method, params }`。
   *
   * 两个接入点：新版 Codex 的请求走 app-server 客户端 sendRequest（异步，见
   * codexPlusApplyExtensionRequestMiddlewareAsync）；旧的消息分发路径走这里，是同步的，
   * 中间件返回 Promise 时视为出错并原样放行。没有中间件时原样返回同一个对象，
   * 调用方靠对象身份判断「是否改过」。method 不允许改。
   */
  function codexPlusApplyExtensionRequestMiddleware(method, params) {
    try {
      if (!codexPlusExtensionInterceptors.has("codex/request")) return params;
      const result = codexPlusRunWaterfall("codex/request", { method, params });
      if (codexPlusIsThenable(result)) {
        Promise.resolve(result).catch(() => {});
        codexPlusMarkExtensionFailure("", "codex/request 中间件必须同步返回，本次请求已原样发出");
        return params;
      }
      return result && typeof result === "object" && result.params !== undefined ? result.params : params;
    } catch {
      // 页面早期（内核尚未初始化）或中间件整体失败时，绝不能挡住 Codex 自己的请求。
      return params;
    }
  }

  /** 有拓展挂了 codex/request 中间件：需要 app-server 客户端补丁（以及它依赖的类捕获）。 */
  function codexPlusExtensionWantsRequestMiddleware() {
    try {
      return codexPlusExtensionInterceptors.has("codex/request");
    } catch {
      return false;
    }
  }

  /**
   * 异步版：app-server 客户端的 sendRequest 本身是异步的，这里允许中间件返回 Promise。
   * 出错或超时（5 秒）一律原样放行，绝不能让 Codex 自己的请求卡住。
   */
  async function codexPlusApplyExtensionRequestMiddlewareAsync(method, params) {
    if (!codexPlusExtensionWantsRequestMiddleware()) return params;
    try {
      const result = await Promise.race([
        Promise.resolve(codexPlusRunWaterfall("codex/request", { method, params })),
        new Promise((resolve) => setTimeout(() => resolve(null), 5000)),
      ]);
      return result && typeof result === "object" && result.params !== undefined ? result.params : params;
    } catch {
      return params;
    }
  }

  /**
   * 创建一个拓展实例并执行 apply(ctx)。
   *
   * definition 可以是 apply 函数本身，也可以是 { id, apply }。apply 同步抛错时
   * 先回滚它已经做过的注册，再把错误抛给调用方——脚本初始化期间调用的话，
   * wrap_script 会把该脚本标成 failed。异步 apply reject 时同样回滚并上报。
   *
   * 必须在脚本同步初始化期间调用才能接上热重载：此时内核向用户脚本运行时登记
   * 一个清理函数，重载前整体卸载。之后（await 之后）创建的实例只能靠 ctx.dispose()。
   */
  function codexPlusCreateExtension(definition, options = {}) {
    const apply = typeof definition === "function" ? definition : definition?.apply;
    if (typeof apply !== "function") throw new Error("拓展必须提供 apply(ctx) 函数");
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const manifest = codexPlusExtensionManifest(scriptKey);
    const extensionId = String(
      (typeof definition === "object" && definition?.id) || options.id || manifest?.id || scriptKey || "anonymous",
    );
    const disposers = [];
    let disposed = false;

    /** 登记一个撤销函数，返回「提前撤销」。实例已卸载时立刻执行。 */
    const track = (dispose) => {
      if (typeof dispose !== "function") return () => {};
      let active = true;
      const entry = () => {
        if (!active) return;
        active = false;
        dispose();
      };
      if (disposed) {
        runCodexPlusExtensionCallback(scriptKey, `${extensionId}.dispose`, entry);
        return () => {};
      }
      disposers.push(entry);
      return () => {
        const index = disposers.indexOf(entry);
        if (index >= 0) disposers.splice(index, 1);
        entry();
      };
    };

    const registerOptions = (itemOptions) => ({ ...(itemOptions || {}), scriptKey });
    // 按拓展隔离的本地存储：键名自动带拓展 id 前缀，值按 JSON 存取。
    const storagePrefix = `codexpp:ext:${extensionId}:`;
    const storage = {
      get(key, fallback = null) {
        try {
          const raw = localStorage.getItem(storagePrefix + key);
          return raw === null ? fallback : JSON.parse(raw);
        } catch {
          return fallback;
        }
      },
      set(key, value) {
        localStorage.setItem(storagePrefix + key, JSON.stringify(value));
      },
      remove(key) {
        localStorage.removeItem(storagePrefix + key);
      },
      keys() {
        const out = [];
        for (let index = 0; index < localStorage.length; index += 1) {
          const key = localStorage.key(index);
          if (key?.startsWith(storagePrefix)) out.push(key.slice(storagePrefix.length));
        }
        return out;
      },
    };
    const info = { id: extensionId, scriptKey };

    // 生效配置（清单 config 默认值叠加用户在管理器 / 对话里设置的值）。改配置会热重载拓展，这里是只读快照。
    const config = Object.freeze({ ...(manifest?.config && typeof manifest.config === "object" ? manifest.config : {}) });
    const ctx = {
      id: extensionId,
      scriptKey,
      manifest,
      config,
      get disposed() {
        return disposed;
      },
      storage,

      /** 立即执行 fn；它返回的函数在卸载时调用。 */
      effect(fn) {
        if (typeof fn !== "function") throw new Error("effect 需要一个函数");
        return track(fn());
      },

      registerRowAction(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("rowAction", codexPlusRegistry.rowActions, definition, registerOptions(itemOptions)));
      },
      registerNavEntry(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("navEntry", codexPlusRegistry.navEntries, definition, registerOptions(itemOptions)));
      },
      registerMenuItem(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("menuItem", codexPlusRegistry.menuItems, definition, registerOptions(itemOptions)));
      },
      registerPage(definition, itemOptions) {
        return track(codexPlusRegisterExtensionPage(definition, registerOptions(itemOptions)));
      },
      registerCommand(definition, itemOptions) {
        return track(codexPlusRegisterExtensionCommand(definition, registerOptions(itemOptions)));
      },
      registerComposerAction(definition, itemOptions) {
        return track(codexPlusRegisterExtensionItem("composerAction", codexPlusRegistry.composerActions, definition, registerOptions(itemOptions)));
      },

      on(event, handler) {
        if (typeof event !== "string" || !event) throw new Error("事件名不能为空");
        if (typeof handler !== "function") throw new Error("事件处理器必须是函数");
        const listener = { handler, scriptKey };
        if (!codexPlusExtensionListeners.has(event)) codexPlusExtensionListeners.set(event, new Set());
        codexPlusExtensionListeners.get(event).add(listener);
        return track(() => {
          const listeners = codexPlusExtensionListeners.get(event);
          listeners?.delete(listener);
          if (listeners && !listeners.size) codexPlusExtensionListeners.delete(event);
        });
      },
      emit(event, ...args) {
        codexPlusEmitExtensionEvent(event, args);
      },

      intercept(name, handler, interceptOptions = {}) {
        if (typeof name !== "string" || !name) throw new Error("中间件链名不能为空");
        if (typeof handler !== "function") throw new Error("中间件必须是函数");
        const item = {
          handler,
          scriptKey,
          order: Number.isFinite(interceptOptions.order) ? Number(interceptOptions.order) : 0,
          seq: ++codexPlusExtensionInterceptorSeq,
        };
        const chain = [...(codexPlusExtensionInterceptors.get(name) || []), item];
        chain.sort((left, right) => left.order - right.order || left.seq - right.seq);
        codexPlusExtensionInterceptors.set(name, chain);
        return track(() => {
          const rest = (codexPlusExtensionInterceptors.get(name) || []).filter((entry) => entry !== item);
          if (rest.length) codexPlusExtensionInterceptors.set(name, rest);
          else codexPlusExtensionInterceptors.delete(name);
        });
      },
      waterfall(name, value) {
        return codexPlusRunWaterfall(name, value);
      },

      listen(target, type, handler, listenOptions) {
        if (!target || typeof target.addEventListener !== "function") throw new Error("listen 的目标必须支持 addEventListener");
        target.addEventListener(type, handler, listenOptions);
        return track(() => target.removeEventListener(type, handler, listenOptions));
      },
      setTimeout(handler, delay, ...args) {
        let timer = null;
        const revoke = track(() => window.clearTimeout(timer));
        timer = window.setTimeout(() => {
          revoke();
          runCodexPlusExtensionCallback(scriptKey, `${extensionId}.setTimeout`, () => handler(...args));
        }, delay);
        return revoke;
      },
      setInterval(handler, delay, ...args) {
        const timer = window.setInterval(() => {
          runCodexPlusExtensionCallback(scriptKey, `${extensionId}.setInterval`, () => handler(...args));
        }, delay);
        return track(() => window.clearInterval(timer));
      },

      /**
       * 调后端路由。拓展包只能调清单 permissions.routes 声明过的路由；单文件脚本
       * 没有清单，沿用全局白名单。这是声明约束而非安全边界：拓展与页面同权。
       */
      call(route, payload, callOptions) {
        if (manifest) {
          const routes = Array.isArray(manifest.permissions?.routes) ? manifest.permissions.routes : [];
          if (!routes.includes(route)) {
            return Promise.reject(new Error(`拓展 ${extensionId} 未在 codexpp.json 声明路由权限：${route}`));
          }
        }
        return codexPlusExtensionCall(route, payload, callOptions);
      },
      toast(message, toastOptions) {
        return runCodexPlusExtensionCallback(scriptKey, `${extensionId}.toast`, () => showToast(String(message ?? ""), toastOptions || {}));
      },
      fail(error) {
        codexPlusMarkExtensionFailure(scriptKey, `${extensionId}: ${String(error?.stack || error?.message || error)}`);
      },

      /** 卸载：按注册逆序撤销，单项撤销抛错只记录、不中断其余撤销。 */
      dispose() {
        if (disposed) return;
        disposed = true;
        for (const entry of disposers.splice(0).reverse()) {
          runCodexPlusExtensionCallback(scriptKey, `${extensionId}.dispose`, entry);
        }
        codexPlusExtensionInstances.delete(ctx);
        codexPlusEmitExtensionEvent("extension/dispose", [info]);
      },
    };

    codexPlusExtensionInstances.add(ctx);
    const runtime = window.__codexPlusUserScripts;
    if (scriptKey && runtime?.currentKey === scriptKey && typeof runtime.registerCleanup === "function") {
      runtime.registerCleanup(() => ctx.dispose());
    }

    let result;
    try {
      result = apply(ctx);
    } catch (error) {
      ctx.dispose();
      throw error;
    }
    if (codexPlusIsThenable(result)) {
      Promise.resolve(result).catch((error) => {
        ctx.fail(error);
        ctx.dispose();
      });
    }
    if (!disposed) codexPlusEmitExtensionEvent("extension/apply", [info]);
    return ctx;
  }
