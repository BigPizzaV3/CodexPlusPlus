import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const MAX_STATE_BYTES = 32 * 1024 * 1024;
const MAX_TEXT = 1024 * 1024;
// 没声明窗口的模型按 Claude 自己的假定值走；声明了就一律以声明为准。
const DEFAULT_MODEL_WINDOW = 200_000;
const idPattern = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;
const isRecord = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
const isText = (value, max = MAX_TEXT) => typeof value === "string" && value.length <= max;
const isId = (value) => typeof value === "string" && idPattern.test(value);
const isDate = (value) => typeof value === "string" && value.length <= 64 && Number.isFinite(Date.parse(value));

// Claude 前端只认 free / pro / max 和 max 的 5x/20x，没有 Ultra 档：
// Ultra 按最高档 max_20x 发，只是显示名不同。
export const PLANS = {
  free:  { plan: "free", label: "Free",  capabilities: [],                           rateLimitTier: "default_claude_ai",      analytics: "claude_free" },
  pro:   { plan: "pro",  label: "Pro",   capabilities: ["claude_pro"],               rateLimitTier: "default_claude_ai",      analytics: "claude_pro" },
  max:   { plan: "max",  label: "Max",   capabilities: ["claude_pro", "claude_max"], rateLimitTier: "default_claude_max_5x",  analytics: "claude_max" },
  ultra: { plan: "max",  label: "Ultra", capabilities: ["claude_pro", "claude_max"], rateLimitTier: "default_claude_max_20x", analytics: "claude_max" },
};

export function planProfile(plan) {
  return Object.hasOwn(PLANS, plan) ? PLANS[plan] : PLANS.ultra;
}

export function runtimeConfig(env = process.env) {
  const managed = env.CLAUDE_LOCAL_MANAGED === "1";
  const flag = (name) => env[`CLAUDE_LOCAL_${name}`] !== "0";
  if (managed && ["URL", "KEY", "MODEL"].some((name) => !env[`CLAUDE_LOCAL_GATEWAY_${name}`])) {
    throw new Error("Managed Claude requires a configured local gateway and model");
  }
  const gatewayUrl = env.CLAUDE_LOCAL_GATEWAY_URL || "http://127.0.0.1:3425";
  const gateway = new URL(gatewayUrl);
  if (!["http:", "https:"].includes(gateway.protocol) || gateway.username || gateway.password || gateway.search || gateway.hash) {
    throw new Error("Invalid Claude local gateway address");
  }
  const model = env.CLAUDE_LOCAL_GATEWAY_MODEL || "claude-sonnet-4-6";
  const smallModel = env.CLAUDE_LOCAL_GATEWAY_SMALL_MODEL || model;
  const key = env.CLAUDE_LOCAL_GATEWAY_KEY || "magpie";
  if (!isText(model, 512) || !model || !isText(smallModel, 512) || !smallModel ||
    !isText(key, 8192) || /[\r\n]/.test(key)) {
    throw new Error("Invalid Claude local gateway configuration");
  }
  let models = managed ? [model] : ["claude-sonnet-4-6", "claude-opus-4-6"];
  if (env.CLAUDE_LOCAL_GATEWAY_MODELS) {
    try {
      models = JSON.parse(env.CLAUDE_LOCAL_GATEWAY_MODELS);
    } catch {
      throw new Error("Invalid Claude local gateway model list");
    }
    if (!Array.isArray(models) || models.length > 4096 || models.some((id) => !isText(id, 512) || !id || /[\r\n]/.test(id))) {
      throw new Error("Invalid Claude local gateway model list");
    }
  }
  let modelWindows = {};
  if (env.CLAUDE_LOCAL_GATEWAY_MODEL_WINDOWS) {
    try {
      modelWindows = JSON.parse(env.CLAUDE_LOCAL_GATEWAY_MODEL_WINDOWS);
    } catch {
      throw new Error("Invalid Claude local gateway model windows");
    }
    if (!isRecord(modelWindows) || Object.entries(modelWindows).length > 4096 ||
      Object.entries(modelWindows).some(([id, window]) =>
        !isText(id, 512) || !id || !Number.isSafeInteger(window) || window <= 0)) {
      throw new Error("Invalid Claude local gateway model windows");
    }
  }
  // 选择器显示名（id → {name, description}）：坏数据当没给，界面回退 id。
  const modelNames = (() => {
    try {
      const value = JSON.parse(env.CLAUDE_LOCAL_GATEWAY_MODEL_NAMES || "{}");
      if (!isRecord(value)) return {};
      return Object.fromEntries(Object.entries(value).filter(([id, info]) =>
        isText(id, 512) && id && isRecord(info) && isText(info.name, 512) && (info.description === undefined || isText(info.description, 512))));
    } catch { return {}; }
  })();
  // 选中模型没有逐模型窗口时用这个值兜底（网关按 binding 解析出的窗口）。
  // 写坏了就当没给，不让一次启动失败在建环境变量上。
  const defaultWindow = (() => {
    const raw = env.CLAUDE_LOCAL_GATEWAY_DEFAULT_WINDOW;
    if (raw === undefined || raw === "") return DEFAULT_MODEL_WINDOW;
    const value = Number(raw);
    return Number.isSafeInteger(value) && value > 0 ? value : DEFAULT_MODEL_WINDOW;
  })();
  // 完整功能模式的「PoC 数据路径」：CLAUDE_LOCAL_RECORDED_DIR 指向本机一份
  // 录制数据（.cache/ 静态资产 + mock/bootstrap.json + mock/api/* + i18n/），
  // 命中时数据面按非托管 PoC 回放；目录缺失/不完整则保持合成数据。
  const recordedDir = (() => {
    if (!managed || !env.CLAUDE_LOCAL_RECORDED_DIR) return null;
    const resolved = path.resolve(env.CLAUDE_LOCAL_RECORDED_DIR);
    try {
      if (!fs.statSync(resolved).isDirectory()) return null;
      if (!fs.statSync(path.join(resolved, ".cache")).isDirectory()) return null;
      if (!fs.statSync(path.join(resolved, "mock", "bootstrap.json")).isFile()) return null;
    } catch { return null; }
    return resolved;
  })();
  return {
    managed, gateway, model, smallModel, key,
    models: [...new Set([model, smallModel, ...models])],
    modelWindows,
    modelNames,
    defaultWindow,
    recordedDir,
    localization: flag("LOCALIZATION"),
    voiceInput: flag("VOICE_INPUT"),
    computerUse: flag("COMPUTER_USE"),
    backgroundMode: flag("BACKGROUND_MODE"),
    // 录制 bootstrap 里自带的原生模型（Opus / Sonnet / Haiku …）不对应任何供应商，
    // 选了也会被 gatewayModel 换成默认模型；默认从选择器里拿掉，只留供应商池里的。
    hideUnroutedModels: flag("HIDE_UNROUTED_MODELS"),
    displayName: displayName(env.CLAUDE_LOCAL_DISPLAY_NAME),
    email: email(env.CLAUDE_LOCAL_EMAIL),
    plan: Object.hasOwn(PLANS, env.CLAUDE_LOCAL_PLAN ?? "") ? env.CLAUDE_LOCAL_PLAN : "ultra",
  };
}

