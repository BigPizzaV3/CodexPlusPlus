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

// 预览语言：`?lang=en` 时演示数据里的中文名换成英文，官网英文页的截图与在线体验不夹中文。
const previewEnglish = (() => {
  try { return window.localStorage.getItem("codex-plus-lang") === "en"; } catch { return false; }
})();
const L = (zh: string, en: string) => (previewEnglish ? en : zh);

const demoSettings = {
  claudeAppExtensions: false,
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
  codexAppSessionDeleteSkipConfirm: false,
  codexAppMarkdownExport: true,
  codexAppUsageStatusLine: true,
  codexAppProxyRequestStatus: true,
  codexAppPasteFix: false,
  codexAppForceChineseLocale: false,
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
    name: L("默认中转", "Default relay"),
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
    stripImageGenerationTool: false,
    anthropicAuthHeader: "x-api-key",
    anthropicMaxTokens: 0,
    anthropicPromptCache: true,
    anthropicImpersonateClaudeCode: false,
    anthropicThinkingMode: "",
  }, {
    id: "sub2api-demo",
    name: L("sub2api 中转（演示）", "sub2api relay (demo)"),
    model: "deepseek-v4-pro[1M]",
    baseUrl: "https://sub2api.example.com/v1",
    upstreamBaseUrl: "https://sub2api.example.com/v1",
    apiKey: "sk-demo",
    protocol: "responses",
    relayMode: "mixedApi",
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
    stripImageGenerationTool: false,
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
  codexAppWhaleBalanceProtocol: "auto",
  codexAppWhaleBalancePath: "",
  codexAppWhaleBalanceField: "",
  codexAppWhaleBalanceCurrency: "USD",
  codexAppWhaleBalanceScale: 1,
  agentPet: {
    enabled: false,
    kind: "image",
    agents: ["codex"],
    image: "whale",
    customImageUrl: "",
    size: 112,
    soundEnabled: true,
    soundSet: "duck",
    customSoundPress: "",
    customSoundRelease: "",
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
// 二次元图库是 Commons 直链，预览直接引用。
const agentPetAnimeCatalog: Array<[string, string, string, string]> = [
  ["chibi-bow", "蝴蝶结", "https://upload.wikimedia.org/wikipedia/commons/c/c1/Chibiw.png", "asrafil · CC0"],
  ["chibi-girl", "Q版少女", "https://upload.wikimedia.org/wikipedia/commons/4/4c/Cute_girl.png", "dcatcherex · CC0"],
  ["anime-girl", "马尾少女", "https://upload.wikimedia.org/wikipedia/commons/e/e7/Anime_girl_publicdomainq.png", "oksmith · CC0"],
  ["anime-angry", "炸毛少女", "https://upload.wikimedia.org/wikipedia/commons/c/c5/Angry_anime_girl.png", "snoopingasusual · CC0"],
];

function agentPetPreviewImage(image: string, customImageUrl: string) {
  if (image === "custom") {
    return /^https:\/\//.test(customImageUrl)
      ? { status: "ok", image: customImageUrl }
      : { status: "failed", message: "自定义图片只支持 https:// 地址" };
  }
  const anime = agentPetAnimeCatalog.find(([id]) => id === image);
  if (anime) return { status: "ok", image: anime[2] };
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
  current_version: "2.3.1",
  update_status: "up_to_date",
  settings_path: "演示模式",
  logs_path: "演示模式",
});

const tools = ok({
  tools: [
    { id: "codex", name: "Codex", homeDir: "~/.codex", switchable: true, active: true, activeRelayName: L("默认中转", "Default relay"), relayCount: 3 },
    { id: "grok", name: "Grok", homeDir: "~/.grok", switchable: false, active: false, activeRelayName: null, relayCount: 0 },
    { id: "claude", name: "Claude Code", homeDir: "~/.claude", switchable: true, active: false, activeRelayName: null, relayCount: 0 },
  ],
  activeTool: "codex",
});

// 网关演示数据：Codex 与 Claude Code 已接入、Grok 未接入；池里有订阅账号、官方 API 和中转站，
// 一家有 Key 在冷却，另有两个自建组与两个自动组，看总览连线、供应商池和路由页的完整样子。
const gatewayBinding = (model: string, extra: Partial<Record<string, unknown>> = {}) => ({
  enabled: true,
  model,
  extraModels: [],
  claudeRoles: { opus: "", sonnet: "", haiku: "", subagent: "" },
  unknownModel: "mapToDefault",
  claudeEnableEffort: false,
  token: "demo",
  ...extra,
});
const price = (input: number, cachedInput: number, output: number, currency = "CNY") => ({ currency, input, cachedInput, output });
const demoKeys = (...masked: string[]) => masked.map((value, index) => ({ id: `k${index + 1}`, masked: value }));
const minutesLater = (minutes: number) => Date.now() + minutes * 60_000;
const gateway = ok({
  gateway: {
    version: 1,
    bindings: {
      codex: gatewayBinding("group/coding"),
      claude: gatewayBinding("deepseek-v4-pro", { claudeRoles: { opus: "chatgpt-pro/gpt-5.5", sonnet: "deepseek-v4-pro", haiku: "deepseek-v4-flash", subagent: "" } }),
    },
    keyPolicy: { mode: "sticky" },
    restoreOnExit: true,
    autoModels: true,
    favoriteModels: ["gpt-5.5", "deepseek-v4-pro"],
  },
  port: 63130,
  helperRunning: true,
  pool: [
    { id: "chatgpt-pro", name: L("ChatGPT 订阅", "ChatGPT subscription"), baseUrl: "https://chatgpt.com/backend-api/codex", protocol: "responses", official: true, account: "chatgpt", plan: "ChatGPT Pro", models: ["gpt-5.5", "gpt-5.4-mini"], imageModels: ["gpt-image-2"], keyCount: 0, quota: { windows: [{ windowMinutes: 300, usedPercent: 23, resetsAtMs: minutesLater(170) }, { windowMinutes: 10080, usedPercent: 41, resetsAtMs: minutesLater(3 * 1440) }], updatedAtMs: Date.now() - 60_000 } },
    { id: "deepseek", name: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", protocol: "chatCompletions", official: true, models: ["deepseek-v4-pro", "deepseek-v4-flash"], keyCount: 3, keys: demoKeys("sk-7f2a…e91c", "sk-03bd…4a77", "sk-c5e0…1b2f"), balanceApi: true, modelPrices: { "deepseek-v4-pro": price(4, 0.4, 16), "deepseek-v4-flash": price(1, 0.1, 2) }, modelWindows: { "deepseek-v4-pro": 1_000_000 } },
    { id: "kimi", name: "Kimi", baseUrl: "https://api.moonshot.cn/v1", protocol: "chatCompletions", official: true, models: ["kimi-k2-turbo"], keyCount: 2, keys: demoKeys("sk-kM9q…20xe", "sk-kQ1v…77ad"), balanceApi: true, price: price(4, 1, 16) },
    { id: "zhipu", name: L("智谱 GLM", "Zhipu GLM"), baseUrl: "https://open.bigmodel.cn/api/paas/v4", protocol: "chatCompletions", official: true, models: ["glm-5"], keyCount: 1, keys: demoKeys("4e1f…a9d3"), price: price(4, 0.8, 16) },
    { id: "sub2api-pool", name: L("sub2api 站点（演示）", "sub2api relay (demo)"), baseUrl: "https://sub2api.example.com", protocol: "responses", official: false, models: ["gpt-5.5", "deepseek-v4-pro"], keyCount: 1, keys: demoKeys("sk-s2a…8c1d") },
    { id: "newapi-pool", name: L("new-api 站点（演示）", "new-api relay (demo)"), baseUrl: "https://newapi.example.com/v1", protocol: "chatCompletions", official: false, models: ["claude-sonnet-5-5", "kimi-k2-turbo"], keyCount: 1, keys: demoKeys("sk-na…3f0e") },
  ],
  groups: [
    { id: "coding", name: L("主力编程", "Main coding"), mode: "inOrder", sticky: "cache", members: [{ model: "chatgpt-pro/gpt-5.5", weight: 1 }, { model: "sub2api-pool/gpt-5.5", weight: 1 }, { model: "deepseek/deepseek-v4-pro", weight: 1 }] },
    { id: "budget", name: L("便宜轮换", "Budget rotation"), mode: "rotate", sticky: "session", members: [{ model: "deepseek/deepseek-v4-flash", weight: 2 }, { model: "zhipu/glm-5", weight: 1 }, { model: "kimi/kimi-k2-turbo", weight: 1 }] },
  ],
  autoGroups: [
    { id: "auto:gpt-5.5", name: "gpt-5.5", mode: "usage", sticky: "cache", members: [{ model: "chatgpt-pro/gpt-5.5", weight: 1 }, { model: "sub2api-pool/gpt-5.5", weight: 1 }], hidden: false, customized: true },
    { id: "auto:deepseek-v4-pro", name: "deepseek-v4-pro", mode: "inOrder", sticky: "cache", members: [{ model: "deepseek/deepseek-v4-pro", weight: 1 }, { model: "sub2api-pool/deepseek-v4-pro", weight: 1 }], hidden: false, customized: false },
    { id: "auto:kimi-k2-turbo", name: "kimi-k2-turbo", mode: "inOrder", sticky: "cache", members: [{ model: "kimi/kimi-k2-turbo", weight: 1 }, { model: "newapi-pool/kimi-k2-turbo", weight: 1 }], hidden: false, customized: false },
  ],
  agents: [
    { agent: "codex", name: "Codex", version: "26.1029", enabled: true, baseUrl: "http://127.0.0.1:63130/codex", defaultModel: "group/coding", models: [] },
    { agent: "claude", name: "Claude Code", version: "2.3.1", enabled: true, baseUrl: "http://127.0.0.1:63130/claude", defaultModel: "deepseek-v4-pro", models: [] },
    { agent: "grok", name: "Grok", enabled: false, baseUrl: "http://127.0.0.1:63130/grok", defaultModel: "", models: [] },
  ],
  health: { keys: [{ relayId: "zhipu", keyId: "k1", state: "cooling", remainingMs: 42000 }], providers: [] },
  speeds: [
    { relayId: "chatgpt-pro", model: "gpt-5.5", ms: 820, samples: 40 },
    { relayId: "sub2api-pool", model: "gpt-5.5", ms: 1240, samples: 12 },
    { relayId: "deepseek", model: "deepseek-v4-pro", ms: 1650, samples: 88 },
    { relayId: "kimi", model: "kimi-k2-turbo", ms: 1100, samples: 30 },
  ],
  activity: {
    agents: [{
      agent: "codex",
      requests: 512,
      rerouted: 9,
      errors: 2,
      inFlight: ["gpt-5.5"],
      last: {
        requestedModel: "group/coding",
        model: "gpt-5.5",
        groupId: "coding",
        planned: [{ relayId: "chatgpt-pro", relayName: "ChatGPT Pro", model: "gpt-5.5" }],
        steps: [{ relayId: "chatgpt-pro", relayName: "ChatGPT Pro", keyId: "account", model: "gpt-5.5", outcome: "answered", elapsedMs: 4200 }],
        result: "answered",
        finishedAtMs: Date.now() - 4000,
        elapsedMs: 4200,
      },
    }, {
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
  imageOptions: [{ id: "chatgpt-pro/gpt-image-2", providerId: "chatgpt-pro", providerName: "ChatGPT Pro", model: "gpt-image-2", vendorPicks: true }],
  imageAuto: { id: "chatgpt-pro/gpt-image-2", providerId: "chatgpt-pro", providerName: "ChatGPT Pro", model: "gpt-image-2", vendorPicks: true },
  // 导入弹窗演示数据：带品牌图标的、只显示首字母的都有，看头像是否保持固定尺寸。
  codexProviders: [
    { id: "hj", name: "哈基米 国模", protocol: "responses", baseUrl: "https://api.biuapi.com" },
    { id: "kimi", name: "Kimi 官方", protocol: "responses", baseUrl: "https://api.moonshot.cn" },
    { id: "deepseek", name: "DeepSeek", protocol: "responses", baseUrl: "https://api.deepseek.com" },
    // 和演示设置里的直连供应商同 id：「Codex 连接」页据此显示「加入供应商池」。
    { id: "default", name: "默认中转", protocol: "responses", baseUrl: "https://api.deepseek.com/v1" },
    { id: "sub2api-demo", name: "sub2api 中转（演示）", protocol: "responses", baseUrl: "https://sub2api.example.com/v1" },
  ],
  claudeProviders: [{ id: "official", name: "Claude Official" }],
  grokProfiles: [],
});
const usageCounts = (calls: number, errors = 0) => ({ calls, errors, inputTokens: calls * 9000, outputTokens: calls * 700, cachedTokens: calls * 5600, reasoningTokens: calls * 180 });
const gatewayUsage = ok({
  totals: usageCounts(908, 8),
  // 近 7 天，用来看用量图表的悬停效果
  days: [320, 610, 240, 760, 450, 130, 908].map((calls, index) => ({ date: new Date(Date.now() - (6 - index) * 86_400_000).toISOString().slice(0, 10), ...usageCounts(calls, index === 3 ? 4 : 0) })),
  byAgent: [{ key: "codex", label: "codex", ...usageCounts(512, 3) }, { key: "claude", label: "claude", ...usageCounts(396, 5) }],
  byProvider: [
    { key: "chatgpt-pro", label: "ChatGPT Pro", ...usageCounts(402, 1) },
    { key: "deepseek", label: "DeepSeek", ...usageCounts(281, 2), costs: [{ currency: "CNY", amount: 21.6 }] },
    { key: "kimi", label: "Kimi", ...usageCounts(115, 3), costs: [{ currency: "CNY", amount: 8.9 }] },
    { key: "sub2api-pool", label: L("sub2api 站点（演示）", "sub2api relay (demo)"), ...usageCounts(110, 2) },
  ],
  byModel: [
    { key: "gpt-5.5", label: "gpt-5.5", ...usageCounts(512, 3) },
    { key: "deepseek-v4-pro", label: "deepseek-v4-pro", ...usageCounts(238, 2), costs: [{ currency: "CNY", amount: 19.4 }] },
    { key: "kimi-k2-turbo", label: "kimi-k2-turbo", ...usageCounts(115, 3), costs: [{ currency: "CNY", amount: 8.9 }] },
    { key: "deepseek-v4-flash", label: "deepseek-v4-flash", ...usageCounts(43, 0), costs: [{ currency: "CNY", amount: 2.2 }] },
  ],
  costs: [{ currency: "CNY", amount: 30.5 }],
});

// 请求日志演示数据：两个 Agent、成功 / 自动重试 / 失败 / 进行中各有，token、缓存与费用都填上。
const demoRequest = (index: number, minutesAgo: number, fields: Record<string, unknown>) => ({
  id: `r${index}`,
  timestampMs: Date.now() - minutesAgo * 60_000,
  startedAtMs: Date.now() - minutesAgo * 60_000,
  method: "POST",
  statusCode: 200,
  success: true,
  stream: true,
  attempts: 1,
  autoRetries: 0,
  state: null,
  ...fields,
});
const codexRequest = { agent: "codex", path: "/v1/responses", wireApi: "responses" };
const claudeRequest = { agent: "claude", path: "/v1/messages", wireApi: "chat" };
const demoRequestLogs = () => [
  demoRequest(1, 0, { ...codexRequest, state: "running", model: "gpt-5.5", upstreamModel: "gpt-5.5", relayId: "chatgpt-pro", relayName: "ChatGPT Pro", durationMs: 0, firstTokenMs: 760, estimatedOutputTokens: 412, threadTitle: L("重构网关的路由面板", "Refactor the gateway routing panel") }),
  demoRequest(2, 0.5, { ...codexRequest, model: "gpt-5.5", upstreamModel: "gpt-5.5", relayId: "chatgpt-pro", relayName: "ChatGPT Pro", durationMs: 18_400, firstTokenMs: 820, usage: { inputTokens: 86_420, outputTokens: 2_310, cachedTokens: 84_992, reasoningTokens: 640 }, cacheHitRatio: 0.983, threadTitle: L("重构网关的路由面板", "Refactor the gateway routing panel") }),
  demoRequest(3, 1, { ...claudeRequest, model: "claude-sonnet-4-5", upstreamModel: "deepseek-v4-pro", relayId: "deepseek", relayName: "DeepSeek", durationMs: 6_200, firstTokenMs: 1_480, usage: { inputTokens: 52_130, outputTokens: 1_204, cachedTokens: 49_920 }, cacheHitRatio: 0.958, cost: { currency: "CNY", amount: 0.043 } }),
  demoRequest(4, 2, { ...codexRequest, model: "gpt-5.5", upstreamModel: "gpt-5.5", relayId: "sub2api-pool", relayName: L("sub2api 站点（演示）", "sub2api relay (demo)"), durationMs: 21_900, firstTokenMs: 3_900, attempts: 2, autoRetries: 1, retryReasons: ["429 Too Many Requests"], attemptDetails: [{ attempt: 1, code: "429", message: "rate limited" }], usage: { inputTokens: 84_000, outputTokens: 1_880, cachedTokens: 81_408 }, cacheHitRatio: 0.969, threadTitle: L("重构网关的路由面板", "Refactor the gateway routing panel") }),
  demoRequest(5, 4, { ...claudeRequest, model: "claude-opus-4-5", upstreamModel: "kimi-k2-turbo", relayId: "kimi", relayName: "Kimi", durationMs: 3_100, firstTokenMs: 940, usage: { inputTokens: 31_800, outputTokens: 690, cachedTokens: 28_672 }, cacheHitRatio: 0.902, cost: { currency: "CNY", amount: 0.031 } }),
  demoRequest(6, 6, { ...claudeRequest, model: "claude-haiku-4-5", upstreamModel: "glm-5", relayId: "zhipu", relayName: L("智谱 GLM", "Zhipu GLM"), durationMs: 900, statusCode: 429, success: false, error: L("上游限流（429），Key 冷却 42 秒，已换下一家", "Upstream rate limited (429); key cooling for 42s, moved to the next provider") }),
  demoRequest(7, 9, { ...codexRequest, model: "deepseek-v4-flash", upstreamModel: "deepseek-v4-flash", relayId: "deepseek", relayName: "DeepSeek", durationMs: 2_400, firstTokenMs: 610, usage: { inputTokens: 12_400, outputTokens: 380, cachedTokens: 11_264 }, cacheHitRatio: 0.908, cost: { currency: "CNY", amount: 0.003 }, threadTitle: L("给会话起名", "Name the conversation") }),
  demoRequest(8, 14, { ...claudeRequest, model: "claude-sonnet-4-5", upstreamModel: "deepseek-v4-pro", relayId: "deepseek", relayName: "DeepSeek", durationMs: 9_800, firstTokenMs: 1_620, usage: { inputTokens: 48_900, outputTokens: 2_040, cachedTokens: 46_080 }, cacheHitRatio: 0.942, cost: { currency: "CNY", amount: 0.052 } }),
  demoRequest(9, 21, { ...codexRequest, model: "gpt-5.5", upstreamModel: "gpt-5.5", relayId: "chatgpt-pro", relayName: "ChatGPT Pro", durationMs: 14_200, firstTokenMs: 790, usage: { inputTokens: 71_300, outputTokens: 1_560, cachedTokens: 70_144, reasoningTokens: 420 }, cacheHitRatio: 0.984, threadTitle: L("修复托盘余额显示", "Fix tray balance display") }),
  demoRequest(10, 33, { ...claudeRequest, model: "claude-sonnet-4-5", upstreamModel: "claude-sonnet-5-5", relayId: "newapi-pool", relayName: L("new-api 站点（演示）", "new-api relay (demo)"), durationMs: 7_300, firstTokenMs: 1_210, usage: { inputTokens: 39_400, outputTokens: 1_120, cachedTokens: 36_864 }, cacheHitRatio: 0.936 }),
];

// 实时请求流演示（托盘「流量」页签、总览拓扑图）：在演示日志之上每隔一两秒「发出」一条请求，
// 几秒后答完（偶尔失败），光点据此来回走。
const demoFlowTemplates = [
  { ...codexRequest, model: "gpt-5.5", upstreamModel: "gpt-5.5", relayId: "chatgpt-pro", relayName: "ChatGPT Pro" },
  { ...claudeRequest, model: "claude-sonnet-4-5", upstreamModel: "deepseek-v4-pro", relayId: "deepseek", relayName: "DeepSeek" },
  { ...claudeRequest, model: "claude-opus-4-5", upstreamModel: "kimi-k2-turbo", relayId: "kimi", relayName: "Kimi" },
  { ...codexRequest, model: "gpt-5.5", upstreamModel: "gpt-5.5", relayId: "sub2api-pool", relayName: L("sub2api 站点（演示）", "sub2api relay (demo)") },
];
const demoFlow = { live: [] as Array<Record<string, unknown> & { finishAt: number }>, nextAt: 0, seq: 0 };
const flowPreviewStartedAt = Date.now();
const demoRequestFlow = (limit: number) => {
  const now = Date.now();
  // 确定性预览：空列表 → 未选路由 → 等待 → 响应 → 结束，覆盖真实日志的分阶段更新。
  const scenario = new URLSearchParams(window.location.search).get("flowPreview");
  if (scenario) {
    const params = new URLSearchParams(window.location.search);
    const startedAtMs = Number(params.get("flowStart")) || flowPreviewStartedAt;
    const finished = scenario === "done" || scenario === "error";
    const generating = scenario === "generating" || scenario === "mixed";
    const history = [
      { ...demoRequest(9997, 5, demoFlowTemplates[0]), id: "flow-history-ok", durationMs: 3600, usage: { inputTokens: 25000, outputTokens: 244 }, state: "success", success: true },
      { ...demoRequest(9998, 6, { ...demoFlowTemplates[0], relayId: "jojo", relayName: "jojocodemax" }),
        id: "flow-history-error", durationMs: 1200, state: "failed", success: false, statusCode: 429, error: "模拟历史请求：上游限流（429）" },
    ];
    if (scenario === "idle") return history;
    const active = { ...demoRequest(9999, 0, demoFlowTemplates[0]), id: "flow-preview", startedAtMs,
      state: finished ? (scenario === "error" ? "failed" : "success") : "running",
      phase: generating ? "generating" : "waiting", firstTokenMs: generating || finished ? 1200 : null,
      estimatedOutputTokens: generating ? Math.floor((now - startedAtMs) / 90) : null,
      usage: finished && scenario !== "error" ? { inputTokens: 25000, outputTokens: 244 } : null,
      durationMs: finished ? 8000 : 0, timestampMs: finished ? startedAtMs + 8000 : now,
      relayId: scenario === "unrouted" ? "" : "chatgpt-pro", relayName: scenario === "unrouted" ? "" : "ChatGPT Pro",
      error: scenario === "error" ? "模拟请求：上游返回 HTTP 500" : null,
      success: scenario === "done", statusCode: scenario === "error" ? 500 : 200 };
    const concurrent = scenario === "mixed" ? [{ ...active, id: "flow-preview-parallel", phase: "waiting",
      firstTokenMs: null, estimatedOutputTokens: null, relayId: "jojo", relayName: "jojocodemax" }] : [];
    return [active, ...concurrent, ...history].slice(0, limit);
  }
  if (now >= demoFlow.nextAt) {
    const template = demoFlowTemplates[demoFlow.seq % demoFlowTemplates.length];
    demoFlow.seq += 1;
    demoFlow.live.unshift({ ...demoRequest(1000 + demoFlow.seq, 0, template), id: `flow-${demoFlow.seq}`, state: "running", success: false, durationMs: 0, finishAt: now + 1500 + Math.random() * 3500 });
    demoFlow.nextAt = now + 1200 + Math.random() * 2400;
  }
  for (const entry of demoFlow.live) {
    if (entry.state !== "running" || now < entry.finishAt) continue;
    const failed = demoFlow.seq % 7 === 0;
    const durationMs = now - (entry.startedAtMs as number);
    const outputTokens = Math.round(durationMs * (0.2 + Math.random() * 0.4));
    Object.assign(entry, {
      state: failed ? "failed" : "success",
      success: !failed,
      statusCode: failed ? 429 : 200,
      error: failed ? L("上游限流（429）", "Upstream rate limited (429)") : null,
      timestampMs: now,
      durationMs,
      firstTokenMs: 600,
      usage: failed ? null : { inputTokens: 40_000 + Math.round(Math.random() * 160_000), outputTokens },
    });
  }
  demoFlow.live = demoFlow.live.slice(0, 40);
  const base = demoRequestLogs().filter((entry) => entry.state !== "running");
  return [...demoFlow.live, ...base].slice(0, limit);
};

// 会话页演示数据：Codex 的本地会话与 Claude Code / Grok 的会话放在一起。
const demoCodexSessions = () => [
  { id: "c1", title: L("重构网关的路由面板", "Refactor the gateway routing panel"), cwd: "/Users/demo/Projects/codex-plus", modelProvider: "codex_plus_gateway", archived: false, updatedAtMs: Date.now() - 30_000, rolloutPath: "", dbPath: "" },
  { id: "c2", title: L("修复托盘余额显示", "Fix tray balance display"), cwd: "/Users/demo/Projects/codex-plus", modelProvider: "codex_plus_gateway", archived: false, updatedAtMs: Date.now() - 21 * 60_000, rolloutPath: "", dbPath: "" },
  { id: "c3", title: L("写一份发布说明", "Draft the release notes"), cwd: "/Users/demo/Projects/website", modelProvider: "codex_plus_gateway", archived: false, updatedAtMs: Date.now() - 3 * 3_600_000, rolloutPath: "", dbPath: "" },
  { id: "c4", title: L("把 CSV 导入脚本改成流式读取", "Stream the CSV import script"), cwd: "/Users/demo/Projects/data-tools", modelProvider: "deepseek", archived: false, updatedAtMs: Date.now() - 26 * 3_600_000, rolloutPath: "", dbPath: "" },
];

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

// 推荐内容演示数据：中性名称，横幅用内联 SVG（尺寸和真实广告图一样大），不请求外部图片。
const demoBanner = (label: string, hue: number) =>
  `data:image/svg+xml;utf8,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="640"><defs><linearGradient id="g" x1="0" x2="1"><stop offset="0" stop-color="hsl(${hue} 60% 18%)"/><stop offset="1" stop-color="hsl(${hue + 40} 60% 30%)"/></linearGradient></defs><rect width="1600" height="640" fill="url(#g)"/><text x="400" y="360" font-family="Helvetica" font-size="120" font-weight="700" fill="white">${label}</text></svg>`)}`;
const demoAds = [
  { id: "demo-a", type: "sponsor", title: "示例中转站 A", description: "稳定的 API 中转服务，支持主流模型与图像能力，适合日常开发和团队协作。", url: "https://example.com/a", image: demoBanner("Relay A", 220), highlights: ["价格划算", "稳定接入", "高缓存命中"] },
  { id: "demo-b", type: "sponsor", title: "示例聚合平台 B", description: "一个 Key 访问多家模型，兼容 Claude Code 等工具。", url: "https://example.com/b", image: demoBanner("Hub B", 280), highlights: ["多模型", "兼容 Claude Code"] },
  { id: "demo-c", type: "sponsor", title: "示例服务 C", description: "开放、稳定、高性价比的模型中转。", url: "https://example.com/c", image: demoBanner("Service C", 20), highlights: ["按量计费"] },
  { id: "demo-e", type: "sponsor", title: "示例云 E", description: "150+ 主流模型，企业级 MaaS，按需选用，适合团队统一采购和管理。这一句故意写得很长，用来检查一行说明超出宽度时会不会把页面撑宽。", url: "https://example.com/e", image: demoBanner("Cloud E", 200), highlights: ["150+ 模型", "企业级"] },
  { id: "demo-f", type: "sponsor", title: "示例站 F", description: "兼容 OpenAI / Anthropic 协议。", url: "https://example.com/f", highlights: ["支持开票"] },
  { id: "demo-g", type: "sponsor", title: "示例平台 G", description: "国内直连，低延迟。", url: "https://example.com/g", image: demoBanner("G", 120), highlights: [] },
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

// Computer Use 各客户端的接入状态。
const demoComputerUseTarget = (id: string, label: string, configPath: string, registered: boolean) =>
  ({ id, label, configPath, registered, state: registered ? "registered" : "missing", message: "" });
const demoComputerUseTargets = () => ({
  claudeCode: demoComputerUseTarget("claudeCode", "Claude Code", "~/.claude.json", true),
  claudeDesktop: demoComputerUseTarget("claudeDesktop", "Claude Desktop", "~/Library/Application Support/Claude/claude_desktop_config.json", false),
  dsh: demoComputerUseTarget("dsh", "DeepSeek Harness", "~/.dsh/config.toml", true),
  zcode: demoComputerUseTarget("zcode", "ZCode", "~/.zcode/mcp.json", false),
  workBuddy: demoComputerUseTarget("workBuddy", "WorkBuddy AI", "~/.workbuddy/mcp.json", false),
  devin: demoComputerUseTarget("devin", "Devin", "~/.config/devin/mcp_config.json", false),
});
const demoClaudeEnhance = {
  supported: true,
  sourceApp: "/Applications/Claude.app",
  copyApp: "~/Applications/Claude (Codex++).app",
  sourceVersion: "1.3.12",
  copyVersion: "1.3.12",
  prepared: true,
  needsRebuild: false,
  running: false,
  nodeAvailable: true,
  pythonAvailable: true,
  shellLocale: "zh-CN",
  catalogEntries: 2864,
  voiceInput: false,
  localization: true,
  extensions: false,
  computerUse: true,
  computerUseSupported: true,
  bashTimeout: { enabled: false, defaultEnabled: false, defaultSeconds: 120, maxEnabled: false, maxSeconds: 600 },
};

// 本地拓展演示数据：一个单文件脚本、一个已加载的拓展包、一个清单有误的拓展包。
const demoBuiltins = [
  ["agent-pet", "Agent 桌宠", true, null], ["dictation", "语音听写", false, null],
  ["turn-rewind", "轮次回退", true, "user:pomodoro_timer"],
  ["side-chat-save", "侧聊保存", true, null], ["history-turn-recovery", "历史轮次补全", true, null],
  ["capacity-auto-retry", "容量不足自动重试", true, null],
  ["model-picker-groups", "模型分组", true, null],
  ["dream-skin", "梦皮主题", true, null], ["plugin-market-unlock", "插件市场解锁", true, null],
  ["instance-identity", "多开身份提示", true, null], ["session-title-hook", "会话标题中间件", true, null],
].map(([id, name, enabled, replacedBy]) => ({ id, key: `builtin:${id}`, name, description: "", enabled, replacedBy }));
const demoUserScripts = { enabled: true, claude_enabled: false, skip_next_start: false, builtins: demoBuiltins, scripts: [
  { key: "user:copy-title.js", name: "copy-title.js", source: "user", kind: "script", package: null, enabled: true, status: "loaded", error: "" },
  { key: "user:pomodoro_timer", name: "番茄钟", source: "user", kind: "package", enabled: true, status: "loaded", error: "",
    package: { id: "pomodoro_timer", version: "0.1.0", description: "在左侧图标栏提供可持久化的 25 分钟番茄钟", engine: 1, routes: [],
      hookEvents: ["Stop"], tools: ["remaining"],
      configSchema: [
        { key: "minutes", type: "number", title: "专注时长（分钟）", description: "每个番茄钟的长度", default: 25, options: [], secret: false },
        { key: "sound", type: "boolean", title: "结束时提示音", description: "", default: true, options: [], secret: false },
        { key: "theme", type: "select", title: "配色", description: "", default: "red", options: [{ value: "red", label: "番茄红" }, { value: "green", label: "薄荷绿" }], secret: false },
        { key: "note", type: "text", title: { zh: "结束提示语", en: "Finish message" }, description: "", default: "休息一下\n喝口水", options: [], secret: false },
        { key: "tags", type: "array", title: "标签", description: "", default: ["工作"], options: [], secret: false,
          items: { key: "", type: "string", title: "", description: "", default: "", options: [], secret: false } },
        { key: "webhook", type: "object", title: { zh: "结束时通知", en: "Webhook" }, description: "番茄钟结束时发一个请求", default: { url: "", retry: 1 }, options: [], secret: false,
          properties: [
            { key: "url", type: "string", title: "地址", description: "", default: "", options: [], secret: false },
            { key: "retry", type: "number", title: "重试次数", description: "", default: 1, options: [], secret: false, min: 0, max: 5 },
          ] },
      ],
      config: { minutes: 25, sound: true, theme: "red", note: "休息一下\n喝口水", tags: ["工作"], webhook: { url: "", retry: 1 } } } },
  { key: "user:future-ext", name: "未来拓展", source: "user", kind: "package", enabled: true, status: "failed",
    error: "需要拓展 API v2，当前 Codex++ 提供 v1，请升级 Codex++", package: { id: "future-ext", version: "1.0.0", description: "", engine: 2, routes: [] } },
  { key: "user:claude-extension-demo", name: "Claude 连通测试", source: "user", kind: "package",
    enabled: false, claude_enabled: true, status: "disabled", error: "",
    package: { id: "claude-extension-demo", version: "0.1.0", description: "按钮、后台工具与会话事件", engine: 1, routes: [], backend: true,
      claudeDesktop: { buttons: [{ id: "status", label: "Status", method: "status" }] } } },
] };
const settingsResult = () => ok({ revision: "web-preview", settings: demoSettings, settings_path: "演示模式", user_scripts: demoUserScripts });

export async function invoke<T = unknown>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  switch (command) {
    case "list_tools": return tools as T;
    case "load_gateway": return gateway as T;
    case "start_gateway_subscription_login": return ok({ start: { message: "", state: {} } }) as T;
    case "scan_agent_cache": return ok({ report: agentCacheReport }) as T;
    case "fetch_provider_models": return ok({ models: ["claude-opus-4-5", "claude-sonnet-4-5", "claude-haiku-4-5"], endpoint: "https://api.anthropic.com/v1/models" }) as T;
    case "agent_pet_catalog": return [
      ...agentPetCatalog.map(([id, name]) => ({ id, name })),
      ...agentPetAnimeCatalog.map(([id, name, , credit]) => ({ id, name, credit })),
    ] as T;
    // 演示模式没有内嵌音频，返回空表；页面照常显示，试听静音。
    case "agent_pet_sounds": return {} as T;
    case "agent_pet_sound_catalog": return [
      { id: "nya", name: "猫娘「にゃ」", author: "Mike_bes", license: "CC0", sourceUrl: "https://freesound.org/people/Mike_bes/sounds/336012/" },
      { id: "sparkle", name: "魔法闪光", author: "Hemplock / MLaudio", license: "CC0", sourceUrl: "https://freesound.org/people/Hemplock/sounds/618961/" },
    ] as T;
    case "agent_pet_sound": return { status: "failed", message: "网页预览不提供音效下载" } as T;
    case "agent_pet_import_sound": return { status: "ok", file: `${String(args.slot ?? "press")}-0123456789abcdef.mp3` } as T;
    // 网页预览不内嵌大肥鱼素材，页面会显示占位。
    case "agent_pet_whale_image": return "" as T;
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
      { agent: "claude", id: "s1", title: L("重新设计当前程序的 UI", "Redesign the app UI"), cwd: "/Users/demo/Projects/codex-plus", updatedAtMs: Date.now() - 60_000 },
      { agent: "claude", id: "s2", title: L("审查 GitHub 全部 issue 和 PR", "Review all GitHub issues and PRs"), cwd: "/Users/demo/Projects/codex-plus", updatedAtMs: Date.now() - 20_000 },
      { agent: "claude", id: "s3", title: L("帮我清理一点电脑垃圾", "Clean up some disk space"), cwd: "/Users/demo", updatedAtMs: Date.now() - 57 * 60_000 },
    ] : args.agent === "grok" ? [
      { agent: "grok", id: "g1", title: L("解释这段正则", "Explain this regex"), cwd: "/Users/demo/Projects/data-tools", updatedAtMs: Date.now() - 5 * 3_600_000 },
    ] : [] }) as T;
    case "load_request_logs": return ok({ entries: demoRequestLogs() }) as T;
    // 「版本与更新」演示：有新版本，后台下载 1 秒后完成（只下载不安装）。
    case "check_update": if (!new URLSearchParams(window.location.search).has("showUpdate")) return ok({ currentVersion: "2.3.1", latestVersion: "2.3.1", releaseSummary: "", assetName: null, assetUrl: null, updateAvailable: false, progress: 0 }) as T;
      return ok({ currentVersion: "2.3.1", latestVersion: "2.3.1", releaseSummary: "### 新增\n- 界面整体重做", assetName: "Codex++-2.3.0.dmg", assetUrl: "https://example.test/Codex++-2.3.0.dmg", updateAvailable: true, downloaded: false, progress: 0 }) as T;
    case "download_update": await new Promise((resolve) => setTimeout(resolve, 1000)); return ok({ downloaded: true, latestVersion: "2.3.1" }) as T;
    // 「日志与诊断」演示：几条诊断日志（含出错和重试）和一份诊断报告。
    case "read_latest_logs": {
      const now = Date.now();
      const line = (offset: number, event: string, detail: unknown) => JSON.stringify({ timestamp_ms: now - offset, pid: 17040, event, detail });
      const text = [
        'ount":0,"models":[]}',
        line(3_600_000, "manager.start", { version: "2.3.1", platform: "macos" }),
        line(1_800_000, "gateway.helper_started", { port: 63130, agents: 2 }),
        line(900_000, "manager.save_settings", { activeRelayId: "codex-plus-gateway", relayProfiles: [{ id: "default", modelCount: 2 }, { id: "sub2api-demo", modelCount: 3 }] }),
        line(600_000, "gateway.retry", { provider: "zhipu", status: 429, attempt: 2 }),
        line(300_000, "manager.navigation_failed", { error: "route not found: legacy", route: "legacy" }),
        line(60_000, "codex.launch", { status: "ok", pid: 31877, cdp: 9229 }),
      ].join("\n");
      return ok({ path: "~/Library/Application Support/Codex++/codex-plus.log", text, lines: 7, truncated: true, fileSize: 2_400_000 }) as T;
    }
    case "copy_diagnostics": return ok({ report: JSON.stringify({ generatedAtMs: Date.now(), version: "2.3.1", overview: { codex_app: { status: "found", path: "/Applications/ChatGPT.app" }, codex_version: "26.1027.61615", silent_shortcut: { status: "missing", path: "/Applications/Codex++.app" }, management_shortcut: { status: "missing", path: "/Applications/Codex++.app" }, latest_launch: { status: "stopped", phase: "user_stopped" } } }, null, 2) }) as T;
    case "load_request_flow": return ok({ entries: demoRequestFlow(Number(args.limit) || 30) }) as T;
    case "startup_options": return ok({ showUpdate: false }) as T;
    case "consume_pending_manager_navigation": return null as T;
    case "load_overview": return overview as T;
    // 置顶赞助位默认空；?showAds 放两条，?showAds=N 放 N 条（看首页横幅几个时怎么排）。
    case "load_ads": {
      const param = new URLSearchParams(location.search).get("showAds");
      const count = param === null ? 0 : Number(param) > 1 ? Number(param) : 2;
      return ok({ version: 1, ads: demoAds, topAds: demoAds.filter((ad) => ad.type === "sponsor").slice(0, count) }) as T;
    }
    case "load_settings":
    case "save_settings":
    case "reset_settings":
    case "set_settings_path":
    case "reset_settings_path":
    case "refresh_user_script_inventory":
      if (command === "save_settings" && args.settings) {
        Object.assign(demoSettings, args.settings);
        tools.activeTool = demoSettings.activeTool;
      }
      return settingsResult() as T;
    case "set_user_scripts_enabled":
      if (args.host === "claude") {
        demoUserScripts.claude_enabled = Boolean(args.enabled);
        demoSettings.claudeAppExtensions = Boolean(args.enabled);
        demoClaudeEnhance.extensions = Boolean(args.enabled);
      } else {
        demoUserScripts.enabled = Boolean(args.enabled);
      }
      return settingsResult() as T;
    case "set_user_script_enabled": {
      const script = demoUserScripts.scripts.find((item) => item.key === args.key);
      if (script) {
        if (args.host === "claude") script.claude_enabled = Boolean(args.enabled);
        else script.enabled = Boolean(args.enabled);
      }
      return settingsResult() as T;
    }
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
    case "update_check": return ok({ currentVersion: "2.3.1", latestVersion: "2.3.1", updateAvailable: false }) as T;
    case "refresh_script_market": return ok({ market: { status: "ok", message: "演示模式", updatedAt: "2026-10-26T12:51:00Z", scripts: [
      { id: "relay-balance", name: "Codex Relay Balance", description: "通用中转站余额与模型用量监控，支持自动读取或手动填写接口、日期范围、Token 明细和实际扣费倍率。", version: "0.3.7", author: "Codex++ Community", tags: ["codex", "relay", "balance", "usage", "quota"], homepage: "https://github.com/demo/relay-balance", script_url: "", installed: false, installedVersion: "", updateAvailable: false },
      { id: "model-matrix", name: "Codex Model Matrix", description: "紧凑且适配主题的 Codex 模型选择器，配备可拖动吸附的推理强度滑块。", version: "1.0.0", author: "lx", tags: ["codex", "models", "reasoning"], homepage: "https://github.com/demo/model-matrix", script_url: "", installed: true, installedVersion: "1.0.0", updateAvailable: false },
      { id: "context-meter", name: "Codex Context Used Meter", description: "在 Codex App 对话界面显示当前会话上下文余量。", version: "101", author: "Minghou-Lei", tags: ["context", "tokens"], homepage: "", script_url: "", installed: false, installedVersion: "", updateAvailable: false },
    ], packages: [
      { kind: "package", id: "session-search", name: L("会话搜索", "Session search"), description: L("在侧边栏搜索全部历史会话，回车跳到命中的那一轮。", "Search all past sessions from the sidebar and jump to the matching turn."), version: "1.0.0", author: "Codex++", tags: ["history"], homepage: "", icon: "", updatedAt: "2026-10-30", repo: "BigPizzaV3/codexpp-session-search", tag: "v1.0.0", sha: "9c1e0b7d3a5f6e8c9d0a1b2c3d4e5f6a7b8c4f2a", official: true, channel: "official", installed: false, installedVersion: "", installedSha: "", updateAvailable: false },
      { kind: "package", id: "word-guard", name: L("命令守卫", "Command guard"), description: L("拦截危险的 shell 命令，并给出更安全的替代写法。", "Block dangerous shell commands and suggest safer alternatives."), version: "1.2.0", author: "alice", tags: ["hooks"], homepage: "", icon: "", updatedAt: "2026-10-28", repo: "alice/codexpp-word-guard", tag: "v1.2.0", sha: "4f2a9c1e0b7d3a5f6e8c9d0a1b2c3d4e5f6a7b8c", official: false, channel: "curated", installed: true, installedVersion: "1.1.0", installedSha: "0b7d3a5f6e8c9d0a1b2c3d4e5f6a7b8c4f2a9c1e", updateAvailable: true },
    ], blockedInstalled: [] }, user_scripts: demoUserScripts }) as T;
    case "load_ccs_providers": return ok({ providers: [] }) as T;
    case "weixin_connect_status": return ok({ state: "disconnected", message: "演示模式", accountId: "", hasToken: false, lastPeerId: "", lastMessageAtMs: 0, processedMessages: 0 }) as T;
    case "load_local_sessions":
    case "list_local_sessions": return ok({ dbPath: "演示模式", dbPaths: [], sessions: demoCodexSessions(), offset: 0, limit: 50, hasMore: false, totalCount: 4 }) as T;
    // 多开演示数据：主 Codex 与两个供应商身份的实例都在运行。
    case "list_codex_multi_instances": return ok({ instances: [
      { instanceId: "main", label: L("主 Codex", "Main Codex"), isMain: true, pid: 31877, debugPort: 9229, status: "running", running: true, startedAtMs: Date.now() - 2 * 3_600_000 },
      { instanceId: "relay-default", profileId: "default", profileName: L("默认中转", "Default relay"), pid: 34301, debugPort: 9631, helperPort: 9632, status: "running", running: true, startedAtMs: Date.now() - 40 * 60_000 },
      { instanceId: "relay-sub2api", profileId: "sub2api-demo", profileName: L("sub2api 中转（演示）", "sub2api relay (demo)"), pid: 33911, debugPort: 9697, helperPort: 9698, status: "running", running: true, startedAtMs: Date.now() - 12 * 60_000 },
    ] }) as T;
    // Computer Use 演示数据：找到了 ChatGPT.app，Claude Code 与 DeepSeek Harness 已接入（Claude Desktop 用增强版自带的那份）。
    case "computer_use_status": return ok({ runtimeFound: true, appPath: "/Applications/ChatGPT.app", packageVersion: "1.2026.300", targets: demoComputerUseTargets() }) as T;
    case "computer_use_browser_extension_status": return { installed: true, storeUrl: "" } as T;
    case "computer_use_approval_get": return ok({ mode: "auto", cuMode: "js", pip: true, approvals: [], lastClient: { name: "claude-code", version: "2.3.1", at: Date.now() - 120_000 } }) as T;
    // Claude 增强演示数据：增强版已制作，开了界面汉化与 Computer Use。
    case "claude_localization_status": return ok(demoClaudeEnhance) as T;
    case "claude_localization_set_extensions": {
      demoClaudeEnhance.extensions = Boolean(args.enabled);
      demoSettings.claudeAppExtensions = Boolean(args.enabled);
      demoUserScripts.claude_enabled = Boolean(args.enabled);
      return ok(demoClaudeEnhance) as T;
    }
    case "load_relay_environment":
    case "check_relay_environment": return ok({ clashVergeTun: { enabled: false, configPath: null }, proxyEnvironment: { variables: [] }, codexEnvFile: { exists: false, path: "" } }) as T;
    // 中转站识别：演示供应商按地址返回 sub2api（倍率 + 钱包余额）或 new-api（分组倍率 + Key 额度），其余识别不出。
    case "probe_relay_billing": {
      const profile = (args as { profile?: { baseUrl?: string } } | undefined)?.profile;
      if (!profile?.baseUrl?.includes("sub2api")) return ok({ platform: null, rate: null, usage: null, detail: "演示模式" }) as T;
      return ok({
        platform: "sub2api",
        rate: { endpoint: "https://sub2api.example.com/v1/sub2api/billing", multiplier: 0.9, appliedPeakMultiplier: 1.5, group: null },
        usage: { endpoint: "https://sub2api.example.com/v1/usage", kind: "wallet", isValid: true, planName: "钱包余额", remaining: 12.345, quotaLimit: null, quotaUsed: null, unit: "USD", rateLimits: [] },
        detail: "",
      }) as T;
    }
    case "probe_gateway_provider_billing": {
      const id = (args as { id?: string }).id;
      if (id === "sub2api-pool") return ok({
        platform: "sub2api",
        rate: { endpoint: "https://sub2api.example.com/v1/sub2api/billing", multiplier: 0.28, appliedPeakMultiplier: null, group: null },
        usage: { endpoint: "https://sub2api.example.com/v1/usage", kind: "wallet", isValid: true, planName: "钱包余额", remaining: 97505834.07, quotaLimit: null, quotaUsed: null, unit: "USD", rateLimits: [] },
        detail: "",
      }) as T;
      if (id === "newapi-pool") return ok({
        platform: "newapi",
        rate: { endpoint: "https://newapi.example.com/api/log/token", multiplier: 1.5, appliedPeakMultiplier: null, group: "vip" },
        usage: { endpoint: "https://newapi.example.com/api/usage/token/", kind: "keyQuota", isValid: true, planName: "codex", remaining: 36.5, quotaLimit: 50, quotaUsed: 13.5, unit: "CNY", rateLimits: [] },
        detail: "",
      }) as T;
      return ok({ platform: null, rate: null, usage: null, detail: "演示模式" }) as T;
    }
    case "set_extension_config": return settingsResult() as T;
    case "reload_user_scripts": return settingsResult() as T;
    case "extension_creator_request": return {
      status: args.action === "connect" ? "connected" : args.action === "disconnect" ? "disconnected" : "not_connected",
      message: "", configPath: "~/.codex/config.toml", extensionsDirectory: "~/.config/Codex++/user_scripts",
    } as T;
    // 社区拓展演示数据：一个带钩子与后台的、一个只有页面脚本且已安装的、一个需要新版 Codex++ 的。
    case "extension_publish_request": {
      const report = { id: args.payload?.id, name: L("字数统计", "Word count"), version: args.action === "bump" ? "1.0.1" : "1.0.0", description: L("统计输入框字数", "Count characters"), errors: [], warnings: [L("没有 README，发布时会按清单生成一份", "No README; one will be generated from the manifest")], secrets: [], files: 4, bytes: 5120, hasReadme: false, hasLicense: false, publishedRepo: null, suggestedRepoName: `codexpp-${args.payload?.id}`, gh: { ready: true, account: "demo-user" } };
      if (args.action === "publish") return { ok: true, repo: `demo-user/codexpp-${args.payload?.id}`, url: "https://github.com/", version: "1.0.0", steps: ["发布前检查", "确认 GitHub 账号", "确认 git 身份", "创建仓库", "初始化工作副本", "整理文件", "提交", "打标签", "推送", "创建 release", "添加 topic"].map((step) => ({ step, ok: true, output: "" })) } as T;
      return report as T;
    }
    case "extension_community_request": {
      const perms = (extra: Record<string, unknown> = {}) => ({ backend: null, hooks: [], gatewayRequest: 0, gatewayResponse: 0, tools: [], routes: [], renderer: ["main.js"], ...extra });
      const entry = (repo: string, extra: Record<string, unknown>) => ({
        repo, url: `https://github.com/${repo}`, owner: repo.split("/")[0], ownerAvatar: "", description: "", stars: 0, license: "MIT",
        pushedAt: "2026-10-28T09:00:00Z", id: repo.split("/")[1], name: repo.split("/")[1], version: "1.0.0", author: "", homepage: "",
        icon: "", keywords: [], engine: 1, compatible: true, permissions: perms(), installed: null, ...extra,
      });
      if (args.action === "prepare" || args.action === "preparePackage") return {
        channel: args.action === "preparePackage" ? "curated" : "community",
        token: "demo", repo: args.payload?.repo ?? "alice/codexpp-word-guard", id: "word-guard", name: L("命令守卫", "Command guard"), version: "1.2.0", tag: "v1.2.0",
        sha: "4f2a9c1e0b7d3a5f6e8c9d0a1b2c3d4e5f6a7b8c", fromDefaultBranch: false, fromAsset: true,
        permissions: perms({ backend: "backend/main.mjs", hooks: [{ event: "PreToolUse", matcher: "^Bash$" }], gatewayRequest: 1, tools: ["guard_status"], routes: ["/settings/get"] }),
        conflict: null,
      } as T;
      if (args.action === "install") return { hooks: true, hooksAdded: true } as T;
      if (args.action === "discard") return {} as T;
      return {
        enabled: true, fetchedAt: Math.floor(Date.now() / 1000) - 180, stale: false, problems: [], message: "",
        entries: [
          entry("alice/word-guard", { name: L("命令守卫", "Command guard"), stars: 128, version: "1.3.0", description: L("拦下危险的 shell 命令，并在后台记录审计日志。", "Blocks dangerous shell commands and keeps an audit log."), permissions: perms({ backend: "backend/main.mjs", hooks: [{ event: "PreToolUse", matcher: "^Bash$" }], gatewayRequest: 1, tools: ["guard_status"] }) }),
          entry("bob/word-count", { name: L("字数统计", "Word count"), stars: 37, description: L("在输入框旁显示字数。", "Shows a word count next to the composer."), installed: { version: "1.0.0", tag: "v1.0.0", sha: "1111111", fromDefaultBranch: false } }),
          entry("carol/future-panel", { name: L("实验面板", "Experimental panel"), stars: 5, engine: 2, compatible: false, description: L("需要下一版拓展 API。", "Needs the next extension API.") }),
        ],
      } as T;
    }
    // dsh 插件演示数据：一个按教程写的组合包（工具 + skill + MCP 行 + 不兼容行）。
    case "dsh_plugins_request": if (args.action === "browse_index") return {
      entries: [
        { name: "@deepseek-ai/dsh-persona", title: L("人设", "Persona"), description: L("给模型加一段人设提示词", "Adds a persona prompt"), version: "0.2.0", source: "@deepseek-ai/dsh-persona@0.2.0", note: null },
        { name: "dsh-git-demo", title: "Git demo", description: L("从 git 仓库导入", "Imported from git"), version: "1.0.0", source: "https://github.com/x/y.git#abc123&path:packages/demo", note: null },
      ],
    } as T; else return {
      message: "",
      plugins: [{
        id: "dsh-greet-bundle", name: "dsh-greet-bundle", version: "0.1.0",
        source: { type: "npm", spec: "dsh-greet-bundle" }, root: "", managed: true, enabled: true,
        disabledRows: [], configs: { greet: { greeting: "Hi there" } }, importedSkills: ["greet-skill"], toolCount: 1,
        mcpJson: JSON.stringify({ mcpServers: { demo: { command: "npx", args: ["-y", "@modelcontextprotocol/server-everything"] } } }, null, 2),
        companion: { id: "dsh-dsh-greet-bundle", hooks: ["PreToolUse", "Stop"], instructions: true, staticPrompt: true, installed: true, enabled: true, scriptsEnabled: true },
        report: {
          kind: "bundle", title: L("问候工具", "Greeting tools"), description: L("给 dsh 用的问候工具。", "Greeting tools for dsh."), client: false, diagnostics: [],
          skills: [{ name: "greet-skill", description: "How to greet people politely." }],
          rows: [
            { id: "greet", module: "dsh-greet-bundle", status: "compatible", kind: "plugin", reasons: [], notes: [], mappings: ["instructions", "PreToolUse", "Stop"], tools: [{ name: "greet", description: "Greet someone by name." }],
              configSchema: { type: "object", dict: { greeting: { type: "string", meta: { default: "Hello", description: L("问候语", "Greeting") } }, punctuation: { type: "union", list: [{ type: "const", value: "!" }, { type: "const", value: "." }], meta: { default: "!" } }, times: { type: "number", meta: { default: 1, min: 1, max: 5 } } } } },
            { id: "session-tool", module: "./session.js", status: "incompatible", kind: "plugin", reasons: [L("依赖 Codex++ 不提供的服务：sessions（属于 dsh 的 agent 循环 / 会话 / 模型等运行时）", "Needs services Codex++ does not provide: sessions")], notes: [], tools: [] },
            { id: "demo-mcp", module: "@deepseek-ai/dsh-mcp-client", status: "compatible", kind: "mcp", reasons: [], notes: [L("MCP 配置可导入 Codex；reconnect、toolCallTimeoutMs 等 dsh 专属字段会被丢弃", "The MCP config can be imported into Codex; dsh-only fields are dropped")], tools: [] },
          ],
        },
      }],
      integration: { status: "registered", registered: true, message: "" },
      toolchain: { node: "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node", npm: "/usr/local/bin/npm", git: "/usr/bin/git" },
      pluginDir: "",
    } as T;
    // Agent 协作演示数据（settings 组追加）：协作页没有快照时整页只剩报错。
    case "agent_collaboration_request": return (args.action === "mcp_config"
      ? { codexToml: "[mcp_servers.codex-plus-agents]\ncommand = \"codex-plus\"", claudeJson: "{\n  \"mcpServers\": {}\n}", command: "codex-plus", args: ["agents-mcp"] }
      : {
        config: { enabled: true, defaultAgentId: "deepseek", maxRuntimeMinutes: 10, agents: [
          { id: "codex", name: "Codex", executable: "", enabled: false, model: "", reuseSavedAuth: false, maxConcurrentTasks: 1 },
          { id: "deepseek", name: "DeepSeek Harness", executable: "", enabled: true, model: "deepseek-account/deepseek-flash", reuseSavedAuth: true, maxConcurrentTasks: 1 },
          { id: "claude", name: "Claude Code", executable: "/usr/local/bin/claude", enabled: true, model: "", reuseSavedAuth: false, maxConcurrentTasks: 2 },
        ] },
        tasks: [
          { id: "task-1", agentId: "claude", prompt: "审核这次改动，重点看并发与错误处理", cwd: "/Users/demo/project", status: "completed", answer: "未发现阻塞问题；建议给重试加上上限。", error: "", sessionId: "s-1", createdAtMs: Date.now() - 3_600_000, updatedAtMs: Date.now() - 3_300_000, cancelRequested: false, progress: "" },
        ],
        availability: [
          { id: "codex", available: true, path: "/Applications/Codex.app/Contents/Resources/codex" },
          { id: "deepseek", available: true, path: "/Users/demo/.dsh/bin/dsh" },
          { id: "claude", available: true, path: "/usr/local/bin/claude" },
        ],
        integration: { status: "connected", message: "已登记到 ~/.codex/config.toml", home: "~/.codex" },
        capabilities: { maxQueuedPerAgent: 8, maxConcurrentTasksLimit: 4 },
      }) as T;
    default: return ok() as T;
  }
}

export function convertFileSrc(path: string): string {
  return path;
}
