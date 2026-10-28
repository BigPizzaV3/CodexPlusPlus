type CommandResult = {
  status: string;
  message: string;
  [key: string]: unknown;
};

const ok = <T extends Record<string, unknown>>(payload: T = {} as T): CommandResult & T => ({
  status: "ok",
  message: "演示模式",
  ...payload,
});

const demoSettings = {
  revision: "web-preview",
  codexAppPath: "/Applications/Codex.app",
  codexAppAutoTakeover: false,
  codexAppAutoTakeoverWhenActive: "defer",
  codexExtraArgs: [],
  dictation: {
    enabled: false,
    provider: "api",
    baseUrl: "https://api.groq.com/openai/v1",
    apiKey: "",
    apiKeyEnv: "",
    model: "whisper-large-v3-turbo",
    language: "",
    timeoutSeconds: 120,
  },
  providerSyncEnabled: false,
  ccsDbPath: "",
  providerSyncSavedProviders: [],
  providerSyncManualProviders: [],
  providerSyncLastSelectedProvider: "",
  relayProfilesEnabled: true,
  localConfigProviderImported: false,
  enhancementsEnabled: true,
  codexAppStandaloneQuotaUnlock: false,
  codexAppPluginMarketplaceUnlock: true,
  codexAppModelWhitelistUnlock: true,
  codexAppSessionDelete: true,
  codexAppMarkdownExport: true,
  codexAppUsageStatusLine: true,
  codexAppPasteFix: false,
  codexAppForceChineseLocale: false,
  codexAppTypingEffect: "off",
  codexAppThreadIdBadge: false,
  codexAppConversationView: false,
  codexAppThreadScrollRestore: true,
  codexAppNativeMenuPlacement: true,
  codexAppNativeBrowserRequireIdentification: false,
  codexAppNativeFastUnlock: false,
  codexAppPetRealMouseLook: false,
  codexAppActivityOverlayRight: false,
  protocolProxyAutoRetry: true,
  protocolProxyAutoRetryMaxAttempts: 5,
  protocolProxyAutoRetryIntervalSeconds: 2,
  protocolProxyAutoRetryMode: "streaming",
  protocolProxyAutoRetryCommitDelayMs: 1000,
  protocolProxyMapUnlistedModels: false,
  helperMaxRequestBodyMib: 128,
  codexAppCapacityAutoRetry: false,
  codexAppStepwiseEnabled: false,
  codexAppAnswerOutlineEnabled: false,
  codexAppStepwiseDirectSend: false,
  codexAppStepwiseProtocol: "chat_completions",
  codexAppStepwiseGenerationMode: "auto",
  codexAppStepwiseBaseUrl: "",
  codexAppStepwiseApiKey: "",
  codexAppStepwiseApiKeyEnv: "CODEX_STEPWISE_API_KEY",
  codexAppStepwiseModel: "",
  codexAppStepwiseMaxItems: 4,
  codexAppStepwiseMaxInputChars: 6000,
  codexAppStepwiseMaxOutputTokens: 500,
  codexAppStepwiseTimeoutMs: 8000,
  codexAppImageOverlayEnabled: false,
  codexAppImageOverlayPath: "",
  codexAppImageOverlayOpacity: 35,
  codexAppImageOverlayFitMode: "fit",
  codexAppDreamSkinEnabled: false,
  codexAppDreamSkinPaused: false,
  codexAppDreamSkinTheme: "pink",
  codexAppDreamSkinThemeConfig: {},
  codexAppDreamSkinImagePath: "",
  codexGoalsEnabled: false,
  weixinConnectEnabled: false,
  weixinConnectBaseUrl: "https://ilinkai.weixin.qq.com",
  weixinConnectToken: "",
  weixinConnectAccountId: "",
  weixinConnectAllowFrom: "",
  weixinConnectRouteTag: "",
  weixinConnectWorkDir: "",
  weixinConnectModel: "",
  weixinConnectSandbox: "read-only",
  weixinConnectCodexPath: "",
  relayBaseUrl: "",
  relayApiKey: "",
  relayProfiles: [{
    id: "default",
    name: "默认中转",
    model: "deepseek-v4-pro[1M]",
    baseUrl: "https://api.deepseek.com/v1",
    upstreamBaseUrl: "https://api.deepseek.com/v1",
    apiKey: "",
    protocol: "responses",
    relayMode: "pureApi",
    officialMixApiKey: false,
    hideOfficialUsageAlert: false,
    testModel: "deepseek-v4-pro",
    configContents: "",
    authContents: "",
    useCommonConfig: true,
    contextSelection: { mcpServers: [], skills: [], plugins: [] },
    contextSelectionInitialized: true,
    contextWindow: "1000000",
    autoCompactLimit: "800000",
    modelList: "deepseek-v4-pro[1M]\ndeepseek-v4-flash[128K]",
    modelWindows: "",
    modelAutoCompact: "",
    modelMetadata: "",
    modelVlm: "",
    vlmApiKey: "",
    vlmModel: "",
    vlmBaseUrl: "",
    userAgent: "",
    customHeaders: [],
    sub2apiEnabled: false,
    noAuth: false,
    sub2apiMultiplier: "",
    standardOpenaiProtocol: false,
    webSearchHistoryCompat: false,
    strictResponsesReasoning: false,
    anthropicAuthHeader: "x-api-key",
    anthropicMaxTokens: 0,
    anthropicPromptCache: true,
    anthropicImpersonateClaudeCode: false,
    anthropicThinkingMode: "",
  }],
  relayCommonConfigContents: "",
  relayPreserveNonproviderSettings: false,
  relayContextConfigContents: "",
  activeRelayId: "default",
  aggregateRelayProfiles: [],
  activeAggregateRelayId: "",
  relayTestModel: "gpt-5.4-mini",
  tools: {},
  activeTool: "codex",
  agentPet: {
    enabled: false,
    agents: ["codex"],
    image: "whale",
    customImageUrl: "",
    size: 112,
    soundEnabled: true,
    soundSet: "duck",
    volume: 0.6,
  },
};

