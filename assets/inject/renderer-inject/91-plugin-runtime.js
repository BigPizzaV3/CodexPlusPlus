  /**
   * 插件运行时内核：codexPlus.plugin(apply)
   *
   * 在 register* 之上加一层「插件实例」：ctx 上的每一次注册（UI 项、事件监听、
   * 中间件、DOM 监听、定时器、自定义副作用）都绑定到实例的寿命上，卸载时按注册
   * 逆序自动撤销。拓展作者不必再自己维护 dispose 列表，热重载也不会残留旧实例。
   *
   * 两种插件间耦合方式：
   *   - 事件：ctx.on / ctx.emit，一对多通知，处理器抛错互不影响
   *   - waterfall：ctx.intercept / ctx.waterfall，中间件链，handler(value, next)
   *     调 next(v) 交给下一个，不调即短路；没有中间件时原样返回
   *
   * 宿主后续会在协议层、界面层的关键点调用 codexPlusRunWaterfall /
   * codexPlusEmitPluginEvent，插件用同一套 ctx 接口挂上去。设计见
   * docs/plans/2026-10-30-codexpp-plugin-system-design.md。
   *
   * 和 91-extension-api.js 一样只做定义、不读 DOM，挂载在 99-startup。
   */

  /** 事件名 → 监听者集合。监听者带 scriptKey，出错时记到对应脚本。 */
  const codexPlusPluginListeners = new Map();
  /** 中间件链名 → 中间件数组，按 order、再按注册先后排序。 */
  const codexPlusPluginInterceptors = new Map();
  let codexPlusPluginInterceptorSeq = 0;
  /** 存活的插件实例，供诊断查看。 */
  const codexPlusPluginInstances = new Set();

  /** 包插件的清单（Rust 侧写进脚本状态记录）；单文件脚本没有清单。 */
  function codexPlusPluginManifest(scriptKey) {
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
  function codexPlusEmitPluginEvent(event, args = []) {
    const listeners = codexPlusPluginListeners.get(event);
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
    const chain = [...(codexPlusPluginInterceptors.get(name) || [])];
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
   * 创建一个插件实例并执行 apply(ctx)。
   *
   * definition 可以是 apply 函数本身，也可以是 { id, apply }。apply 同步抛错时
   * 先回滚它已经做过的注册，再把错误抛给调用方——脚本初始化期间调用的话，
   * wrap_script 会把该脚本标成 failed。异步 apply reject 时同样回滚并上报。
   *
   * 必须在脚本同步初始化期间调用才能接上热重载：此时内核向用户脚本运行时登记
   * 一个清理函数，重载前整体卸载。之后（await 之后）创建的实例只能靠 ctx.dispose()。
   */
  function codexPlusCreatePlugin(definition, options = {}) {
    const apply = typeof definition === "function" ? definition : definition?.apply;
    if (typeof apply !== "function") throw new Error("插件必须提供 apply(ctx) 函数");
    const scriptKey = codexPlusCurrentExtensionScriptKey(options);
    const manifest = codexPlusPluginManifest(scriptKey);
    const pluginId = String(
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
        runCodexPlusExtensionCallback(scriptKey, `${pluginId}.dispose`, entry);
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
    const info = { id: pluginId, scriptKey };

    const ctx = {
      id: pluginId,
      scriptKey,
      manifest,
      get disposed() {
        return disposed;
      },

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

      on(event, handler) {
        if (typeof event !== "string" || !event) throw new Error("事件名不能为空");
        if (typeof handler !== "function") throw new Error("事件处理器必须是函数");
        const listener = { handler, scriptKey };
        if (!codexPlusPluginListeners.has(event)) codexPlusPluginListeners.set(event, new Set());
        codexPlusPluginListeners.get(event).add(listener);
        return track(() => {
          const listeners = codexPlusPluginListeners.get(event);
          listeners?.delete(listener);
          if (listeners && !listeners.size) codexPlusPluginListeners.delete(event);
        });
      },
      emit(event, ...args) {
        codexPlusEmitPluginEvent(event, args);
      },

      intercept(name, handler, interceptOptions = {}) {
        if (typeof name !== "string" || !name) throw new Error("中间件链名不能为空");
        if (typeof handler !== "function") throw new Error("中间件必须是函数");
        const item = {
          handler,
          scriptKey,
          order: Number.isFinite(interceptOptions.order) ? Number(interceptOptions.order) : 0,
          seq: ++codexPlusPluginInterceptorSeq,
        };
        const chain = [...(codexPlusPluginInterceptors.get(name) || []), item];
        chain.sort((left, right) => left.order - right.order || left.seq - right.seq);
        codexPlusPluginInterceptors.set(name, chain);
        return track(() => {
          const rest = (codexPlusPluginInterceptors.get(name) || []).filter((entry) => entry !== item);
          if (rest.length) codexPlusPluginInterceptors.set(name, rest);
          else codexPlusPluginInterceptors.delete(name);
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
          runCodexPlusExtensionCallback(scriptKey, `${pluginId}.setTimeout`, () => handler(...args));
        }, delay);
        return revoke;
      },
      setInterval(handler, delay, ...args) {
        const timer = window.setInterval(() => {
          runCodexPlusExtensionCallback(scriptKey, `${pluginId}.setInterval`, () => handler(...args));
        }, delay);
        return track(() => window.clearInterval(timer));
      },

      /**
       * 调后端路由。包插件只能调清单 permissions.routes 声明过的路由；单文件脚本
       * 没有清单，沿用全局白名单。这是声明约束而非安全边界：拓展与页面同权。
       */
      call(route, payload, callOptions) {
        if (manifest) {
          const routes = Array.isArray(manifest.permissions?.routes) ? manifest.permissions.routes : [];
          if (!routes.includes(route)) {
            return Promise.reject(new Error(`插件 ${pluginId} 未在 codexpp.json 声明路由权限：${route}`));
          }
        }
        return codexPlusExtensionCall(route, payload, callOptions);
      },
      toast(message, toastOptions) {
        return runCodexPlusExtensionCallback(scriptKey, `${pluginId}.toast`, () => showToast(String(message ?? ""), toastOptions || {}));
      },
      fail(error) {
        codexPlusMarkExtensionFailure(scriptKey, `${pluginId}: ${String(error?.stack || error?.message || error)}`);
      },

      /** 卸载：按注册逆序撤销，单项撤销抛错只记录、不中断其余撤销。 */
      dispose() {
        if (disposed) return;
        disposed = true;
        for (const entry of disposers.splice(0).reverse()) {
          runCodexPlusExtensionCallback(scriptKey, `${pluginId}.dispose`, entry);
        }
        codexPlusPluginInstances.delete(ctx);
        codexPlusEmitPluginEvent("plugin/dispose", [info]);
      },
    };

    codexPlusPluginInstances.add(ctx);
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
    if (!disposed) codexPlusEmitPluginEvent("plugin/apply", [info]);
    return ctx;
  }
