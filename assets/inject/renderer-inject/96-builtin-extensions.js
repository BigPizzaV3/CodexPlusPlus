  /**
   * 内置拓展：Codex++ 自带的功能也挂在拓展内核上，和第三方拓展走同一套生命周期
   * （没有特权功能：自带的也能单独停用、被别的拓展顶替）。
   *
   *   - 每个内置功能是一个拓展实例（scriptKey = builtin:<id>）：实例在时功能可用，卸载时功能撤下；
   *   - 管理器「内置拓展」里可以单独停用（用户脚本配置里 scripts["builtin:<id>"] = false）；
   *   - 第三方拓展包在 codexpp.json 里声明 `replaces: ["<id>"]`，它启用时对应的内置拓展让位；
   *   - 功能自己的设置开关（如「Agent 桌宠」）照旧生效：内置拓展是外面一层，两者都开功能才出现。
   *
   * 停用 / 顶替状态由 Codex++ 随用户脚本注入包下发（window.__codexPlusBuiltinExtensionState），
   * 热重载时更新。功能的开关检查在注入早期（本分片执行前）、以及只抽单个分片的测试里也会被调用，所以只记
   * 「停用的」：globalThis.__codexPlusBuiltinInactive 里没有的 id 都算启用，功能侧写成
   * `!globalThis.__codexPlusBuiltinInactive?.has("<id>")`。实例卸载时加进集合，装上时移出。
   */
  function codexPlusBuiltinInactiveIds() {
    if (!(globalThis.__codexPlusBuiltinInactive instanceof Set)) globalThis.__codexPlusBuiltinInactive = new Set();
    return globalThis.__codexPlusBuiltinInactive;
  }

  /** 内置拓展是否在用（没被停用、没被顶替）。 */
  function codexPlusBuiltinExtensionActive(id) {
    return !globalThis.__codexPlusBuiltinInactive?.has(id);
  }

  /**
   * 内置拓展清单：每一项只有 apply(ctx)，和第三方拓展一样只用公开的 ctx 接口（onScan、on、effect、registerSlot……），
   * 装上即生效、卸下时登记的东西自动撤销。管理器里的名字与说明在 Rust 侧 BUILTIN_EXTENSIONS 维护，两边 id 要一致。
   * 没有 apply 的几项由 Rust 侧（启动器、注入、本地代理）或独立运行时按停用状态生效，页面侧只登记。
   */
  function codexPlusBuiltinExtensionCatalog() {
    return [
      // Agent 桌宠（含大肥鱼）、用量状态栏、请求状态条已改成随 Codex++ 发布的官方拓展包（bundled:agent-pet、
      // bundled:usage-statusline、bundled:proxy-request-status），不再是内置拓展。
      { id: "dictation", name: "语音听写", apply: codexPlusDictationExtension },
      { id: "turn-rewind", name: "轮次回退", apply: codexPlusTurnRewindExtension },
      { id: "side-chat-save", name: "侧聊保存", apply: codexPlusSideChatSaveExtension },
      // 由 Rust 经 CDP 驱动，页面侧只登记；被顶替时 Rust 侧不再补载，顶替它的拓展订阅 codexpp/session-index-repaired 自己做。
      { id: "history-turn-recovery", name: "历史轮次补全" },
      { id: "capacity-auto-retry", name: "容量不足自动重试", apply: codexPlusCapacityAutoRetryExtension },
      { id: "model-picker-groups", name: "模型分组", apply: codexPlusModelPickerGroupsExtension },
      { id: "dream-skin", name: "梦皮主题", apply: codexPlusDreamSkinExtension },
      { id: "plugin-market-unlock", name: "插件市场解锁", apply: codexPlusPluginMarketUnlockExtension },
      { id: "instance-identity", name: "多开身份提示", apply: codexPlusInstanceIdentityExtension },
      { id: "session-title-hook", name: "会话标题中间件", apply: codexPlusSessionTitleExtension },
      { id: "session-delete", name: "会话删除", apply: codexPlusSessionDeleteExtension },
      { id: "markdown-export", name: "Markdown 导出", apply: codexPlusMarkdownExportExtension },
      { id: "thread-id-badge", name: "会话 ID 角标", apply: codexPlusThreadIdBadgeExtension },
      { id: "thread-scroll-restore", name: "滚动位置恢复", apply: codexPlusThreadScrollRestoreExtension },
      { id: "conversation-view", name: "对话宽度", apply: codexPlusConversationViewExtension },
      { id: "activity-overlay-right", name: "消息导航条靠右", apply: codexPlusActivityOverlayExtension },
      { id: "paste-fix", name: "粘贴修复", apply: codexPlusPasteFixExtension },
      { id: "weixin-desktop-sync", name: "微信会话同步", apply: codexPlusWeixinDesktopSyncExtension },
      // 悬浮球是独立运行时，自己按停用集合判断；这两项让它在装卸、改设置时重新算一次。
      { id: "stepwise", name: "下一步建议", apply: codexPlusStepwiseExtension },
      { id: "answer-outline", name: "回答大纲", apply: codexPlusStepwiseExtension },
      // 启动器每 5 秒按内置拓展状态决定是否驱动，页面侧无事可做。等价写法：/system/cursor-position + /cdp/evaluate。
      { id: "pet-real-mouse", name: "桌宠跟随鼠标" },
      // 以下由 Rust 侧（启动、注入、本地代理）按设置项生效，多数要重启 Codex，页面侧只登记。被顶替时 Rust 侧按
      // 「关」处理（builtin_takeover），顶替它的拓展用 preload（页面最早期脚本）、launch（启动前后）、网关中间件写等价功能。
      { id: "model-whitelist-unlock", name: "模型白名单解锁" },
      { id: "native-fast-unlock", name: "原生 Fast 强制解锁" },
      { id: "native-predictions", name: "原生输入预测" },
      { id: "reasoning-collapse", name: "第三方模型回答完自动折叠过程" },
      { id: "disable-work-view", name: "纯 API 模式禁用 Work 视图" },
      { id: "force-chinese-locale", name: "强制中文界面" },
      { id: "native-browser-identification", name: "原生 Edge / Chrome 请求标识兼容" },
      { id: "standalone-quota-unlock", name: "官方额度解锁" },
    ];
  }

  /** 下发的停用 / 顶替状态：{ disabled: [id], replacedBy: { id: 拓展 key } }。 */
  function codexPlusBuiltinExtensionState() {
    const state = window.__codexPlusBuiltinExtensionState;
    return {
      disabled: new Set(Array.isArray(state?.disabled) ? state.disabled.map(String) : []),
      replacedBy: state?.replacedBy && typeof state.replacedBy === "object" ? state.replacedBy : {},
    };
  }

  // 同页重复注入：先卸下上一份注入包装上的内置拓展（它们的监听、中间件、扫描步骤挂在旧内核上，新内核看不到）。
  window.__codexPlusBuiltinExtensionHandles?.forEach((handle) => {
    try {
      handle.dispose();
    } catch {}
  });
  const codexPlusBuiltinExtensionHandles = window.__codexPlusBuiltinExtensionHandles = new Map();

  // 用量状态栏、请求状态条的数据服务 codexpp.usage / codexpp.proxyStatus 改由对应的官方拓展包提供
  // （assets/extensions/usage-statusline、proxy-request-status）。

  /** 按下发状态装上 / 卸下内置拓展。重复调用只处理有变化的项。 */
  function syncCodexPlusBuiltinExtensions() {
    const inactive = codexPlusBuiltinInactiveIds();
    const state = codexPlusBuiltinExtensionState();
    for (const definition of codexPlusBuiltinExtensionCatalog()) {
      const wanted = !state.disabled.has(definition.id) && !state.replacedBy[definition.id];
      const handle = codexPlusBuiltinExtensionHandles.get(definition.id);
      if (wanted && !handle) {
        try {
          const ctx = codexPlusCreateExtension({
            id: `builtin:${definition.id}`,
            apply(ctx) {
              // 停用集合是功能侧「这个内置拓展装着没有」的读数：装上前移出；卸下时最先记回（最后登记、最先撤销），
              // 功能自己的撤销步骤（再对齐一次）读到的就是停用。
              inactive.delete(definition.id);
              try {
                if (typeof definition.apply === "function") definition.apply(ctx);
              } finally {
                ctx.effect(() => () => inactive.add(definition.id));
              }
            },
          }, { scriptKey: `builtin:${definition.id}` });
          codexPlusBuiltinExtensionHandles.set(definition.id, ctx);
        } catch (error) {
          inactive.add(definition.id);
          codexPlusMarkExtensionFailure(`builtin:${definition.id}`, String(error?.stack || error?.message || error));
        }
      } else if (!wanted && handle) {
        codexPlusBuiltinExtensionHandles.delete(definition.id);
        handle.dispose();
      } else if (!wanted && !inactive.has(definition.id)) {
        // 还没装上过（注入早期按默认启用跑着）：直接记成停用，补一轮扫描让按停用集合判断的功能撤下。
        inactive.add(definition.id);
        scheduleDeferredScan();
      }
    }
  }

  /** 内置拓展的现状：给 codexPlus.builtins() 与诊断用。 */
  function codexPlusBuiltinExtensionList() {
    const state = codexPlusBuiltinExtensionState();
    return codexPlusBuiltinExtensionCatalog().map(({ id, name }) => ({
      id,
      name,
      active: codexPlusBuiltinExtensionActive(id),
      disabled: state.disabled.has(id),
      replacedBy: state.replacedBy[id] || null,
    }));
  }

  /**
   * 「按开关对齐」的内置功能共用的写法，只用公开 ctx 接口（第三方拓展照抄即可）：
   * 装上时对齐一次，卸下时再对齐一次（这时已记成停用，功能自己撤下）；scan：跟着页面扫描对齐（"deferred" | "light"）。
   * settings：true 在启动读到设置、页面里改了设置（settings/changed）时对齐；"heartbeat" 另外每次心跳读到设置
   * （含管理器里改的）都对齐，只给对齐很便宜、又要跟上管理器改动的功能用。
   */
  function codexPlusAlignWith(ctx, align, { settings = false, scan = null } = {}) {
    const run = () => runScanStep(align);
    if (scan) ctx.onScan(align, { phase: scan });
    if (settings) {
      ctx.on("settings/loaded", (load) => {
        if (settings === "heartbeat" || load?.reason !== "heartbeat") run();
      });
      ctx.on("settings/changed", run);
    }
    ctx.effect(() => {
      run();
      return run;
    });
  }

  /** 语音听写：输入框 footer 会重建，入口跟着页面扫描补；卸下时撤掉入口并取消进行中的录音。 */
  function codexPlusDictationExtension(ctx) {
    ctx.listen(document, "keydown", codexPlusDictationKeyHandler, true);
    codexPlusAlignWith(ctx, installCodexPlusDictation, { scan: "deferred" });
  }

  /** 轮次回退：菜单项跟着轻量扫描补，开关改了立即启停。 */
  function codexPlusTurnRewindExtension(ctx) {
    codexPlusAlignWith(ctx, syncCodexPlusTurnRewind, { scan: "light", settings: true });
  }

  /** 容量不足自动重试：轮询定时器装上时按设置启动，改设置即启停，卸下时清掉定时器与浮标。 */
  function codexPlusCapacityAutoRetryExtension(ctx) {
    codexPlusAlignWith(ctx, syncCodexPlusCapacityAutoRetry, { settings: "heartbeat" });
  }

  /** 模型分组：模型菜单的轮询随拓展装上开始；卸下时扫描一次清理分组、停下轮询。 */
  function codexPlusModelPickerGroupsExtension(ctx) {
    codexPlusAlignWith(ctx, () => window.__codexPlusModelPickerGroupsRuntime?.sync?.());
  }

  /** 梦皮主题：主界面重建时补皮肤，设置变了就重装或清理。 */
  function codexPlusDreamSkinExtension(ctx) {
    codexPlusAlignWith(ctx, refreshDreamSkin, { settings: "heartbeat", scan: "deferred" });
  }

  /** 插件市场解锁：补丁装上后撤不掉，停用后刷新页面才完全恢复。 */
  function codexPlusPluginMarketUnlockExtension(ctx) {
    ctx.onScan(installCodexPluginMarketplacePatches);
  }

  /**
   * 按设置开关注册 / 撤销一项会话行操作（只用公开 ctx 接口）：读到、改了设置时重新判断，开着就注册、关了就撤销。
   */
  function codexPlusRowActionWhen(ctx, enabled, definition) {
    let dispose = null;
    const sync = () => {
      const wanted = !!enabled();
      if (wanted && !dispose) dispose = ctx.registerRowAction(definition, { id: definition.id });
      else if (!wanted && dispose) {
        dispose();
        dispose = null;
      }
    };
    ctx.on("settings/loaded", () => runScanStep(sync));
    ctx.on("settings/changed", () => runScanStep(sync));
    sync();
  }

  /**
   * 会话删除：会话行上常驻的删除按钮（公开的行内会话行操作），确认、删除、撤销都在 80-session-share.js；
   * 侧栏重建（如折叠再展开项目分组）时把已删会话重新藏起来（#2671）；点击走文档级委托（按钮会被克隆替换）。
   */
  function codexPlusSessionDeleteExtension(ctx) {
    ctx.onScan(filterDeletedSessionRows);
    ctx.effect(() => {
      installDeleteButtonEventDelegation();
      return () => {
        document.removeEventListener("click", window.__codexSessionDeleteDocumentDeleteHandler, true);
        window.__codexSessionDeleteDocumentDeleteHandler = null;
      };
    });
    codexPlusRowActionWhen(ctx, () => codexPlusSettings().sessionDelete, {
      id: "session-delete",
      placement: "inline",
      label: "删除",
      icon: trashIconSvg(),
      className: buttonClass,
      order: 10,
      onMount: ({ button }) => {
        button.dataset.codexDeleteVersion = codexDeleteVersion;
      },
      onActivate: ({ row, button, event }) => openDeleteConfirmForRow(row, button, sessionRefFromRow(row), event),
    });
  }

  /** Markdown 导出：会话行「…」菜单与归档页行上的「导出」（公开的会话行操作，归档页的行按标题解析会话 id）。 */
  function codexPlusMarkdownExportExtension(ctx) {
    codexPlusRowActionWhen(ctx, () => codexPlusSettings().markdownExport, {
      id: "markdown-export",
      label: "导出",
      icon: "⇩",
      className: exportButtonClass,
      surfaces: ["sidebar", "archived"],
      order: 10,
      onActivate: async ({ archived, resolve, row }) => {
        const ref = archived ? await resolve() : sessionRefFromRow(row);
        if (!ref?.session_id) {
          showToast("导出失败：未找到归档会话 ID", null);
          return;
        }
        await exportMarkdown(ref);
      },
    });
  }

  /** 会话 ID 角标。 */
  function codexPlusThreadIdBadgeExtension(ctx) {
    codexPlusAlignWith(ctx, refreshThreadIdBadges, { scan: "deferred" });
  }

  /**
   * 滚动位置恢复：滚动、导航、路由钩子跟着轻量扫描补（钩子按设置判断，关着时不动），
   * 完整扫描时同步一次位置；关掉（改设置或卸下）时撤掉排着的保存 / 恢复定时器。
   */
  function codexPlusThreadScrollRestoreExtension(ctx) {
    ctx.onScan(() => {
      updateThreadScrollHandlers();
      installThreadScrollProgrammaticScrollGuard();
      installThreadScrollNavigationCapture();
      installThreadScrollUserIntentCapture();
      installThreadScrollRouteHooks();
      scheduleThreadScrollSync(true);
    }, { phase: "light" });
    ctx.onScan(() => scheduleThreadScrollSync());
    const stopWhenOff = () => {
      if (!codexPlusSettings().threadScrollRestore) stopThreadScrollRestore();
    };
    ctx.on("settings/changed", () => runScanStep(stopWhenOff));
    codexPlusAlignWith(ctx, () => {
      stopWhenOff();
      scheduleDeferredScan();
    });
  }

  /**
   * 对话宽度：完整扫描、窗口缩放时对齐对话区宽度；Codex++ 设置变了（管理器里改、心跳同步过来）且开关有变化时立即重算。
   */
  function codexPlusConversationViewExtension(ctx) {
    ctx.onScan(refreshConversationView);
    let resizeFrame = 0;
    ctx.listen(window, "resize", () => {
      cancelAnimationFrame(resizeFrame);
      resizeFrame = requestAnimationFrame(() => runScanStep(refreshConversationView));
    });
    ctx.effect(() => () => cancelAnimationFrame(resizeFrame));
    let previous = !!codexPlusSettings().conversationView;
    ctx.on("settings/loaded", () => {
      const current = !!codexPlusSettings().conversationView;
      if (current === previous) return;
      previous = current;
      runScanStep(refreshConversationView);
    });
    codexPlusAlignWith(ctx, () => {
      refreshConversationViewControls();
      refreshConversationView();
    });
  }

  /** 消息导航条靠右：按设置切换页面根节点上的属性。 */
  function codexPlusActivityOverlayExtension(ctx) {
    codexPlusAlignWith(ctx, () => globalThis.__CODEX_PLUS_ACTIVITY_OVERLAY_SYNC__?.(), { settings: "heartbeat" });
  }

  /** 微信会话同步：微信写入的会话通知桌面目录，读到设置（含心跳）时按设置里的待同步清单跑一次。 */
  function codexPlusWeixinDesktopSyncExtension(ctx) {
    ctx.on("settings/loaded", () => void syncWeixinDesktopThreads());
  }

  /**
   * 下一步建议 / 回答大纲：悬浮球是独立运行时，这里在装卸、读到 / 改了设置时让它按当前设置重新算一次；
   * 开关写进后端后让它重读自己的设置。两个内置拓展共用这一份，各自只管自己的开关。
   */
  function codexPlusStepwiseExtension(ctx) {
    const backendKey = ctx.id === "builtin:answer-outline" ? "codexAppAnswerOutlineEnabled" : "codexAppStepwiseEnabled";
    codexPlusAlignWith(ctx, () => syncStepwisePanel(), { settings: true });
    ctx.on("settings/changed", (change) => {
      if (change?.key !== backendKey || change.persisted !== true) return;
      Promise.resolve(window.__codexStepwisePanel?.loadSettings?.()).then(() => syncStepwisePanel());
    });
  }

  /**
   * 多开身份提示：完全用公开拓展接口写的内置拓展（overlay 插槽），示范内置功能与第三方拓展同一种写法。
   * 启动器在独立 Codex 实例注入前设置 window.__CODEX_PLUS_INSTANCE_LABEL__（或兼容的 __CODEX_PLUS_INSTANCE_ID__）；
   * 普通主实例没有这些值，不注册任何东西。
   */
  function codexPlusInstanceIdentityExtension(ctx) {
    const raw = window.__CODEX_PLUS_INSTANCE_LABEL__ || window.__CODEX_PLUS_INSTANCE_ID__;
    const label = String(raw || "").replace(/\s+/g, " ").trim();
    if (!label) return;
    ctx.registerSlot({
      slot: "overlay",
      order: 0,
      render: ({ container }) => {
        const node = document.createElement("div");
        node.id = "codex-plus-instance-identity";
        node.setAttribute("role", "status");
        node.setAttribute("aria-label", `当前 Codex++ 多开身份：${label}`);
        node.style.cssText = [
          "position:fixed",
          "top:8px",
          "left:50%",
          "transform:translateX(-50%)",
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
        node.textContent = `Codex++ 多开 · ${label}`;
        container.appendChild(node);
      },
    });
  }

  /**
   * 会话标题中间件：会话第一轮结束后，若有拓展挂了 `session/title` 中间件，
   * 以 { sessionId, title, firstMessage, lastMessage } 跑一遍链，结果的 title 与原标题不同就改名。
   * 每个会话只处理一次（本页面生命周期内）；没有中间件时什么也不做，也不启动会话观察。
   */
  function codexPlusSessionTitleExtension(ctx) {
    const handled = new Set();
    const firstMessages = new Map();
    let watching = null;
    const sync = () => {
      const wanted = codexPlusExtensionInterceptors.has("session/title");
      if (wanted && !watching) {
        watching = ctx.on("session/event", (event, { sessionId }) => {
          if (event?.type === "user/message" && !firstMessages.has(sessionId)) firstMessages.set(sessionId, String(event.data?.text ?? ""));
          if (event?.type !== "turn/complete" || handled.has(sessionId)) return;
          handled.add(sessionId);
          void codexPlusRunSessionTitle(sessionId, firstMessages.get(sessionId) ?? "", String(event.data?.lastMessage ?? ""));
        });
      } else if (!wanted && watching) {
        watching();
        watching = null;
      }
    };
    ctx.on("extensions/changed", sync);
    ctx.setTimeout(sync, 0);
  }

  async function codexPlusRunSessionTitle(sessionId, firstMessage, lastMessage) {
    const query = codexPlusServices.get("sessionQuery")?.value;
    const sessions = codexPlusServices.get("sessions")?.value;
    if (!query || !sessions) return;
    try {
      const title = String((await query.readTitle(sessionId)) ?? "");
      const result = await Promise.race([
        Promise.resolve(codexPlusRunWaterfall("session/title", { sessionId, title, firstMessage, lastMessage })),
        new Promise((resolve) => setTimeout(() => resolve(null), 30000)),
      ]);
      const next = String(result?.title ?? "").trim();
      if (next && next !== title) await sessions.rename(sessionId, next.slice(0, 120));
    } catch (error) {
      codexPlusMarkExtensionFailure("builtin:session-title-hook", String(error?.stack || error?.message || error));
    }
  }