// 网页预览不走后端缓存，直接引用 jsDelivr 上的同一份图片。
const agentPetCatalog: Array<[string, string, string]> = [
  ["whale", "鲸鱼", "Spouting whale/3D/spouting_whale_3d.png"],
  ["cat", "小猫", "Cat/3D/cat_3d.png"],
  ["dog", "小狗", "Dog face/3D/dog_face_3d.png"],
  ["fox", "狐狸", "Fox/3D/fox_3d.png"],
  ["panda", "熊猫", "Panda/3D/panda_3d.png"],
  ["penguin", "企鹅", "Penguin/3D/penguin_3d.png"],
  ["hamster", "仓鼠", "Hamster/3D/hamster_3d.png"],
  ["rabbit", "兔子", "Rabbit face/3D/rabbit_face_3d.png"],
  ["frog", "青蛙", "Frog/3D/frog_3d.png"],
  ["octopus", "章鱼", "Octopus/3D/octopus_3d.png"],
  ["chick", "小鸡", "Baby chick/3D/baby_chick_3d.png"],
  ["duck", "鸭子", "Duck/3D/duck_3d.png"],
  ["dolphin", "海豚", "Dolphin/3D/dolphin_3d.png"],
  ["otter", "水獭", "Otter/3D/otter_3d.png"],
  ["unicorn", "独角兽", "Unicorn/3D/unicorn_3d.png"],
  ["dino", "恐龙", "Sauropod/3D/sauropod_3d.png"],
  ["robot", "机器人", "Robot/3D/robot_3d.png"],
  ["ghost", "小幽灵", "Ghost/3D/ghost_3d.png"],
];

function agentPetPreviewImage(image: string, customImageUrl: string) {
  if (image === "custom") {
    return /^https:\/\//.test(customImageUrl)
      ? { status: "ok", image: customImageUrl }
      : { status: "failed", message: "自定义图片只支持 https:// 地址" };
  }
  const entry = agentPetCatalog.find(([id]) => id === image);
  if (!entry) return { status: "failed", message: `未知的内置桌宠图片：${image}` };
  const path = entry[2].split("/").map(encodeURIComponent).join("/");
  return { status: "ok", image: `https://cdn.jsdelivr.net/gh/microsoft/fluentui-emoji@1ffb34c752ecf5d402f04cfb4b392c77f57c54bc/assets/${path}` };
}