// 显示名：trim、去控制字符、最多 64 字符；空则按默认。
function displayName(value) {
  const name = String(value ?? "").trim().replace(/[\u0000-\u001F\u007F]/g, "").slice(0, 64).trim();
  return name || "Local User";
}

// 邮箱：trim 后须形如 a@b（最长 254、无空白与控制字符）；空或不合法按默认。
function email(value) {
  const text = String(value ?? "").trim();
  const at = text.lastIndexOf("@");
  const valid = text.length <= 254 && !/[\s\u0000-\u001F\u007F]/.test(text) && at > 0 && at < text.length - 1;
  return valid ? text : "local@claude.local";
}

export function gatewayTarget(config, requestPath) {
  const target = new URL(config.gateway);
  const requested = new URL(requestPath, "http://localhost");
  target.pathname = `${target.pathname.replace(/\/+$/, "")}/${requested.pathname.replace(/^\/+/, "")}`;
  target.search = requested.search;
  return target;
}

export function gatewayModel(config, requested) {
  if (config.models.includes(requested)) return requested;
  if (/^(?:claude-)?haiku(?:-|$)/i.test(requested || "")) return config.smallModel;
  if (config.managed || !requested || /^(?:claude-|sonnet|opus|haiku)/i.test(requested)) return config.model;
  return requested;
}

export function gatewayBody(config, raw) {
  const value = JSON.parse(raw.toString("utf8"));
  if (!isRecord(value)) throw new Error("Invalid model request");
  delete value.diagnostics;
  if (isRecord(value.metadata)) delete value.metadata.diagnostics;
  value.model = gatewayModel(config, value.model);
  return Buffer.from(JSON.stringify(value));
}

/**
 * 从录制 bootstrap 的各份模型清单里删掉供应商池之外的条目（就地修改）。
 *
 * 这些条目是 Claude 官方模型，池里没有对应的路由：选中后请求会被
 * `gatewayModel` 悄悄换成默认模型，名字与实际不符。只动选择器读的几份清单；
 * growthbook 不碰（池模型本来就不在那里，删空了前端怎么兜底说不准）。
 */
