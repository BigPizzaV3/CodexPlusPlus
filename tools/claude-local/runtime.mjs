import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const MAX_STATE_BYTES = 32 * 1024 * 1024;
const MAX_TEXT = 1024 * 1024;
const idPattern = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;
const isRecord = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
const isText = (value, max = MAX_TEXT) => typeof value === "string" && value.length <= max;
const isId = (value) => typeof value === "string" && idPattern.test(value);
const isDate = (value) => typeof value === "string" && value.length <= 64 && Number.isFinite(Date.parse(value));

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
  return {
    managed, gateway, model, smallModel, key,
    models: [...new Set([model, smallModel, ...models])],
    localization: flag("LOCALIZATION"),
    voiceInput: flag("VOICE_INPUT"),
    computerUse: flag("COMPUTER_USE"),
    backgroundMode: flag("BACKGROUND_MODE"),
  };
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

export function modelList(config) {
  return {
    data: config.models.map((id) => ({
      id, type: "model", display_name: id, created_at: "2026-01-01T00:00:00Z",
    })),
    has_more: false,
    first_id: config.models[0] ?? null,
    last_id: config.models.at(-1) ?? null,
  };
}

export function modelSelectors(config, selected = config.model) {
  const model = gatewayModel(config, selected);
  const thinking = { type: "effort", effort: "medium" };
  const models = config.models.map((id) => ({
    id, name: id, short_name: id, description: "", section: "main",
    disabled: false, disabled_reason: null, hard_limit: 200000,
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