const overview = ok({
  codex_app: { status: "found", path: "/Applications/Codex.app" },
  codex_version: "1.10.0",
  silent_shortcut: { status: "installed", path: null },
  management_shortcut: { status: "installed", path: null },
  latest_launch: null,
  runtime_health: {
    codex_app: { status: "stopped", address: null, message: "等待启动" },
    local_server: { status: "running", address: "127.0.0.1:63122", message: "本地服务已就绪" },
    debugger: { status: "disconnected", address: null, message: "等待 Codex 连接" },
    protocol_conversion_enabled: true,
  },
  current_version: "2.0.0",
  update_status: "up_to_date",
  settings_path: "演示模式",
  logs_path: "演示模式",
});

const tools = ok({
  tools: [
    { id: "codex", name: "Codex", homeDir: "~/.codex", switchable: true, active: true, activeRelayName: "默认中转", relayCount: 3 },
    { id: "grok", name: "Grok", homeDir: "~/.grok", switchable: false, active: false, activeRelayName: null, relayCount: 0 },
    { id: "claude", name: "Claude Code", homeDir: "~/.claude", switchable: true, active: false, activeRelayName: null, relayCount: 0 },
  ],
  activeTool: "codex",
});

// 网关演示数据：Claude Code 已接入、Grok 未接入，池里三家供应商，其中一家有 Key 在冷却。
const gatewayBinding = (model: string) => ({
  enabled: true,
  model,
  extraModels: [],
  claudeRoles: { opus: "", sonnet: "", haiku: "", subagent: "" },
  unknownModel: "mapToDefault",
  claudeEnableEffort: false,
  token: "demo",
});
const gateway = ok({
  gateway: { version: 1, bindings: { claude: gatewayBinding("deepseek-v4-pro") }, keyPolicy: { mode: "sticky" }, restoreOnExit: true },
  port: 63130,
  helperRunning: true,
  pool: [
    { id: "deepseek", name: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", protocol: "chatCompletions", official: true, models: ["deepseek-v4-pro", "deepseek-v4-flash"], keyCount: 3 },
    { id: "kimi", name: "Kimi", baseUrl: "https://api.moonshot.cn/v1", protocol: "chatCompletions", official: true, models: ["kimi-k2-turbo"], keyCount: 2 },
    { id: "zhipu", name: "智谱 GLM", baseUrl: "https://open.bigmodel.cn/api/paas/v4", protocol: "chatCompletions", official: true, models: ["glm-5"], keyCount: 1 },
  ],
  groups: [],
  agents: [
    { agent: "claude", name: "Claude Code", enabled: true, baseUrl: "http://127.0.0.1:63130/claude", defaultModel: "deepseek-v4-pro", models: [] },
    { agent: "grok", name: "Grok", enabled: false, baseUrl: "http://127.0.0.1:63130/grok", defaultModel: "", models: [] },
  ],
  health: { keys: [{ relayId: "zhipu", keyId: "k1", state: "cooling", remainingMs: 42000 }], providers: [] },
  activity: {
    agents: [{
      agent: "claude",
      requests: 396,
      rerouted: 14,
      errors: 1,
      inFlight: [],
      last: {
        requestedModel: "claude-sonnet-4-5",
        model: "deepseek-v4-pro",
        planned: [{ relayId: "deepseek", relayName: "DeepSeek", model: "deepseek-v4-pro" }],
        steps: [{ relayId: "deepseek", relayName: "DeepSeek", keyId: "k1", model: "deepseek-v4-pro", outcome: "answered", elapsedMs: 1840 }],
        result: "answered",
        finishedAtMs: Date.now() - 8000,
        elapsedMs: 1840,
      },
    }],
  },
  // 导入弹窗演示数据：带品牌图标的、只显示首字母的都有，看头像是否保持固定尺寸。
  codexProviders: [
    { id: "hj", name: "哈基米 国模", protocol: "responses", baseUrl: "https://api.biuapi.com" },
    { id: "kimi", name: "Kimi 官方", protocol: "responses", baseUrl: "https://api.moonshot.cn" },
    { id: "deepseek", name: "DeepSeek", protocol: "responses", baseUrl: "https://api.deepseek.com" },
  ],
  claudeProviders: [{ id: "official", name: "Claude Official" }],
  grokProfiles: [],
});
const usageCounts = (calls: number, errors = 0) => ({ calls, errors, inputTokens: calls * 9000, outputTokens: calls * 700, cachedTokens: calls * 5600, reasoningTokens: 0 });
const gatewayUsage = ok({
  totals: usageCounts(396, 5),
  // 近 7 天，用来看用量图表的悬停效果
  days: [120, 260, 90, 310, 180, 40, 396].map((calls, index) => ({ date: new Date(Date.now() - (6 - index) * 86_400_000).toISOString().slice(0, 10), ...usageCounts(calls, index === 3 ? 4 : 0) })),
  byAgent: [{ key: "claude", label: "claude", ...usageCounts(396, 5) }],
  byProvider: [{ key: "deepseek", label: "DeepSeek", ...usageCounts(281, 2) }, { key: "kimi", label: "Kimi", ...usageCounts(115, 3) }],
  byModel: [],
  costs: [{ currency: "CNY", amount: 12.84 }],
});

// 缓存清理演示数据：三个应用、各类目录，数字取自一台常用机器的量级。
const cacheGroup = (id: string, app: string, kind: string, totalMiB: number, eligibleMiB: number, files: number) => ({
  id, app, kind, path: `~/Library/Caches/${app}/${kind}`, totalBytes: totalMiB * 1048576, eligibleBytes: eligibleMiB * 1048576,
  eligibleFiles: files, skippedFiles: 3, cleanable: kind !== "temporary",
});
const agentCacheReport = {
  scanId: "demo",
  warnings: [],
  groups: [
    cacheGroup("codex-cache", "Codex", "cache", 83, 80, 412),
    cacheGroup("codex-logs", "Codex", "logs", 221, 153, 96),
    cacheGroup("codex-tmp", "Codex", "temporary", 894, 0, 0),
    cacheGroup("claude-cache", "Claude", "cache", 23, 23, 120),
    cacheGroup("claude-logs", "Claude", "logs", 36, 36, 40),
    cacheGroup("plus-update", "Codex++", "update", 459, 459, 2),
    cacheGroup("plus-cache", "Codex++", "cache", 0.07, 0.07, 6),
  ],
};

// 推荐内容演示数据：中性名称，横幅用内联 SVG，不请求外部图片。
const demoBanner = (label: string, hue: number) =>
  `data:image/svg+xml;utf8,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="640" height="200"><defs><linearGradient id="g" x1="0" x2="1"><stop offset="0" stop-color="hsl(${hue} 60% 18%)"/><stop offset="1" stop-color="hsl(${hue + 40} 60% 30%)"/></linearGradient></defs><rect width="640" height="200" fill="url(#g)"/><text x="40" y="118" font-family="Helvetica" font-size="44" font-weight="700" fill="white">${label}</text></svg>`)}`;
const demoAds = [
  { id: "demo-a", type: "sponsor", title: "示例中转站 A", description: "稳定的 API 中转服务，支持主流模型与图像能力，适合日常开发和团队协作。", url: "https://example.com/a", image: demoBanner("Relay A", 220), highlights: ["价格划算", "稳定接入", "高缓存命中"] },
  { id: "demo-b", type: "sponsor", title: "示例聚合平台 B", description: "一个 Key 访问多家模型，兼容 Claude Code 等工具。", url: "https://example.com/b", image: demoBanner("Hub B", 280), highlights: ["多模型", "兼容 Claude Code"] },
  { id: "demo-c", type: "sponsor", title: "示例服务 C", description: "开放、稳定、高性价比的模型中转。", url: "https://example.com/c", image: demoBanner("Service C", 20), highlights: ["按量计费"] },
  { id: "demo-d", type: "normal", title: "示例教程", description: "如何把多个 Agent 接到同一个供应商池。", url: "https://example.com/d", highlights: [] },
];

// MCP 演示数据：Claude Code 与 Grok 各自已有的服务器（Codex 一列来自设置里的公共配置）。
// Skills 页演示数据：内置、已安装（启用到不同 Agent）、仓库里未安装的各几条。
const demoSkill = (id: string, repoKey: string, description: string, extra: Record<string, unknown> = {}) =>
  ({ id, name: id, description, repoKey, installed: false, enabled: false, bundled: false, updateAvailable: false, agents: [], managed: true, ...extra });
const demoSkills = {
  status: "ok", message: "",
  skills: [
    demoSkill("imagegen", "", "Generate or edit raster images when the task benefits from AI-created bitmap visuals.", { installed: true, enabled: true, bundled: true, agents: ["codex"], managed: false }),
    demoSkill("skill-creator", "", "Create or update a Codex skill with appropriately scoped instructions.", { installed: true, enabled: true, bundled: true, agents: ["codex"], managed: false }),
    demoSkill("pdf", "anthropics/skills@main", "Read, merge, split and fill PDF files.", { installed: true, enabled: true, agents: ["codex", "claude"] }),
    demoSkill("frontend-design", "anthropics/skills@main", "Build distinctive, production-grade frontend interfaces.", { installed: true, enabled: false, agents: ["claude"], updateAvailable: true }),
    demoSkill("algorithmic-art", "anthropics/skills@main", "Creating algorithmic art using p5.js with seeded randomness."),
    demoSkill("academy-guide", "anthropics/skills@main", "Check this skill before answering questions about courses."),
  ],
  repos: [{ owner: "anthropics", name: "skills", branch: "main", subdir: "", enabled: true }],
  repositorySkills: [], repoCounts: { "anthropics/skills@main": 20 }, repoErrors: [],
  skillsDir: "~/.codex-plus/skills", codexSkillsDir: "~/.codex/skills",
  agentSkillsDirs: { codex: "~/.codex/skills", claude: "~/.claude/skills", grok: "~/.grok/skills" },
};

// 插件市场演示数据：几条长短不一的说明，看卡片排版。
const demoMarketPlugin = (id: string, description: string, extra: Record<string, unknown> = {}) =>
  ({ id, name: id, displayName: id, description, version: "1.0.0", author: "demo", tags: [], license: "MIT", path: "", tree: "", bytes: 64_000, skills: 1, requiresAuth: false, installed: false, updateAvailable: false, ...extra });
const demoPluginMarket = {
  status: "ok", message: "", source: "public", repository: "demo/plugins", total: 6, cached: true,
  plugins: [
    demoMarketPlugin("01 Superdesign", "Superdesign adds a design skill to ChatGPT for web UI, presentations, and graphics.", { version: "0.6.0", bytes: 230_900 }),
    demoMarketPlugin("12ui Design", "Create distinctive interfaces with 12ui design skills and hosted design APIs.", { version: "0.2.65", bytes: 108_400 }),
    demoMarketPlugin("1Password", "Use the local 1Password MCP server to manage 1Password Developer Environments and local .env mounts from Codex.", { version: "0.2.0", bytes: 42_500, installed: true }),
    demoMarketPlugin("6X6", "Essential answers first, with focused detail on request.", { version: "1.0.2", bytes: 64_700 }),
    demoMarketPlugin("Accurate Translation", "Translate text and files locally without metered translation API tokens.", { requiresAuth: true }),
    demoMarketPlugin("Actively", "Win more deals with Actively by directly accessing your always-on personal sales assistant."),
  ],
};

const demoAgentMcp: Record<string, unknown> = {
  claude: { agent: "claude", configPath: "~/.claude.json", exists: true, entries: [
    { id: "context7", enabled: true, tomlBody: "command = \"npx\"\nargs = [\"-y\", \"@upstash/context7-mcp\"]\n" },
    { id: "github", enabled: true, tomlBody: "url = \"https://api.githubcopilot.com/mcp/\"\n" },
  ] },
  grok: { agent: "grok", configPath: "~/.grok/config.toml", exists: true, entries: [
    { id: "context7", enabled: true, tomlBody: "command = \"npx\"\nargs = [\"-y\", \"@upstash/context7-mcp\"]\n" },
  ] },
};

const settingsResult = () => ok({ revision: "web-preview", settings: demoSettings, settings_path: "演示模式", user_scripts: { enabled: true, skip_next_start: false, scripts: [] } });

export async function invoke<T = unknown>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  switch (command) {
    case "list_tools": return tools as T;
    case "load_gateway": return gateway as T;
    case "scan_agent_cache": return ok({ report: agentCacheReport }) as T;
    case "fetch_provider_models": return ok({ models: ["claude-opus-4-5", "claude-sonnet-4-5", "claude-haiku-4-5"], endpoint: "https://api.anthropic.com/v1/models" }) as T;
    case "agent_pet_catalog": return agentPetCatalog.map(([id, name]) => ({ id, name })) as T;
    // 演示模式没有内嵌音频，返回空表；页面照常显示，试听静音。
    case "agent_pet_sounds": return {} as T;
    case "agent_pet_image": return agentPetPreviewImage(String(args.image ?? ""), String(args.customImageUrl ?? "")) as T;
    case "lookup_model_prices": return ok({ source: "models.dev", fetchedAtMs: Date.now(), prices: ((args.models as string[]) ?? []).map((model) => ({ model, price: { providerId: "anthropic", providerName: "Anthropic", modelId: model, currency: "USD", input: 3, cachedInput: 0.3, output: 15, context: 200000 } })) }) as T;
    case "list_installed_skills":
    case "refresh_skill_catalog": return demoSkills as T;
    case "refresh_plugin_market": return demoPluginMarket as T;
    case "load_agent_mcp":
    case "save_agent_mcp": return ok({ config: demoAgentMcp[String(args.agent)] }) as T;
    case "load_gateway_usage": return gatewayUsage as T;
    // Agent 概览仪表板的演示数据
    case "list_agent_sessions": return ok({ sessions: args.agent === "claude" ? [
      { agent: "claude", id: "s1", title: "重新设计当前程序的 UI", cwd: "/Users/demo/Desktop/CodexPlusPlus", updatedAtMs: Date.now() - 60_000 },
      { agent: "claude", id: "s2", title: "审查 GitHub 全部 issue 和 PR", cwd: "/Users/demo/Desktop/CodexPlusPlus", updatedAtMs: Date.now() - 20_000 },
      { agent: "claude", id: "s3", title: "帮我清理一点电脑垃圾", cwd: "/Users/demo", updatedAtMs: Date.now() - 57 * 60_000 },
    ] : [] }) as T;
    case "load_request_logs": return ok({ entries: [
      { id: "r1", agent: "claude", timestampMs: Date.now() - 30_000, startedAtMs: 0, durationMs: 1800, success: true, model: "claude-sonnet-4-5", upstreamModel: "deepseek-v4-pro", relayName: "DeepSeek", state: null },
      { id: "r2", agent: "claude", timestampMs: Date.now() - 90_000, startedAtMs: 0, durationMs: 3100, success: true, model: "claude-opus-4-5", upstreamModel: "kimi-k2", relayName: "Kimi", state: null },
      { id: "r3", agent: "claude", timestampMs: Date.now() - 240_000, startedAtMs: 0, durationMs: 900, success: false, model: "claude-haiku-4-5", upstreamModel: "glm-4.6", relayName: "智谱 GLM", state: null },
    ] }) as T;
    case "startup_options": return ok({ showUpdate: false }) as T;
    case "consume_pending_manager_navigation": return null as T;
    case "load_overview": return overview as T;
    case "load_ads": return ok({ version: 1, ads: demoAds, topAds: [] }) as T;
    case "load_settings":
    case "save_settings":
    case "reset_settings":
    case "set_settings_path":
    case "reset_settings_path":
    case "refresh_user_script_inventory":
      if (command === "save_settings" && args.settings) return ok({ revision: "web-preview", settings: args.settings, settings_path: "演示模式", user_scripts: { enabled: true, skip_next_start: false, scripts: [] } }) as T;
      return settingsResult() as T;
    case "relay_status": return ok({ authenticated: true, authSource: "演示", accountLabel: "Web preview", configPath: "演示模式", configured: true, requiresOpenaiAuth: false, hasBearerToken: true, backupPath: null }) as T;
    case "read_relay_files": return ok({ configPath: "演示模式", authPath: "演示模式", configContents: "[model_providers.custom]", authContents: "{}" }) as T;
    case "check_env_conflicts": return ok({ conflicts: [] }) as T;
    case "load_provider_sync_targets": return ok({ currentProvider: "deepseek", targets: [] }) as T;
    case "load_pending_provider_import": return ok({ pending: null }) as T;
    // 皮肤管理：预览里没有本机主题、市场与社区，返回空列表，页面能正常打开。
    case "list_dream_skin_themes": return ok({ themes: [
      { key: "builtin:dream-skin", id: "dream-skin", name: "Dream Skin", previewPath: "", kind: "builtin", builtin: true, active: false, modified: false },
      { key: "active", id: "dream-skin", name: "Dream Skin", previewPath: "", kind: "activeUnsaved", builtin: false, active: true, modified: true },
      { key: "saved:aurora", id: "aurora", name: "Aurora", previewPath: "", kind: "stored", builtin: false, active: false, modified: false },
    ] }) as T;
    case "refresh_dream_skin_market": return ok({ schemaVersion: 1, updatedAt: "", repositoryUrl: "", cached: true, warning: "", themes: [] }) as T;
    case "refresh_dream_skin_community": return ok({ items: [], total: 0, fetchedAt: "", cached: true }) as T;
    case "load_pending_dream_skin_community": return ok({ versionId: "" }) as T;
    case "load_watcher_state": return ok({ enabled: false, running: false, status: "disabled", message: "演示模式" }) as T;
    case "load_auto_takeover_state": return ok({ running: false, enabled: false, message: "演示模式" }) as T;
    case "update_check": return ok({ currentVersion: "2.0.0", latestVersion: "2.0.0", updateAvailable: false }) as T;
    case "refresh_script_market": return ok({ market: { status: "ok", message: "演示模式", updatedAt: "2026-10-26T12:51:00Z", scripts: [
      { id: "relay-balance", name: "Codex Relay Balance", description: "通用中转站余额与模型用量监控，支持自动读取或手动填写接口、日期范围、Token 明细和实际扣费倍率。", version: "0.3.7", author: "Codex++ Community", tags: ["codex", "relay", "balance", "usage", "quota"], homepage: "https://github.com/demo/relay-balance", script_url: "", installed: false, installedVersion: "", updateAvailable: false },
      { id: "model-matrix", name: "Codex Model Matrix", description: "紧凑且适配主题的 Codex 模型选择器，配备可拖动吸附的推理强度滑块。", version: "1.0.0", author: "lx", tags: ["codex", "models", "reasoning"], homepage: "https://github.com/demo/model-matrix", script_url: "", installed: true, installedVersion: "1.0.0", updateAvailable: false },
      { id: "context-meter", name: "Codex Context Used Meter", description: "在 Codex App 对话界面显示当前会话上下文余量。", version: "101", author: "Minghou-Lei", tags: ["context", "tokens"], homepage: "", script_url: "", installed: false, installedVersion: "", updateAvailable: false },
    ] }, user_scripts: { enabled: true, skip_next_start: false, scripts: [] } }) as T;
    case "load_ccs_providers": return ok({ providers: [] }) as T;
    case "weixin_connect_status": return ok({ state: "disconnected", message: "演示模式", accountId: "", hasToken: false, lastPeerId: "", lastMessageAtMs: 0, processedMessages: 0 }) as T;
    case "load_local_sessions":
    case "list_local_sessions": return ok({ dbPath: "演示模式", dbPaths: [], sessions: [], offset: 0, limit: 50, hasMore: false, totalCount: 0 }) as T;
    case "load_relay_environment":
    case "check_relay_environment": return ok({ clashVergeTun: { enabled: false, configPath: null }, proxyEnvironment: { variables: [] }, codexEnvFile: { exists: false, path: "" } }) as T;
    default: return ok() as T;
  }
}

export function convertFileSrc(path: string): string {
  return path;
}