export function dropUnroutedModels(bootstrap, config) {
  if (!isRecord(bootstrap) || !config.models.length) return;
  const routable = new Set(config.models);
  const keep = (list, key) => Array.isArray(list)
    ? list.filter((entry) => !isRecord(entry) || routable.has(entry[key]))
    : list;
  const orgs = [
    bootstrap.activeOrganization,
    ...(Array.isArray(bootstrap.account?.memberships) ? bootstrap.account.memberships.map((m) => m?.organization) : []),
  ];
  for (const org of orgs) {
    if (isRecord(org) && Array.isArray(org.claude_ai_bootstrap_models_config)) {
      org.claude_ai_bootstrap_models_config = keep(org.claude_ai_bootstrap_models_config, "model");
    }
  }
  if (isRecord(bootstrap.claude_ai_available_models)) {
    bootstrap.claude_ai_available_models.models = keep(bootstrap.claude_ai_available_models.models, "model_id");
  }
  if (Array.isArray(bootstrap.model_selector_config)) {
    for (const selector of bootstrap.model_selector_config) {
      if (isRecord(selector)) selector.models = keep(selector.models, "id");
    }
  }
}

/** 选择器里的名称与副标题：有友好名用友好名（副标题是供应商），否则名称沿用模型 id、副标题为空。 */
export function modelLabel(config, id) {
  const entry = config.modelNames?.[id];
  return { name: entry?.name || id, description: entry?.description || "" };
}

export function modelList(config) {
  return {
    data: config.models.map((id) => ({
      id, type: "model", display_name: config.modelNames?.[id]?.name || id, created_at: "2026-01-01T00:00:00Z",
    })),
    has_more: false,
    first_id: config.models[0] ?? null,
    last_id: config.models.at(-1) ?? null,
  };
}

/**
 * 一个模型在界面上显示多长窗口。
 *
 * 逐模型声明最优先；键可能是 `provider/model`，也可能只是裸模型名（老配置和
 * 手写的 `modelWindows` 都是后者），所以两边都认。选中的那个模型还没声明时，
 * 用网关按 binding 解析出的 `defaultWindow` —— 它就是启动 Claude Code 时
 * 写进 `CLAUDE_CODE_MAX_CONTEXT_TOKENS` 的同一个数，界面和 CLI 不该各说各话。
 * 其余没声明的模型仍按 Claude 自己的假定值。
 */
export function modelWindow(config, model) {
  const windows = config.modelWindows ?? {};
  const id = typeof model === "string" ? model : "";
  if (Object.hasOwn(windows, id)) return windows[id];
  const folded = id.toLowerCase();
  const match = (key) => key.toLowerCase() === folded;
  const declared = Object.entries(windows).find(([key]) => match(key));
  if (declared !== undefined) return declared[1];
  const bareName = (value) => (value.includes("/") ? value.slice(value.lastIndexOf("/") + 1) : value);
  const wanted = bareName(id);
  if (wanted !== "") {
    const bare = Object.entries(windows).find(([key]) => bareName(key).toLowerCase() === wanted.toLowerCase());
    if (bare !== undefined) return bare[1];
  }
  if (id !== "" && id === (config.model ?? "")) return config.defaultWindow ?? DEFAULT_MODEL_WINDOW;
  return DEFAULT_MODEL_WINDOW;
}

export function modelSelectors(config, selected = config.model) {
  const model = gatewayModel(config, selected);
  const thinking = { type: "effort", effort: "medium" };
  // Code / Cowork 等页面的模型选择器读的是这里，不是 /v1/models：名称与副标题同样用友好名。
  const models = config.models.map((id) => ({
    id,
    name: modelLabel(config, id).name,
    short_name: modelLabel(config, id).name,
    description: modelLabel(config, id).description,
    section: "main",
    disabled: false, disabled_reason: null,
    hard_limit: modelWindow(config, id),
    context_window: modelWindow(config, id),
    runtime: { max_input_tokens: modelWindow(config, id) },
    capabilities: { compass: false, gsuite_tools: false, mm_images: false, mm_pdf: false, web_search: false },
    thinking: { type: "effort", effort_options: [{ id: "medium", name: "Medium" }] },
  }));
  const surfaces = ["chat", "code", "cc", "ccr", "ccd", "cowork", "design", "office_agent", "claude_science"];
  return {
    model_selector_state: surfaces.map((id) => ({
      id, model, thinking, thinking_by_model: config.models.map((id) => ({ id, thinking })),
      selection_source: "global_default",
    })),
    model_selector_config: surfaces.map((id) => ({
      id, models, description: "",
      settings_vocabulary: { create_mode: { default: 1 }, effort_level: { medium: 2 }, thinking_mode: { off: 3 } },
    })),
  };
}

