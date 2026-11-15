  /**
   * 内置拓展「Feature Lab」。
   *
   * Codex 的 app-server 已经暴露了 `experimentalFeature/list`：按 stage（stable / beta /
   * underDevelopment / deprecated / removed）、默认值、说明列出所有 feature。原生设置页只把
   * `stage === "beta"` 的一小部分画出来，而且排除了 multi_agent / plugins / remote_control /
   * realtime_* 等；这里补一个完整面板，方便排查「原生有、界面不出现」的功能。
   *
   * 写开关走 `config/batchWrite` 的 `features.<name>`（和原生 Fast 补丁同一路径），
   * 不直接改 config.toml。改动需要重启 Codex 才生效，面板里会明确提示。
   */
  const codexPlusFeatureLabStageMeta = {
    stable: { label: "稳定", tone: "stable", order: 0 },
    beta: { label: "Beta", tone: "beta", order: 1 },
    underDevelopment: { label: "开发中", tone: "dev", order: 2 },
    deprecated: { label: "已弃用", tone: "deprecated", order: 3 },
    removed: { label: "已移除", tone: "removed", order: 4 },
  };
  const codexPlusFeatureLabStageOrder = ["stable", "beta", "underDevelopment", "deprecated", "removed"];
  const codexPlusFeatureLabRiskyStages = new Set(["underDevelopment", "deprecated", "removed"]);
  const codexPlusFeatureLabNamePattern = /^[A-Za-z][A-Za-z0-9_]*$/;

  /**
   * 内置说明表。
   *
   * `experimentalFeature/list` 只给极少数 feature 带 displayName / description（实测 215 项里只有 5 项），
   * 其余在界面上只有裸的英文 key。这里按 key 补中文标题与说明，服务端下发了描述时以服务端为准。
   * 只收「名字能对得上行为、且从应用包/二进制里能核实」的项；拿不准的不写，界面照旧显示（没有说明）。
   */
  const codexPlusFeatureLabCatalog = {
    // —— 模型与工具调用 ——
    code_mode: { title: "Code mode", description: "让模型写一段 JS 编排多次工具调用，而不是一次只发一个工具调用。宿主进程 codex-code-mode-host 已随应用提供。" },
    code_mode_only: { title: "Code mode（仅此模式）", description: "只保留 code mode，不再暴露逐个工具调用。需先开 code_mode。" },
    code_mode_only_strict_3p_tools: { title: "Code mode 严格约束第三方工具", description: "第三方 MCP 工具也必须在 code mode 内调用。" },
    code_mode_prewarm: { title: "Code mode 预热", description: "提前启动 code mode 宿主，减少首次调用的等待。" },
    code_mode_tool_search: { title: "Code mode 工具检索", description: "工具多时先检索再调用，压缩提示词里的工具清单。" },
    deferred_executor: { title: "延迟执行器", description: "把工具执行推迟到批次末尾统一调度。" },
    deferred_tool_world_state: { title: "延迟工具的世界状态", description: "延迟执行的工具按同一份环境快照运行。" },
    tool_search: { title: "工具检索", description: "工具数量超过阈值时改为按需检索，而不是全部塞进上下文。" },
    tool_search_always_defer_mcp_tools: { title: "MCP 工具一律延迟暴露", description: "MCP 工具不预先注入，始终走检索。" },
    search_tool: { title: "搜索工具", description: "独立的搜索工具入口（removed 阶段，写入不一定生效，见面板提示）。" },
    standalone_web_search: { title: "独立网页搜索", description: "网页搜索作为独立能力，不依附于特定工具面。" },
    web_search_cached: { title: "网页搜索走缓存", description: "搜索引擎结果优先命中缓存，省流量、更快。" },
    web_search_request: { title: "网页搜索请求开关", description: "允许模型主动发起网页搜索请求。" },
    unified_exec: { title: "统一 exec 工具", description: "把命令执行收敛到一个工具里。" },
    unified_exec_tty: { title: "统一 exec 支持 TTY", description: "统一 exec 工具可开交互式终端。" },
    shell_tool: { title: "shell 工具", description: "内置 shell 命令工具。" },
    sleep_tool: { title: "sleep 工具", description: "允许模型主动等待一小段时间再继续。" },
    view_image: { title: "看图工具", description: "内置读取图片的工具。" },
    item_ids: { title: "给条目分配 id", description: "会话条目带稳定 id，便于撤销、去重与前端定位。" },
    content_item_kinds: { title: "条目类型细分", description: "内容条目区分更多类型，前端可按类型渲染。" },
    incremental_tools: { title: "增量工具流", description: "工具清单与结果按增量下发，减少整包重传。" },
    incremental_exec_tools: { title: "增量 exec 工具", description: "执行类工具的增量下发。" },
    instant_interrupt: { title: "立即中断", description: "中断指令不排队，立刻打断正在进行的回合。" },
    interrupt_on_cancel: { title: "取消即中断", description: "取消请求直接触发中断，不再等当前步骤跑完。" },

    // —— 多代理 / 子代理 ——
    multi_agent: { title: "多代理", description: "主会话可以派生子代理并行干活。" },
    multi_agent_v2: { title: "多代理 v2", description: "新版多代理编排。" },
    multi_agent_mode: { title: "多代理模式", description: "多代理的回合模式切换。" },
    enable_fanout: { title: "扇出", description: "一次任务扇出到多个代理。" },
    agent_message_board: { title: "代理留言板", description: "多个代理之间通过留言板交换消息。" },
    agent_message_board_mcp: { title: "代理留言板（MCP）", description: "把留言板以 MCP 工具的形式暴露给代理。" },
    wake_on_agent_messages: { title: "有消息即唤醒", description: "代理收到消息时唤醒休眠中的会话。" },
    send_message_to_user_async: { title: "异步给用户发消息", description: "代理可在回合之外主动给用户发消息。" },
    subagent_default_context_limits: { title: "子代理默认上下文上限", description: "给子代理设置默认的上下文窗口上限。" },
    subagent_thread_environments: { title: "子代理独立环境", description: "每个子代理拥有自己的运行环境。" },
    runtime_agent_instances: { title: "代理运行实例", description: "按实例管理运行中的代理。" },

    // —— 记忆 / 上下文 / 压缩 ——
    memories: { title: "记忆", description: "跨会话记忆：会话结束后提炼要点，下次带回来。" },
    external_agent_memory_import: { title: "导入外部 Agent 的记忆", description: "把别的 Agent（如 Claude Code）的记忆导进来。" },
    context_management: { title: "上下文管理", description: "按策略主动整理上下文，而不是只在超限时压缩。" },
    token_budget: { title: "token 预算", description: "给回合设定 token 预算，超出时按预算策略处理。" },
    rollout_budget: { title: "rollout 预算", description: "限制整段会话轨迹的预算。" },
    async_compaction: { title: "异步压缩", description: "长上下文压缩不阻塞下一轮，边压边跑。" },
    async_compaction_root_only: { title: "异步压缩仅根会话", description: "只有根会话做异步压缩，子会话不单独压。" },
    compaction_image_budget: { title: "压缩时保留图片预算", description: "压缩时给图片留出额度，避免图片被清空。" },
    remote_compaction_v2: { title: "远程压缩 v2", description: "压缩放到服务端做的新版实现。" },
    transcript_v2: { title: "会话记录 v2", description: "新版会话记录格式。" },
    local_thread_store_compression: { title: "本地会话库压缩", description: "压缩本地会话存储，长期使用后体积不至于失控。" },
    local_thread_store_shared_compression: { title: "本地会话库共享压缩", description: "多实例共享同一份压缩后的会话库。" },
    model_catalog_in_context: { title: "模型目录进上下文", description: "把模型目录随上下文一起下发，模型按目录选型。" },
    step_model_switching: { title: "按步骤切模型", description: "同一回合里按步骤切换模型。" },
    reasoning_effort_override: { title: "覆盖推理强度", description: "允许按会话覆盖默认推理强度。" },

    // —— 网络与凭据 ——
    network_proxy: { title: "网络代理限制", description: "对已开放网络的沙箱会话施加代理限制。" },
    respect_system_proxy: { title: "跟随系统代理", description: "Codex 请求使用 macOS / Windows 的系统代理设置。" },
    system_proxy_fallback: { title: "系统代理回退", description: "系统代理不可用时回退到直连或环境变量。" },
    enable_request_compression: { title: "请求压缩", description: "上行请求体压缩，弱网下更省带宽。" },
    unbounded_connection_retries: { title: "无限重连", description: "连接断开时不再限制重试次数。" },
    credential_masking: { title: "凭据打码", description: "日志与界面里的密钥自动打码。" },
    secret_auth_storage: { title: "密钥库存储", description: "密钥改存系统密钥库，而不是明文配置文件。" },

    // —— 权限与审批 ——
    guardian_approval: { title: "替我审批", description: "越界操作交给审批助手判断，只在检测到风险时才问你。" },
    guardian_include_additional_context: { title: "审批时附带更多上下文", description: "审批助手能看到更多会话上下文再作判断。" },
    guardian_reuse_parent_compaction: { title: "审批复用父会话压缩", description: "审批助手复用父会话的压缩结果，省 token。" },
    guardianv2: { title: "审批助手 v2", description: "新版审批助手。" },
    guardianv2_decisions_comparison: { title: "审批决策对比", description: "对比新旧审批助手的判断，用于调优。" },
    guardianv3: { title: "审批助手 v3", description: "实验中的审批助手第三代实现。" },
    exec_permission_approvals: { title: "命令执行走审批", description: "命令执行前先走权限审批流程。" },
    request_permissions_tool: { title: "权限申请工具", description: "模型可主动申请额外权限的工具。" },
    write_stdin_approval: { title: "写 stdin 需审批", description: "向运行中的进程写输入需要审批。" },

    // —— 会话与界面 ——
    goals: { title: "会话目标", description: "给会话设置长期目标，配合预算与提醒。" },
    hooks: { title: "Hooks", description: "在工具调用、会话开始结束等节点挂脚本。" },
    worktrees: { title: "Worktree", description: "为会话创建独立 git worktree。" },
    undo: { title: "撤销", description: "撤销上一步操作。" },
    steer: { title: "运行中纠偏", description: "会话运行中改方向，不打断当前回合。" },
    chronicle: { title: "Computer history", description: "按应用与网站记录活动，供记忆使用（可设排除名单）。" },
    artifact: { title: "Artifact", description: "生成可预览、可导出的文档 / 表格 / 幻灯片。" },
    image_generation: { title: "生成图片", description: "模型直接出图。" },
    image_generation_animation: { title: "图片生成动画", description: "生成过程的动态预览。" },
    image_resize_notice: { title: "图片缩放提示", description: "图片被缩放时告知模型。" },
    resize_all_images: { title: "统一缩放图片", description: "所有图片按统一规则缩放。" },
    unified_image_budget: { title: "统一图片预算", description: "图片共用一份额度。" },
    mentions_v2: { title: "提及 v2", description: "输入框 @ 提及文件/会话的新版实现。" },
    skill_search: { title: "技能检索", description: "技能多时按需检索。" },
    skill_catalog_deltas: { title: "技能目录增量", description: "技能清单按增量下发。" },
    skill_mcp_dependency_install: { title: "自动装技能依赖", description: "技能声明了 MCP 依赖时自动安装。" },
    plugins: { title: "插件", description: "插件系统。" },
    plugin_sharing: { title: "插件分享", description: "把插件分享给他人（plugin/share/*）。" },
    plugin_hooks: { title: "插件 hooks", description: "插件可以挂 hooks。" },
    recommended_plugins: { title: "推荐插件", description: "在界面里推荐插件。" },
    remote_plugin: { title: "远程插件", description: "从远端目录加载插件。" },
    apps: { title: "连接器", description: "ChatGPT 侧的连接器（Gmail、Drive 等）。" },
    in_app_browser: { title: "应用内浏览器", description: "内置浏览器面板。" },
    in_app_chat: { title: "应用内对话", description: "应用内嵌的对话视图。" },
    in_app_voice: { title: "应用内语音", description: "语音通话界面。" },
    in_app_dictation: { title: "应用内听写", description: "按住说话的语音转文字。" },
    in_app_local_automation: { title: "本地定时任务", description: "在本机按计划起会话。" },
    in_app_updates: { title: "应用内更新", description: "应用内检查并安装更新。" },
    computer_use: { title: "电脑操控", description: "让模型操作本机界面。" },
    browser_use: { title: "浏览器操控", description: "让模型驱动浏览器。" },
    browser_use_full_cdp_access: { title: "浏览器完整 CDP 权限", description: "允许走完整 CDP 协议控制浏览器。" },
    browser_use_external: { title: "外部浏览器", description: "操控本机已安装的浏览器。" },
    browser_annotation_api: { title: "浏览器标注 API", description: "在页面上标注并回传给模型。" },
    realtime_conversation: { title: "实时语音对话", description: "走 Realtime 后端的语音对话，需要官方登录。" },

    // —— 环境与沙箱 ——
    terminal_resize_reflow: { title: "终端缩放重排", description: "窗口尺寸变化时终端输出重新折行。" },
    shell_snapshot: { title: "shell 快照", description: "会话开始前给 shell 环境拍快照，加速后续命令。" },
    shell_snapshot_v2: { title: "shell 快照 v2", description: "新版 shell 快照实现。" },
    shell_zsh_fork: { title: "zsh fork 优化", description: "用 fork 方式起 zsh，减少启动开销。" },
    unified_exec_zsh_fork: { title: "统一 exec 用 zsh fork", description: "统一 exec 也走 zsh fork。" },
    login_shell_package_path: { title: "保留登录 shell 的工具", description: "登录 shell 重置 PATH 时仍保留 ripgrep 等内置工具。" },
    use_linux_sandbox_bwrap: { title: "Linux 用 bwrap 沙箱", description: "Linux 上改用 bubblewrap 做沙箱。" },
    use_legacy_landlock: { title: "旧版 Landlock", description: "回退旧的 Landlock 沙箱实现。" },
    windows_sandbox_service: { title: "Windows 沙箱服务", description: "Windows 上的沙箱服务。" },
    psp: { title: "PSP", description: "实验中的权限策略包。" },
    workspace_dependencies: { title: "工作区依赖", description: "按工作区解析与安装依赖。" },
    workspace_owner_usage_nudge: { title: "工作区用量提醒", description: "接近配额时提示工作区管理员。" },
    multi_env_copy_file: { title: "跨环境复制文件", description: "在多个运行环境之间复制文件。" },
    stable_environment_tools: { title: "稳定的环境工具", description: "环境相关工具使用稳定实现。" },

    // —— 性能与诊断 ——
    fast_mode: { title: "Fast 模式", description: "优先速度的模型与服务档位。" },
    ultrafast_mode: { title: "Ultrafast 模式", description: "更激进的提速档位。" },
    prevent_idle_sleep: { title: "运行时不休眠", description: "Codex 跑任务时让电脑不进入睡眠。" },
    analytics_plan_history: { title: "额度历史", description: "在 /analytics 里看五小时与每周额度历史。" },
    critical_path_timing: { title: "关键路径计时", description: "记录启动与回合各阶段的耗时。" },
    runtime_metrics: { title: "运行时指标", description: "采集运行时性能指标。" },
    concurrent_reasoning_summaries: { title: "并发推理摘要", description: "多段推理摘要并行生成。" },
    output_token_replay: { title: "输出 token 回放", description: "记录输出 token 便于回放与排查。" },
    app_server_websocket_compression: { title: "app-server 帧压缩", description: "app-server 的 WebSocket 帧压缩。" },
    omit_app_server_notification_media: { title: "通知里不带媒体", description: "通知消息省略图片等大字段。" },
    omit_skill_descriptions: { title: "省略技能描述", description: "下发技能清单时不带描述，省 token。" },

    // —— 会话组织 ——
    personality: { title: "人格", description: "给助手设定人格风格。" },
    collaboration_modes: { title: "协作模式", description: "Plan 等协作模式。" },
    default_mode_request_user_input: { title: "默认模式可主动提问", description: "非 Plan 模式下也能主动向你提问。" },
    prediction_preserve_parent_tools: { title: "预测保留父会话工具", description: "生成下一条预测时沿用父会话的工具集。" },
    retain_client_tool_outputs: { title: "保留客户端工具输出", description: "客户端工具的输出保留在上下文里。" },
    retain_client_developer_messages: { title: "保留开发者消息", description: "保留开发者角色消息不被裁剪。" },
    tool_suggest: { title: "工具建议", description: "根据当前任务建议可用的工具。" },
    tool_call_mcp_elicitation: { title: "工具调用触发追问", description: "MCP 工具可在调用中向你追问。" },
    mcp_connection_sharing: { title: "共享 MCP 连接", description: "多个会话共用同一条 MCP 连接。" },
    mcp_lazy_stdio_startup: { title: "MCP 懒启动", description: "stdio 型 MCP 按需启动，不占启动时间。" },
    mcp_oauth_refresh_coordination: { title: "MCP OAuth 刷新协调", description: "多进程协调 MCP 的 OAuth 刷新，避免互相顶掉。" },
    auth_elicitation: { title: "鉴权追问", description: "需要重新登录时在会话里直接追问。" },
    api_key_model_discovery: { title: "API Key 模式发现模型", description: "纯 API Key 也能从上游拉模型清单。" },
  };

  /** 内置说明表命中的 feature：界面上标「常用」，也可只看这些。 */
  function codexPlusFeatureLabCatalogEntry(name) {
    return Object.prototype.hasOwnProperty.call(codexPlusFeatureLabCatalog, name)
      ? codexPlusFeatureLabCatalog[name]
      : null;
  }

  function codexPlusFeatureLabStage(raw) {
    const stage = String(raw || "");
    return Object.prototype.hasOwnProperty.call(codexPlusFeatureLabStageMeta, stage) ? stage : "underDevelopment";
  }

  /**
   * 把 app-server 的 ExperimentalFeature 归一化；字段以后可能增加，缺的给空值。
   *
   * 标题/说明的优先级：服务端 displayName/description > 内置说明表 > 裸 key。
   * `known` 表示内置表里有这一项（界面标「常用」）。
   */
  function codexPlusFeatureLabNormalizeFeature(raw) {
    if (!raw || typeof raw !== "object" || typeof raw.name !== "string") return null;
    const name = raw.name.trim();
    if (!codexPlusFeatureLabNamePattern.test(name)) return null;
    const stage = codexPlusFeatureLabStage(raw.stage);
    const known = codexPlusFeatureLabCatalogEntry(name);
    const serverTitle = typeof raw.displayName === "string" && raw.displayName.trim()
      ? raw.displayName.trim()
      : typeof raw.title === "string" && raw.title.trim()
        ? raw.title.trim()
        : "";
    const serverDescription = typeof raw.description === "string" && raw.description.trim()
      ? raw.description.trim()
      : typeof raw.announcement === "string" ? raw.announcement.trim() : "";
    const title = serverTitle || known?.title || name;
    const description = serverDescription || known?.description || "";
    return {
      name,
      title,
      stage,
      description,
      known: Boolean(known),
      enabled: raw.enabled === true,
      defaultEnabled: raw.defaultEnabled === true,
      // removed 阶段不能直接判定为不可写：实测 search_tool 写了会生效，js_repl / remote_control 不会。
      // 所以不做静态锁定，交给写入后的回读校验（codexPlusFeatureLabWrite 后对比 enabled）。
      risky: codexPlusFeatureLabRiskyStages.has(stage),
      locked: false,
    };
  }

  function codexPlusFeatureLabCompare(left, right) {
    const stage = codexPlusFeatureLabStageOrder.indexOf(left.stage) - codexPlusFeatureLabStageOrder.indexOf(right.stage);
    if (stage) return stage;
    return left.title.localeCompare(right.title) || left.name.localeCompare(right.name);
  }

  /** 拉全量 feature（分页），按 stage / 名称排序。同一个客户端可以重复调用。 */
  async function codexPlusFeatureLabFetch(client) {
    if (!client || typeof client.sendRequest !== "function") {
      throw new Error("还没有连上 Codex 的 app-server");
    }
    const features = new Map();
    let cursor = null;
    for (let page = 0; page < 20; page += 1) {
      const result = await client.sendRequest("experimentalFeature/list", { cursor, limit: 100 });
      const data = Array.isArray(result?.data) ? result.data : Array.isArray(result) ? result : [];
      for (const raw of data) {
        const feature = codexPlusFeatureLabNormalizeFeature(raw);
        if (feature && !features.has(feature.name)) features.set(feature.name, feature);
      }
      const next = typeof result?.nextCursor === "string" && result.nextCursor ? result.nextCursor : null;
      if (!next || next === cursor) break;
      cursor = next;
    }
    return [...features.values()].sort(codexPlusFeatureLabCompare);
  }

  /** 写入 `features.<name>`。原生 Fast 补丁用同一条 config/batchWrite 路径。 */
  async function codexPlusFeatureLabWrite(client, feature, enabled) {
    if (!feature || !codexPlusFeatureLabNamePattern.test(String(feature.name || ""))) {
      throw new Error("feature 名称不合法");
    }
    return client.sendRequest("config/batchWrite", {
      edits: [{
        keyPath: `features.${feature.name}`,
        value: enabled === true,
        mergeStrategy: "upsert",
      }],
      filePath: null,
      expectedVersion: null,
      reloadUserConfig: true,
    });
  }

  /**
   * 写入后回读一个 feature 当前是否真的生效。
   *
   * `config/batchWrite` 对任何 key 都返回 ok，连不存在的 feature 也会写进 config.toml；
   * 而 `removed` 阶段里有的会生效（search_tool）、有的不会（js_repl、remote_control）。
   * 所以「写成功」不等于「已生效」，必须回读列表里同名的 enabled 才能给准话。
   * 返回 true / false / null（列表里没有这一项，无从判断）。
   */
  async function codexPlusFeatureLabVerify(client, name) {
    if (!client || typeof client.sendRequest !== "function") return null;
    let cursor = null;
    for (let page = 0; page < 20; page += 1) {
      const result = await client.sendRequest("experimentalFeature/list", { cursor, limit: 100 });
      const data = Array.isArray(result?.data) ? result.data : Array.isArray(result) ? result : [];
      const hit = data.find((raw) => raw && raw.name === name);
      if (hit) return hit.enabled === true;
      const next = typeof result?.nextCursor === "string" && result.nextCursor ? result.nextCursor : null;
      if (!next || next === cursor) break;
      cursor = next;
    }
    return null;
  }

  function codexPlusFeatureLabStyles() {
    return `
      .codex-plus-feature-lab {
        display: flex; flex-direction: column; gap: 12px;
        box-sizing: border-box; width: 100%; max-width: 920px;
        min-height: 100%; padding: 18px 22px 36px;
        color: light-dark(#1f2328, #e8e8e8);
        background: transparent;
        font: 13px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      .codex-plus-feature-lab * { box-sizing: border-box; }
      .codex-plus-feature-lab h2 { margin: 0; font-size: 17px; font-weight: 600; }
      .codex-plus-feature-lab p { margin: 0; }
      .codex-plus-feature-lab .cpl-sub { color: light-dark(#57606a, #9aa4ad); }
      .codex-plus-feature-lab .cpl-toolbar { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
      .codex-plus-feature-lab input[type="search"],
      .codex-plus-feature-lab select {
        height: 30px; padding: 0 9px; border-radius: 7px;
        border: 1px solid light-dark(#d0d7de, #3a3f45);
        background: light-dark(#fff, #1f2328);
        color: inherit; font: inherit; outline: none;
      }
      .codex-plus-feature-lab input[type="search"] { flex: 1 1 240px; min-width: 160px; }
      .codex-plus-feature-lab button {
        height: 30px; padding: 0 11px; border-radius: 7px; cursor: pointer;
        border: 1px solid light-dark(#d0d7de, #3a3f45);
        background: light-dark(#f6f8fa, #262b31);
        color: inherit; font: inherit;
      }
      .codex-plus-feature-lab button:hover:not(:disabled) { background: light-dark(#eef1f4, #2f353c); }
      .codex-plus-feature-lab button:disabled { cursor: default; opacity: .55; }
      .codex-plus-feature-lab .cpl-status { min-height: 18px; color: light-dark(#57606a, #9aa4ad); }
      .codex-plus-feature-lab .cpl-status[data-tone="error"] { color: light-dark(#b42318, #ff8a80); }
      .codex-plus-feature-lab .cpl-status[data-tone="ok"] { color: light-dark(#1a7f37, #7ee2a8); }
      .codex-plus-feature-lab .cpl-status[data-tone="warn"] { color: light-dark(#8a6d00, #f2cc60); }
      .codex-plus-feature-lab .cpl-known { display: inline-flex; align-items: center; gap: 5px; color: light-dark(#57606a, #b6bec6); cursor: pointer; user-select: none; }
      .codex-plus-feature-lab .cpl-known input { appearance: auto !important; margin: 0; }
      .codex-plus-feature-lab .cpl-list { display: flex; flex-direction: column; gap: 7px; }
      .codex-plus-feature-lab .cpl-row {
        display: flex; gap: 14px; align-items: flex-start;
        padding: 11px 13px; border-radius: 9px;
        border: 1px solid light-dark(#e1e7ec, #333940);
        background: light-dark(#fff, #20252b);
      }
      .codex-plus-feature-lab .cpl-row[data-enabled="true"] { border-color: light-dark(#b7dfc4, #2f6b45); }
      .codex-plus-feature-lab .cpl-main { min-width: 0; flex: 1 1 auto; display: flex; flex-direction: column; gap: 3px; }
      .codex-plus-feature-lab .cpl-title { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; font-weight: 600; }
      .codex-plus-feature-lab .cpl-name { font: 11px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; color: light-dark(#6e7781, #8b949e); }
      .codex-plus-feature-lab .cpl-desc { color: light-dark(#57606a, #b6bec6); }
      .codex-plus-feature-lab .cpl-meta { font-size: 11px; color: light-dark(#8c959f, #8b949e); }
      .codex-plus-feature-lab .cpl-stage {
        display: inline-flex; align-items: center; height: 18px; padding: 0 6px;
        border-radius: 999px; font-size: 11px; font-weight: 500;
        border: 1px solid transparent;
      }
      .codex-plus-feature-lab .cpl-stage[data-tone="stable"] { color: light-dark(#1a7f37, #7ee2a8); background: light-dark(#dafbe1, #1b3326); }
      .codex-plus-feature-lab .cpl-stage[data-tone="beta"] { color: light-dark(#8a6d00, #f2cc60); background: light-dark(#fff8c5, #3a2f10); }
      .codex-plus-feature-lab .cpl-stage[data-tone="dev"] { color: light-dark(#9a3412, #ffb27d); background: light-dark(#ffebe9, #421f18); }
      .codex-plus-feature-lab .cpl-stage[data-tone="deprecated"] { color: light-dark(#6e7781, #b6bec6); background: light-dark(#eef1f4, #2c3238); }
      .codex-plus-feature-lab .cpl-stage[data-tone="removed"] { color: light-dark(#b42318, #ff8a80); background: light-dark(#ffebe9, #421f18); }
      .codex-plus-feature-lab .cpl-stage[data-tone="known"] { color: light-dark(#0550ae, #79c0ff); background: light-dark(#ddf4ff, #102a43); }
      .codex-plus-feature-lab .cpl-switch {
        position: relative; flex: 0 0 auto; width: 38px; height: 22px; padding: 0;
        border-radius: 999px; border: 1px solid light-dark(#c8d0d8, #454b52);
        background: light-dark(#eaeef2, #333940); transition: background .15s ease;
      }
      .codex-plus-feature-lab .cpl-switch::after {
        content: ""; position: absolute; top: 2px; left: 2px; width: 16px; height: 16px;
        border-radius: 50%; background: light-dark(#fff, #d8dde2);
        box-shadow: 0 1px 2px rgb(0 0 0 / .28); transition: transform .15s ease;
      }
      .codex-plus-feature-lab .cpl-switch[aria-checked="true"] {
        border-color: transparent; background: light-dark(#2da44e, #3fb950);
      }
      .codex-plus-feature-lab .cpl-switch[aria-checked="true"]::after { transform: translateX(16px); background: #fff; }
      .codex-plus-feature-lab .cpl-empty { padding: 22px 0; color: light-dark(#57606a, #9aa4ad); }
      .codex-plus-feature-lab .cpl-note {
        padding: 9px 11px; border-radius: 8px;
        background: light-dark(#f6f8fa, #262b31);
        color: light-dark(#57606a, #b6bec6); font-size: 12px;
      }
    `;
  }

  function codexPlusFeatureLabEnsureStyles() {
    const id = "codex-plus-feature-lab-style";
    if (document.getElementById(id)) return;
    const style = document.createElement("style");
    style.id = id;
    style.textContent = codexPlusFeatureLabStyles();
    document.head.appendChild(style);
  }

  /** 设置页分区本体。渲染是瞬态：每次打开都重新拉一次列表。 */
  function codexPlusFeatureLabRender({ container, ctx }) {
    if (!container) return;
    codexPlusFeatureLabEnsureStyles();
    const root = document.createElement("div");
    root.className = "codex-plus-feature-lab";
    root.innerHTML = `
      <div>
        <h2>Feature Lab</h2>
        <p class="cpl-sub">Codex app-server 暴露的全部实验开关。改动写入 config.toml 的 <code>features.&lt;name&gt;</code>，重启 Codex 后生效。</p>
      </div>
      <div class="cpl-toolbar">
        <input type="search" placeholder="搜索名称或说明" aria-label="搜索 feature" />
        <select aria-label="按阶段过滤">
          <option value="all">全部阶段</option>
          <option value="stable">稳定</option>
          <option value="beta">Beta</option>
          <option value="underDevelopment">开发中</option>
          <option value="deprecated">已弃用</option>
          <option value="removed">已移除</option>
        </select>
        <label class="cpl-known">
          <input type="checkbox" class="cpl-known-input" />
          只看常用
        </label>
        <button type="button" class="cpl-reload">重新读取</button>
        <span class="cpl-count"></span>
      </div>
      <div class="cpl-status" role="status"></div>
      <div class="cpl-list"></div>
      <div class="cpl-note">开发中 / 已弃用的 feature 可能让会话异常或丢失数据；已移除的写入后不一定生效，面板会回读并如实提示。中文说明由 Codex++ 补充，服务端带了说明时以服务端为准。服务端校验的能力（云端任务、真实配额）不受这里影响。</div>
    `;
    container.replaceChildren(root);

    const search = root.querySelector("input[type=search]");
    const stageSelect = root.querySelector("select");
    const reload = root.querySelector(".cpl-reload");
    const status = root.querySelector(".cpl-status");
    const count = root.querySelector(".cpl-count");
    const list = root.querySelector(".cpl-list");
    const knownOnly = root.querySelector(".cpl-known-input");
    const state = { features: [], query: "", stage: "all", knownOnly: false, loading: false };

    const setStatus = (message, tone = "") => {
      if (!status.isConnected) return;
      status.textContent = message || "";
      status.dataset.tone = tone;
    };

    const visibleFeatures = () => {
      const query = state.query.trim().toLowerCase();
      return state.features.filter((feature) => {
        if (state.stage !== "all" && feature.stage !== state.stage) return false;
        if (state.knownOnly && !feature.known) return false;
        if (!query) return true;
        return `${feature.name} ${feature.title} ${feature.description}`.toLowerCase().includes(query);
      });
    };

    const draw = () => {
      if (!list.isConnected) return;
      const features = visibleFeatures();
      count.textContent = state.loading
        ? ""
        : `${features.length} / ${state.features.length} 项`;
      if (!features.length) {
        list.innerHTML = `<div class="cpl-empty">${state.loading ? "正在读取…" : "没有匹配的 feature"}</div>`;
        return;
      }
      const fragment = document.createDocumentFragment();
      for (const feature of features) {
        const row = document.createElement("div");
        row.className = "cpl-row";
        row.dataset.enabled = feature.enabled ? "true" : "false";
        row.dataset.name = feature.name;

        const main = document.createElement("div");
        main.className = "cpl-main";
        const title = document.createElement("div");
        title.className = "cpl-title";
        const titleText = document.createElement("span");
        titleText.textContent = feature.title;
        const name = document.createElement("span");
        name.className = "cpl-name";
        name.textContent = feature.name;
        const stage = document.createElement("span");
        stage.className = "cpl-stage";
        stage.dataset.tone = codexPlusFeatureLabStageMeta[feature.stage]?.tone || "dev";
        stage.textContent = codexPlusFeatureLabStageMeta[feature.stage]?.label || feature.stage;
        title.append(titleText, name, stage);
        if (feature.known) {
          const badge = document.createElement("span");
          badge.className = "cpl-stage";
          badge.dataset.tone = "known";
          badge.textContent = "常用";
          title.appendChild(badge);
        }

        const desc = document.createElement("div");
        desc.className = "cpl-desc";
        desc.textContent = feature.description || "（没有说明）";

        const meta = document.createElement("div");
        meta.className = "cpl-meta";
        const defaultText = feature.defaultEnabled ? "开" : "关";
        meta.textContent = `默认${defaultText}${feature.stage === "removed" ? " · 已移除，写入后不一定生效" : ""}`;

        main.append(title, desc, meta);

        const toggle = document.createElement("button");
        toggle.type = "button";
        toggle.className = "cpl-switch";
        toggle.setAttribute("role", "switch");
        toggle.setAttribute("aria-checked", feature.enabled ? "true" : "false");
        toggle.setAttribute("aria-label", `${feature.title} 开关`);
        toggle.disabled = feature.locked;
        toggle.addEventListener("click", () => {
          void toggleFeature(feature, toggle);
        });

        row.append(main, toggle);
        fragment.appendChild(row);
      }
      list.replaceChildren(fragment);
    };

    const toggleFeature = async (feature, button) => {
      if (state.loading || feature.locked) return;
      const next = !feature.enabled;
      if (next && feature.risky) {
        const answer = await ctx.ask({
          title: `开启「${feature.title}」？`,
          message: feature.stage === "removed"
            ? "该 feature 已在当前版本标记为「已移除」：写入后不一定生效，生效的话行为也可能和旧版本不一样。确定要试试吗？"
            : `该 feature 标记为「${codexPlusFeatureLabStageMeta[feature.stage]?.label || feature.stage}」，可能让会话异常或丢失数据。确定要打开吗？`,
          options: [{ value: "enable", label: "仍然开启" }],
        });
        if (!answer || answer.value !== "enable") return;
      }
      button.disabled = true;
      setStatus(`正在写入 features.${feature.name}…`);
      try {
        const client = await codexPlusAppServerClient("local");
        await codexPlusFeatureLabWrite(client, feature, next);
        // 写入静默接受不等于生效：回读一次，按实际值给准话。
        const actual = await codexPlusFeatureLabVerify(client, feature.name).catch(() => null);
        if (actual === null) {
          feature.enabled = next;
          setStatus(`已写入 features.${feature.name} = ${next}，但列表里没有这一项，无法确认是否生效。`, "warn");
        } else if (actual === next) {
          feature.enabled = next;
          setStatus(`已${next ? "开启" : "关闭"} ${feature.name}，重启 Codex 后生效。`, "ok");
        } else {
          feature.enabled = actual;
          setStatus(
            `features.${feature.name} 已写入，但当前版本不接受这个开关（仍为${actual ? "开" : "关"}）。`
            + "该 feature 可能已在本版本移除或合并到别的开关里。",
            "warn",
          );
        }
        button.setAttribute("aria-checked", feature.enabled ? "true" : "false");
        button.closest(".cpl-row")?.setAttribute("data-enabled", feature.enabled ? "true" : "false");
      } catch (error) {
        setStatus(`写入失败：${error?.message || error}`, "error");
      } finally {
        button.disabled = feature.locked || state.loading;
      }
    };

    const load = async () => {
      state.loading = true;
      reload.disabled = true;
      setStatus("正在读取 feature 列表…");
      draw();
      try {
        const client = await codexPlusAppServerClient("local");
        state.features = await codexPlusFeatureLabFetch(client);
        state.loading = false;
        setStatus(state.features.length ? "" : "app-server 没有返回 feature。", state.features.length ? "" : "error");
      } catch (error) {
        state.loading = false;
        state.features = [];
        setStatus(`读取失败：${error?.message || error}`, "error");
      } finally {
        reload.disabled = false;
        draw();
      }
    };

    search.addEventListener("input", () => {
      state.query = search.value;
      draw();
    });
    stageSelect.addEventListener("change", () => {
      state.stage = stageSelect.value;
      draw();
    });
    knownOnly.addEventListener("change", () => {
      state.knownOnly = knownOnly.checked;
      draw();
    });
    reload.addEventListener("click", () => void load());
    void load();
  }

  function codexPlusFeatureLabExtension(ctx) {
    ctx.registerSettingsSection({
      id: "feature-lab",
      title: "Feature Lab",
      order: 90,
      render: ({ container }) => codexPlusFeatureLabRender({ container, ctx }),
    });
  }
