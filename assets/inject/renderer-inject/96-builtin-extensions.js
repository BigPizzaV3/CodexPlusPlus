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
   * 内置拓展清单。sync 在装上 / 卸下后调用，让功能按新的开关状态挂载或撤下（函数声明提前，
   * 这里引用后面分片里的函数没有问题；不存在时跳过）。apply 是完全用公开拓展接口写的内置拓展。
   * 管理器里的名字与说明在 Rust 侧 BUILTIN_EXTENSIONS 维护，两边 id 要一致。
   */
  function codexPlusBuiltinExtensionCatalog() {
    return [
      { id: "agent-pet", name: "Agent 桌宠", sync: () => syncCodexPlusAgentPet(), apply: codexPlusAgentPetExtension },
      { id: "usage-statusline", name: "用量状态栏", sync: () => window.__codexPlusUsageStatusLineRuntime?.sync?.(), apply: codexPlusUsageStatusLineExtension },
      { id: "dictation", name: "语音听写", sync: () => installCodexPlusDictation() },
      { id: "turn-rewind", name: "轮次回退", sync: () => syncCodexPlusTurnRewind() },
      { id: "typing-effects", name: "打字特效", sync: () => syncCodexPlusTypingEffects() },
      { id: "side-chat-save", name: "侧聊保存", sync: () => refreshSideChatSaveButtons() },
      { id: "history-turn-recovery", name: "历史轮次补全" },
      { id: "capacity-auto-retry", name: "容量不足自动重试" },
      { id: "proxy-request-status", name: "请求状态条", sync: () => window.__codexPlusProxyRequestStatusRuntime?.refresh?.(), apply: codexPlusProxyStatusExtension },
      { id: "session-share", name: "会话分享", sync: () => installSessionShareButton() },
      { id: "model-picker-groups", name: "模型分组", sync: () => window.__codexPlusModelPickerGroupsRuntime?.scan?.() },
      { id: "dream-skin", name: "梦皮主题", sync: () => refreshDreamSkin() },
      { id: "plugin-market-unlock", name: "插件市场解锁" },
      { id: "instance-identity", name: "多开身份提示", apply: codexPlusInstanceIdentityExtension },
      { id: "session-title-hook", name: "会话标题中间件", apply: codexPlusSessionTitleExtension },
      // 以下几项的开关都经 codexPlusSettings() 读（停用时对应设置项读成关），切换后补一轮扫描让界面跟上。
      { id: "session-delete", name: "会话删除", sync: () => scheduleDeferredScan() },
      { id: "markdown-export", name: "Markdown 导出", sync: () => scheduleDeferredScan() },
      { id: "thread-id-badge", name: "会话 ID 角标", sync: () => scheduleDeferredScan() },
      {
        id: "thread-scroll-restore",
        name: "滚动位置恢复",
        sync: () => {
          if (!codexPlusSettings().threadScrollRestore) stopThreadScrollRestore();
          scheduleDeferredScan();
        },
      },
      {
        id: "conversation-view",
        name: "对话宽度",
        sync: () => {
          refreshConversationViewControls();
          scheduleDeferredScan();
        },
      },
      { id: "activity-overlay-right", name: "消息导航条靠右", sync: () => globalThis.__CODEX_PLUS_ACTIVITY_OVERLAY_SYNC__?.() },
      // 每次粘贴时现查，不用同步。
      { id: "paste-fix", name: "粘贴修复" },
      // 悬浮球自己读后台设置再按停用集合判断，这里只让它重新算一次。
      { id: "stepwise", name: "下一步建议", sync: () => window.__codexStepwisePanel?.syncSettings?.() },
      { id: "answer-outline", name: "回答大纲", sync: () => window.__codexStepwisePanel?.syncSettings?.() },
      // 启动器每 5 秒按内置拓展状态决定是否驱动，页面侧无事可做。
      { id: "pet-real-mouse", name: "桌宠跟随鼠标" },
      // 以下由 Rust 侧（启动、注入、本地代理）按设置项与内置拓展状态生效，多数要重启 Codex，页面侧只登记。
      { id: "model-whitelist-unlock", name: "模型白名单解锁" },
      { id: "native-fast-unlock", name: "原生 Fast 强制解锁" },
      { id: "native-predictions", name: "原生输入预测" },
      { id: "reasoning-collapse", name: "第三方模型回答完自动折叠过程" },
      { id: "disable-work-view", name: "纯 API 模式禁用 Work 视图" },
      { id: "force-chinese-locale", name: "强制中文界面" },
      { id: "native-browser-identification", name: "原生 Edge / Chrome 请求标识兼容" },
      { id: "standalone-quota-unlock", name: "官方额度解锁" },
      { id: "image-overlay", name: "背景图" },
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

  const codexPlusBuiltinExtensionHandles = new Map();

  function codexPlusRunBuiltinSync(definition) {
    if (typeof definition.sync !== "function") return;
    runScanStep(() => definition.sync());
  }

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
              inactive.delete(definition.id);
              ctx.effect(() => () => {
                inactive.add(definition.id);
                codexPlusRunBuiltinSync(definition);
              });
              if (typeof definition.apply === "function") definition.apply(ctx);
              codexPlusRunBuiltinSync(definition);
            },
          }, { scriptKey: `builtin:${definition.id}` });
          codexPlusBuiltinExtensionHandles.set(definition.id, ctx);
        } catch (error) {
          codexPlusMarkExtensionFailure(`builtin:${definition.id}`, String(error?.stack || error?.message || error));
        }
      } else if (!wanted && handle) {
        codexPlusBuiltinExtensionHandles.delete(definition.id);
        handle.dispose();
      } else if (!wanted && !inactive.has(definition.id)) {
        // 还没装上过（注入早期按默认启用跑着）：直接记成停用，让功能撤下。
        inactive.add(definition.id);
        codexPlusRunBuiltinSync(definition);
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

  /** Agent 桌宠：经公开的 overlay 插槽拿容器，桌宠挂在里面（拖动、按压的逻辑在 97-agent-pet.js）。 */
  function codexPlusAgentPetExtension(ctx) {
    ctx.registerSlot({
      slot: "overlay",
      order: 10,
      render: ({ container }) => {
        codexPlusAgentPet.host = container;
        runScanStep(syncCodexPlusAgentPet);
        return () => {
          if (codexPlusAgentPet.host === container) codexPlusAgentPet.host = null;
          runScanStep(syncCodexPlusAgentPet);
        };
      },
    });
  }

  /** 用量状态栏：经公开的 composer.below 插槽拿每个输入框下方的容器，状态栏放进当前输入框的那个。 */
  function codexPlusUsageStatusLineExtension(ctx) {
    codexPlusUsageState.slotHosts ||= new Set();
    ctx.registerSlot({
      slot: "composer.below",
      order: 0,
      render: ({ container }) => {
        codexPlusUsageState.slotHosts.add(container);
        window.__codexPlusUsageStatusLineRuntime?.sync?.();
        return () => {
          codexPlusUsageState.slotHosts.delete(container);
          if (codexPlusUsageState.node?.parentElement === container) codexPlusUsageState.node.remove();
          window.__codexPlusUsageStatusLineRuntime?.sync?.();
        };
      },
    });
  }

  /** 请求状态条：经公开的 composer.above 插槽拿每个输入框上方的容器。 */
  function codexPlusProxyStatusExtension(ctx) {
    codexPlusProxyStatusState.slotHosts ||= new Set();
    ctx.registerSlot({
      slot: "composer.above",
      order: 0,
      render: ({ container }) => {
        codexPlusProxyStatusState.slotHosts.add(container);
        window.__codexPlusProxyRequestStatusRuntime?.refresh?.();
        return () => {
          codexPlusProxyStatusState.slotHosts.delete(container);
          if (codexPlusProxyStatusState.node?.parentElement === container) codexPlusProxyStatusState.node.remove();
        };
      },
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