function validateTurn(turn) {
  return isRecord(turn) &&
    ["humanId", "asstId", "groupId", "blockId"].every((key) => isId(turn[key])) &&
    (turn.humanParent === undefined || isId(turn.humanParent)) &&
    Number.isSafeInteger(turn.humanIndex) && turn.humanIndex >= 0 &&
    Number.isSafeInteger(turn.asstIndex) && turn.asstIndex >= 0 &&
    isText(turn.prompt) && isText(turn.text) &&
    typeof turn.complete === "boolean" &&
    (turn.model === undefined || isText(turn.model, 512)) &&
    (turn.stopReason === null || isText(turn.stopReason, 128)) &&
    isDate(turn.created);
}

function validateConversation(conv) {
  return isRecord(conv) && isId(conv.uuid) && isText(conv.name, 4096) &&
    (conv.model === undefined || isText(conv.model, 512)) &&
    isDate(conv.created_at) && isDate(conv.updated_at) &&
    Number.isSafeInteger(conv.nextIndex) && conv.nextIndex >= 0 &&
    (conv.leafMessageId === null || isId(conv.leafMessageId)) &&
    Array.isArray(conv.turns) && conv.turns.length <= 10000 && conv.turns.every(validateTurn) &&
    Array.isArray(conv.chat_messages) && conv.chat_messages.length <= 20000 &&
    conv.chat_messages.every((message) => isRecord(message) && isId(message.uuid) &&
      ["human", "assistant"].includes(message.sender) && isText(message.text) &&
      Number.isSafeInteger(message.index) && message.index >= 0 && isDate(message.created_at) &&
      (message.parent_message_uuid === null || isId(message.parent_message_uuid)));
}

function validateNotifications(entry) {
  if (!isRecord(entry) || !isId(entry.org) || !isRecord(entry.value)) return false;
  const state = entry.value;
  if (!isRecord(state.preferences) || !isRecord(state.preferences.feature_preference) ||
    !isRecord(state.effective_preferences) || !isRecord(state.push_reachability)) return false;
  const reachability = state.push_reachability;
  if (typeof reachability.has_active_channel !== "boolean" ||
    !Array.isArray(reachability.platforms) || reachability.platforms.length > 64 ||
    !reachability.platforms.every((platform) => isText(platform, 128)) ||
    !(reachability.most_recent_token_refresh === null || isDate(reachability.most_recent_token_refresh))) return false;
  return [state.preferences.feature_preference, state.effective_preferences].every((features) =>
    Object.entries(features).length <= 512 && Object.entries(features).every(([feature, channels]) =>
      isText(feature, 256) && isRecord(channels) && Object.entries(channels).every(([key, value]) =>
        ["enable_push", "enable_email"].includes(key) ? typeof value === "boolean" :
          ["push_source", "email_source"].includes(key) && isText(value, 64))));
}

export function createStateStore(file, conversations, notifications, warn = () => {}) {
  let timer;
  let disabled = false;
  if (file && fs.existsSync(file)) {
    try {
      if (fs.statSync(file).size > MAX_STATE_BYTES) throw new Error("State exceeds size limit");
      const state = JSON.parse(fs.readFileSync(file, "utf8"));
      if (!isRecord(state) || state.version !== 1 || !Array.isArray(state.conversations) ||
        state.conversations.length > 2000 || !state.conversations.every(validateConversation) ||
        !Array.isArray(state.notifications) || state.notifications.length > 2000 ||
        !state.notifications.every(validateNotifications)) throw new Error("Invalid state contract");
      for (const conv of state.conversations) {
        // 中断的请求不自动重发，避免重启后重复计费或执行。
        for (const turn of conv.turns) {
          if (!turn.complete) {
            turn.complete = true;
            turn.stopReason = "interrupted";
          }
          turn.replyStarted = false;
          turn.created = new Date(turn.created);
        }
        conversations.set(conv.uuid, conv);
      }
      for (const entry of state.notifications) notifications.set(entry.org, entry.value);
    } catch {
      disabled = true;
      warn("Local conversation state is invalid; the original file was preserved");
    }
  }
  const flush = () => {
    clearTimeout(timer);
    timer = undefined;
    if (!file || disabled) return;
    let temporary;
    try {
      const body = Buffer.from(JSON.stringify({
        version: 1,
        conversations: [...conversations.values()],
        notifications: [...notifications].map(([org, value]) => ({ org, value })),
      }));
      if (body.length > MAX_STATE_BYTES) throw new Error("State exceeds size limit");
      fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
      temporary = `${file}.${process.pid}.${crypto.randomUUID()}.tmp`;
      fs.writeFileSync(temporary, body, { mode: 0o600 });
      fs.renameSync(temporary, file);
      fs.chmodSync(file, 0o600);
    } catch {
      warn("Local conversation state could not be saved");
    }
  };
  return {
    flush,
    changed() {
      if (!file || disabled || timer) return;
      timer = setTimeout(flush, 250);
      timer.unref();
    },
  };
}
