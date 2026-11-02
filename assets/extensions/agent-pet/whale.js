  // 大肥鱼（完整版鲸鱼挂件）由上游原始引擎（whale-engine.js）+ Codex 数据适配层（whale-compat.js）组成。
  // 它是 Agent 桌宠的一种类型：本拓展的 config.kind 为 whaleWidget 时挂载，与 pet.js 的图片桌宠互斥。
  // 数据都经公开路由：`/whale/full`（挂件设置、角色 / 音频图库、余额与用量）、`/whale/history`、`/whale/session`
  // （本机 Codex 用量统计）、`/whale/media`（内嵌素材）。供应商的 Key 只在 Codex++ 里用，不会交给页面。

  /** 拓展配置里的余额设置，原样转给 Codex++ 的余额查询。 */
  function codexPlusWhaleBalanceFrom(config) {
    const raw = config && typeof config === "object" ? config : {};
    return {
      protocol: raw.codexAppWhaleBalanceProtocol === "custom" || raw.codexAppWhaleBalanceProtocol === "off" ? raw.codexAppWhaleBalanceProtocol : "auto",
      path: String(raw.codexAppWhaleBalancePath ?? ""),
      field: String(raw.codexAppWhaleBalanceField ?? ""),
      currency: String(raw.codexAppWhaleBalanceCurrency ?? "USD"),
      scale: Number(raw.codexAppWhaleBalanceScale ?? 1),
    };
  }

  /**
   * 大肥鱼挂件。`ctx` 只用公开接口；返回 { sync, dispose, state }。
   * 余额设置变了（换协议 / 接口）就重建挂件，让它按新设置重新取数。
   */
  function createCodexPlusWhaleWidget(ctx) {
    const whale = { disposed: false, runtime: null, fingerprint: "", failedUntil: 0, notice: null, mediaLoaded: false };
    const config = () => ctx.config || {};
    const selected = () => !whale.disposed && config().kind === "whaleWidget";
    // 挂件的数据请求：路由在清单 permissions.routes 里声明过；/whale/full 带上本拓展配置里的余额设置。
    const platform = {
      call(path, payload) {
        const body = path === "/whale/full" ? { ...payload, balance: codexPlusWhaleBalanceFrom(config()) } : payload;
        return ctx.call(path, body, { timeout: 45000 });
      },
      session() {
        const session = codexPlus.currentSession?.() || {};
        return { session_id: session.id || "", title: session.title || "", host_id: session.hostId || "local" };
      },
      on(method, handler) {
        return ctx.on(`codex/${method}`, (params) => handler(params));
      },
    };

    function stop() {
      whale.runtime?.dispose();
      whale.runtime = null;
      whale.notice?.remove();
      whale.notice = null;
    }

    function initializationFailed(runtime) {
      runtime?.dispose();
      whale.runtime = null;
      whale.failedUntil = Date.now() + 10000;
      if (whale.notice) return;
      const notice = document.createElement("div");
      notice.setAttribute("data-codex-plus-ext", "whale-widget");
      notice.setAttribute("role", "status");
      notice.style.cssText = "position:fixed;right:20px;bottom:20px;z-index:2147483600;max-width:320px;padding:12px;border-radius:10px;background:#fff5ec;color:#7c3218;font:12px/1.6 system-ui;box-shadow:0 3px 16px #0002";
      notice.textContent = "鲸鱼挂件暂时无法加载，10 秒后自动重试。可在 Codex++「Agent 桌宠」页关闭后重新开启。";
      document.body.appendChild(notice);
      whale.notice = notice;
    }

    async function loadMedia() {
      if (whale.mediaLoaded) return;
      const media = await ctx.call("/whale/media", {}, { timeout: 45000 });
      if (media?.status !== "ok" || !media.assets || typeof media.image !== "string") throw new Error("大肥鱼素材读取失败");
      codexPlusWhaleMedia.image = media.image;
      codexPlusWhaleMedia.assets = media.assets;
      whale.mediaLoaded = true;
    }

    function sync() {
      if (!selected()) {
        stop();
        whale.failedUntil = 0;
        return;
      }
      const fingerprint = JSON.stringify(codexPlusWhaleBalanceFrom(config()));
      if (whale.fingerprint !== fingerprint) {
        stop();
        whale.failedUntil = 0;
      }
      whale.fingerprint = fingerprint;
      if (whale.runtime) {
        whale.runtime.context();
        whale.runtime.subscribe();
        whale.runtime.pause(document.hidden === true);
        return;
      }
      // 隐藏窗口等恢复可见后再挂载，不在后台构造音频或轮询会话日志。
      if (document.hidden || !document.body) return;
      if (Date.now() < whale.failedUntil) return;
      const runtime = createCodexPlusWhaleFullRuntime(platform);
      whale.runtime = runtime;
      const start = () => runtime.initialize().then(() => {
        if (runtime.active() && whale.runtime === runtime && selected()) {
          codexPlusWhaleEngine(...runtime.environment);
          if (runtime.state.initializationError || !runtime.state.controls) throw new Error("完整鲸鱼初始化未完成");
          whale.notice?.remove();
          whale.notice = null;
        }
      });
      // 素材已经取到时立刻初始化（与原来一样同步发出第一份设置请求，初始化期间的短回合也能记上）。
      const ready = whale.mediaLoaded || Object.keys(codexPlusWhaleMedia.assets).length > 0;
      void (ready ? start() : loadMedia().then(start)).catch(() => {
        if (!runtime.active()) return;
        if (whale.runtime === runtime) initializationFailed(runtime);
      });
    }

    return {
      sync,
      dispose() {
        stop();
        whale.disposed = true;
      },
      get state() {
        return whale.runtime?.state || null;
      },
      // 只读，给诊断与测试看：当前的挂件运行时、加载失败提示、数据平台。
      get runtime() {
        return whale.runtime;
      },
      get notice() {
        return whale.notice;
      },
      platform,
    };
  }
