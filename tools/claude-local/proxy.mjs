#!/usr/bin/env node
/**
 * Claude Desktop 本地兼容后端（PoC）
 *
 * 目标：让 Claude.app 认为自己在跟 claude.ai 说话，实际由我们接管：
 *   - /api/*         → 本地 mock（账号、会话、feature flags）
 *   - 其余静态资源   → 转发上游（真 UI），带磁盘缓存
 *   - demo 模式      → 连上游都不需要，直接回本地演示页（用于验证链路）
 *
 * 用法：
 *   node proxy.mjs --port 8899 --mode mock --upstream https://claude.ai
 *   node proxy.mjs --mode demo
 *
 * 客户端（Claude.app）需要把系统代理或 --proxy-server 指到 127.0.0.1:8899，
 * 并信任 scripts/gen-ca.sh 生成并安装的本地 CA。
 */
import http from "node:http";
import https from "node:https";
import tls from "node:tls";
import net from "node:net";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";
import { createConversationService } from "./conversation-service.mjs";
import { isPatchedStartupAsset, startupAssets } from "./scripts/patch-dictation-start.mjs";
import { createStateStore, dropUnroutedModels, gatewayBody, gatewayModel, gatewayTarget, modelLabel, modelList, modelSelectors, modelWindow, planProfile, runtimeConfig } from "./runtime.mjs";
import { createOfficialAssets } from "./official-assets.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const runtime = runtimeConfig();
const officialAssets = runtime.managed
  ? createOfficialAssets(process.env.CLAUDE_LOCAL_DESKTOP_RESOURCES)
  : null;
const LOCALIZATION_SHIM_PATH = "/__codex_plus/claude-localization.js";
const localizationShim = runtime.managed && !runtime.recordedDir && runtime.localization
  ? fs.readFileSync(path.join(here, "page-shim.js"))
  : null;

function addLocalizationShim(html, officialLocales) {
  if (!localizationShim) return html;
  const head = /<head\b[^>]*>/i.exec(html);
  const headEnd = /<\/head\s*>/i.exec(html);
  if (!head || !headEnd || head.index >= headEnd.index) {
    throw new Error("Claude Desktop official shell has no supported localization injection point");
  }
  const contentStart = head.index + head[0].length;
  const headContent = html.slice(contentStart, headEnd.index);
  const existingMeta = /<meta\b[^>]*\bname=(["'])i18n-catalogs\1[^>]*>/i.exec(headContent);
  const existingValue = existingMeta?.[0].match(/\bdata-i18n-catalogs=(["'])(.*?)\1/i)?.[2] ?? "";
  const catalogs = new Map();
  for (const entry of existingValue.split(",")) {
    const match = entry.trim().match(/^([\w-]+):([\w-]+)\.([\w-]+)$/);
    if (match) catalogs.set(match[1], `${match[2]}.${match[3]}`);
  }
  for (const locale of officialLocales) {
    if (!catalogs.has(locale)) catalogs.set(locale, "1.1.1");
  }
  catalogs.set("zh-Hans", catalogs.get("zh-Hans") ?? "1.1.1");
  const catalogValue = [...catalogs]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([locale, versions]) => `${locale}:${versions}`)
    .join(",");
  const meta = `<meta name="i18n-catalogs" data-i18n-catalogs="${catalogValue}">`;
  const withoutMeta = existingMeta
    ? headContent.replace(existingMeta[0], "")
    : headContent;
  const firstScript = /<script\b/i.exec(withoutMeta);
  const additions = `${meta}${localizationShim && !html.includes(`src="${LOCALIZATION_SHIM_PATH}"`)
    ? `<script src="${LOCALIZATION_SHIM_PATH}"></script>`
    : ""}`;
  const insertAt = firstScript ? firstScript.index : withoutMeta.length;
  return `${html.slice(0, contentStart)}${withoutMeta.slice(0, insertAt)}${additions}${withoutMeta.slice(insertAt)}${html.slice(headEnd.index)}`;
}

const officialShell = runtime.managed
  ? addLocalizationShim(officialAssets.indexHtml, officialAssets.i18nLocales)
  : null;
const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1] ?? fallback;
};

const PORT = Number(arg("port", "8899"));
const MODE = arg("mode", "mock"); // mock | passthrough | demo
const UPSTREAM = arg("upstream", "https://claude.ai");
const CA_DIR = arg("ca", path.join(here, "ca"));
const APP_STATE_DIR = process.env.CODEX_PLUS_STATE_DIR || path.join(os.homedir(), ".codex-session-delete");
const CACHE_DIR = process.env.CLAUDE_LOCAL_CACHE_DIR || path.join(here, ".cache");
// 完整功能模式的「PoC 数据路径」：CLAUDE_LOCAL_RECORDED_DIR 指向本机录制数据
// （.cache/ + mock/ + i18n/，不进包）。命中时数据面与 PoC 回放完全一致；
// 生命周期、网关鉴权、dictation 等仍按 managed 运行。
// POC_DIR 只在两类来源下启用：显式 dev 覆盖（CLAUDE_LOCAL_RECORDED_DIR），
// 或由后端 install_assets 写入代理旁的内嵌模板（mock/.codex-plus-template 标记）。
// 代理目录里来历不明的 mock 一律忽略（托管测试用 privateFixture 守住这条）。
const TRUSTED_TEMPLATE = fs.existsSync(path.join(here, "mock", ".codex-plus-template"));
const POC_DIR = runtime.recordedDir || (TRUSTED_TEMPLATE ? here : null);
const POC_MOCK = path.join(POC_DIR ?? here, "mock");
const POC_I18N = path.join(POC_DIR ?? here, "i18n");
// 托管模式在存在 PoC 数据（dev 录制目录或安装的内嵌 mock 模板）时走 PoC 数据路径；
// 完全没有模板数据时保持合成兜底（officialShell/localBootstrap + 状态告警）。
const dataManaged = runtime.managed && !POC_DIR;
const READ_CACHE = POC_DIR ? path.join(POC_DIR, ".cache") : CACHE_DIR;
const LOG = arg("log", "1") !== "0";
const READY_FILE = process.env.CLAUDE_LOCAL_READY_FILE;
const INSTANCE_ID = process.env.CLAUDE_LOCAL_INSTANCE_ID || crypto.randomUUID();
const STATE_FILE = process.env.CLAUDE_LOCAL_STATE_FILE;
const DICTATION_WS_PATH = "/api/ws/speech_to_text/voice_stream";
const DICTATION_HELPER_PATH = "/dictation/claude-stream";
const MAX_DICTATION_WS_CONNECTIONS = 4;
const MAX_DICTATION_WS_BYTES = 26 * 1024 * 1024;
const MAX_DICTATION_WS_IDLE_MS = 130_000;
const MAX_DICTATION_WS_DURATION_MS = 16 * 60_000;
const DICTATION_ALLOWED_HOSTS = new Set(["claude.ai", "claude.com", "api.anthropic.com"]);
const DICTATION_ALLOWED_ORIGINS = new Set(["https://claude.ai", "https://claude.com"]);
let lastClientAt = null;
let listeningPort = PORT;
let persistenceWarning = null;

if (!Number.isInteger(PORT) || PORT < 0 || PORT > 65535) throw new Error("Invalid Claude local proxy port");
if (runtime.managed && (!READY_FILE || !STATE_FILE)) throw new Error("Managed Claude requires runtime state paths");
if (runtime.managed && !officialAssets) throw new Error("Managed Claude requires the original Claude Desktop resources path");

fs.mkdirSync(CACHE_DIR, { recursive: true });

const key = fs.readFileSync(path.join(CA_DIR, "server.key"));
const cert = fs.readFileSync(path.join(CA_DIR, "server.crt"));
const ctx = tls.createSecureContext({ key, cert });

// ── mock 数据 ────────────────────────────────────────────────
// 显示给 Claude 的账号信息：用户名、邮箱与套餐来自本地设置。
const plan = planProfile(runtime.plan);
const displayName = runtime.displayName;
const isMax = plan.capabilities.includes("claude_max");
const ACCOUNT = {
  id: "c46bd1eb-fbbf-4e5a-9bb3-5ebe8521eb54",
  uuid: "c46bd1eb-fbbf-4e5a-9bb3-5ebe8521eb54",
  email: runtime.email,
  name: displayName,
  full_name: displayName,
  plan_type: plan.plan,
  plan: plan.plan,
  tier: plan.plan,
  user_tier: plan.plan,
  plan_tier: plan.plan,
  entitlement_tier: plan.plan,
  membership_plan: plan.plan,
  plan_display_name: plan.label,
  plan_display_label: plan.label,
  subscription: { plan: plan.plan, status: "active" },
  is_max: isMax,
  has_claude_max: isMax,
  capabilities: [
    "chat",
    "projects",
    "artifacts",
    "computer_use",
    "research",
    "connectors",
    "claude_code",
    "cowork",
  ],
  settings: { ...(runtime.localization ? { locale: "zh-Hans" } : {}), theme: "system" },
};

const ORG = {
  uuid: "e0e877d3-8475-44a1-a501-4fc431b085ab",
  id: "e0e877d3-8475-44a1-a501-4fc431b085ab",
  name: "Local",
  organization_type: "personal",
  raven_type: "personal",
  plan_type: plan.plan,
  plan: plan.plan,
  tier: plan.plan,
  user_tier: plan.plan,
  plan_tier: plan.plan,
  entitlement_tier: plan.plan,
  membership_plan: plan.plan,
  capabilities: [
    "chat",
    ...plan.capabilities,
    "projects",
    "artifacts",
    "computer_use",
    "research",
    "connectors",
    "claude_code",
    "cowork",
    "compliance_api",
  ],
  billing_type: "stripe_subscription",
  is_personal: true,
  settings: {},
  visibility_status: "active",
  parent_organization_uuid: null,
  rate_limit_tier: plan.rateLimitTier,
  analytics_subscription_plan: plan.analytics,
  plan_display_name: plan.label,
  plan_display_label: plan.label,
  has_hosted_code_environments: false,
  claude_ai_bootstrap_models_config: runtime.models.map((model) => ({
    model, name: runtime.modelNames?.[model]?.name || model, description: runtime.modelNames?.[model]?.description || "", paprika_modes: [], thinking_modes: [],
    hard_limit: modelWindow(runtime, model),
    context_window: modelWindow(runtime, model),
  })),
  created_at: "2026-01-01T00:00:00Z",
};

const BOOTSTRAP = {
  account: ACCOUNT,
  user: ACCOUNT,
  organization: ORG,
  organizations: [ORG],
  memberships: [{ uuid: "11111111-2222-4333-8444-555555555556", organization: ORG, role: "admin", status: "active", seat_tier: "full_member", email_address: ACCOUNT.email, created_at: "2025-01-01T00:00:00Z", updated_at: "2025-01-01T00:00:00Z" }],
  statsig: { user: { userID: ACCOUNT.id }, values: {}, dynamic_configs: {}, feature_gates: {} },
  feature_flags: Object.fromEntries(
    [
      "computer_use",
      "computer_use_rollout",
      "projects",
      "artifacts",
      "research",
      "connectors",
      "claude_code",
      "cowork",
    ].map((k) => [k, true]),
  ),
};
BOOTSTRAP.feature_flags.computer_use = runtime.computerUse;
BOOTSTRAP.feature_flags.computer_use_rollout = runtime.computerUse;
if (!runtime.computerUse) {
  ACCOUNT.capabilities = ACCOUNT.capabilities.filter((name) => name !== "computer_use");
  ORG.capabilities = ORG.capabilities.filter((name) => name !== "computer_use");
}

// 读取录制到的真实 bootstrap 作为模板（capture.mjs 产出）
function loadRecordedBootstrap() {
  if (dataManaged) return null;
  try {
    for (const f of fs.readdirSync(READ_CACHE)) {
      if (!f.endsWith(".json")) continue;
      const meta = JSON.parse(fs.readFileSync(path.join(READ_CACHE, f), "utf8"));
      if (typeof meta.url === "string" && meta.url.includes("/edge-api/bootstrap")) {
        const body = fs.readFileSync(path.join(READ_CACHE, f.replace(/\.json$/, "")));
        return JSON.parse(body.toString("utf8"));
      }
    }
  } catch {}
  return null;
}

const RECORDED_BOOTSTRAP = loadRecordedBootstrap();

// i18n 字典：.cache 里录制过的真实翻译文件（url 形如 claude.ai/i18n/s/zh-Hans.json?v=...）
const i18nMap = (() => {
  const m = new Map(); // pathname -> abs body file
  if (dataManaged) return m;
  try {
    for (const f of fs.readdirSync(READ_CACHE)) {
      if (!f.endsWith(".json")) continue;
      const meta = JSON.parse(fs.readFileSync(path.join(READ_CACHE, f), "utf8"));
      const u = String(meta.url || "");
      const bodyFile = path.join(READ_CACHE, f.replace(/\.json$/, ""));
      if (!u.includes("/i18n/") || meta.status !== 200 || !fs.existsSync(bodyFile)) continue;
      const pathPart = u.slice(u.indexOf("/i18n/")).split("?")[0];
      m.set(pathPart, bodyFile);
    }
  } catch {}
  return m;
})();

function readI18n(file) {
  if (!file) return {};
  try {
    const catalog = JSON.parse(fs.readFileSync(file, "utf8"));
    return catalog && !Array.isArray(catalog) && typeof catalog === "object" &&
      Object.values(catalog).every((value) => typeof value === "string") ? catalog : {};
  } catch {
    return {};
  }
}

// app.asar 的词表只覆盖桌面壳；主界面需要完整 Web 词表。
const I18N_DIR = path.join(here, "i18n");
const WEB_I18N_DIR = process.env.CLAUDE_LOCAL_WEB_I18N_DIR || path.resolve(here, "../../assets/claude-localization/catalogs");
const i18nResponses = new Map();
function localI18n(pathname) {
  const match = pathname.match(/^\/i18n\/(?:(s|dynamic)\/)?([a-zA-Z-]+)\.json$/);
  if (!match) return null;
  if (i18nResponses.has(pathname)) return i18nResponses.get(pathname);
  const [, kind, requestedLocale] = match;
  const locale = runtime.localization || requestedLocale === "zh-CN"
    ? "zh-Hans"
    : requestedLocale;
  const base = `${locale}.json`;
  const catalog = kind === "dynamic" ? {
    ...readI18n(path.join(WEB_I18N_DIR, "dynamic", base)),
    ...readI18n(i18nMap.get(pathname)),
  } : {
    ...(!dataManaged ? readI18n(path.join(POC_I18N || I18N_DIR, base)) : {}),
    ...readI18n(path.join(WEB_I18N_DIR, base)),
    ...readI18n(i18nMap.get(`/i18n/${base}`)),
    ...readI18n(path.join(WEB_I18N_DIR, `${locale}.overrides.json`)),
    ...(kind === "s" ? readI18n(i18nMap.get(pathname)) : {}),
  };
  if (kind === "s") {
    // 与客户端 cI() 一致：仅为无冲突的长 ID 补短别名，并保留已有短 ID。
    const prefixes = new Map();
    for (const id of Object.keys(catalog)) {
      if (!/^[A-Za-z0-9+/]{10}$/.test(id)) continue;
      const short = id.slice(0, 6);
      prefixes.set(short, prefixes.has(short) ? null : id);
    }
    for (const [short, id] of prefixes) {
      if (id !== null && !Object.hasOwn(catalog, short)) catalog[short] = catalog[id];
    }
  }
  const body = Buffer.from(JSON.stringify(catalog));
  i18nResponses.set(pathname, body);
  return body;
}

// ── 工具 ─────────────────────────────────────────────────────
const now = () => new Date().toISOString();
const log = (...a) => LOG && console.log(`[${now()}]`, ...a);

function json(res, obj, code = 200, extraHeaders = {}) {
  const body = JSON.stringify(obj);
  res.writeHead(code, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store",
    ...extraHeaders,
  });
  res.end(body);
}

function activeOrgCookies() {
  return [
    `org_uuid=${ORG.uuid}; Path=/; SameSite=Lax`,
    `active_org_uuid=${ORG.uuid}; Path=/; SameSite=Lax`,
    `last_active_org=${ORG.uuid}; Path=/; SameSite=Lax`,
    `lastActiveOrg=${ORG.uuid}; Domain=.claude.ai; Path=/; Secure; SameSite=None`,
    `sessionKey=${ACCOUNT.uuid}-local-session; Path=/; HttpOnly; Secure; SameSite=None`,
  ];
}

function jsonWithActiveOrg(res, obj, code = 200) {
  return json(res, obj, code, { "set-cookie": activeOrgCookies() });
}

function health() {
  return {
    instanceId: INSTANCE_ID, pid: process.pid, port: listeningPort, mode: MODE,
    lastClientAt, ...(persistenceWarning ? { warning: persistenceWarning } : {}),
  };
}

let readyWriteTimer;
function writeReady() {
  if (!READY_FILE) return;
  const temporary = `${READY_FILE}.${process.pid}.tmp`;
  try {
    fs.mkdirSync(path.dirname(READY_FILE), { recursive: true, mode: 0o700 });
    fs.writeFileSync(temporary, `${JSON.stringify(health())}\n`, { mode: 0o600 });
    fs.renameSync(temporary, READY_FILE);
    fs.chmodSync(READY_FILE, 0o600);
  } catch {
    console.error("Claude local readiness could not be saved");
  }
}

function noteClient() {
  lastClientAt = now();
  if (!readyWriteTimer) {
    readyWriteTimer = setTimeout(() => {
      readyWriteTimer = undefined;
      writeReady();
    }, 500);
    readyWriteTimer.unref();
  }
}

const CONV_STATE = new Map();
const NOTIFICATION_SETTINGS = new Map();
let DEVICE_ROW = null; // Code 模式注册的设备行 {id, k, ...}
const stateStore = createStateStore(STATE_FILE, CONV_STATE, NOTIFICATION_SETTINGS, (warning) => {
  persistenceWarning = warning;
  console.error(warning);
});

function ensureConv(uuid) {
  if (!CONV_STATE.has(uuid)) {
    CONV_STATE.set(uuid, {
      uuid,
      name: "New conversation",
      created_at: now(),
      updated_at: now(),
      chat_messages: [],
      current_leaf_message_uuid: null,
      // claudeai-rpc 侧状态：turns 是完整的消息实体历史，供 StreamTimeline 回放
      turns: [],
      nextIndex: 0,
      leafMessageId: null,
      model: runtime.model,
    });
    stateStore.changed();
  }
  return CONV_STATE.get(uuid);
}

// claudeai-rpc 服务：StreamTimeline / PerformAction / 其余 ConversationService 方法
const conversationService = createConversationService({
  ensureConv,
  listConvs: () => [...CONV_STATE.values()],
  log,
  onChange: (flush) => flush ? stateStore.flush() : stateStore.changed(),
});

function pushMessage(conv, msg) {
  const index = conv.chat_messages.length;
  const m = {
    uuid: msg.uuid,
    sender: msg.sender,
    index,
    parent_message_uuid: msg.parent ?? null,
    text: msg.text ?? "",
    content: [{ type: "text", text: msg.text ?? "", citations: [] }],
    created_at: now(),
    updated_at: now(),
    ...(msg.stop_reason ? { stop_reason: msg.stop_reason } : {}),
  };
  conv.chat_messages.push(m);
  conv.current_leaf_message_uuid = m.uuid;
  conv.updated_at = now();
  stateStore.changed();
  return m;
}

const FAKE_ACCOUNT = {
  uuid: ACCOUNT.id,
  id: ACCOUNT.id,
  email: ACCOUNT.email,
  name: ACCOUNT.name,
  full_name: ACCOUNT.name,
  plan_type: plan.plan,
  plan: plan.plan,
  tier: plan.plan,
  user_tier: plan.plan,
  plan_tier: plan.plan,
  entitlement_tier: plan.plan,
  membership_plan: plan.plan,
  plan_display_name: plan.label,
  plan_display_label: plan.label,
  is_max: isMax,
  has_claude_max: isMax,
  membership: { plan: plan.plan, status: "active", plan_type: plan.plan },
  subscription: { plan: plan.plan, status: "active" },
  organization_uuid: ORG.uuid,
  organization: ORG,
  capabilities: ACCOUNT.capabilities,
  settings: {},
};

const LOCAL_MEMBERSHIP_ID = "11111111-2222-4333-8444-555555555556";

function localMembership() {
  return {
    uuid: LOCAL_MEMBERSHIP_ID,
    organization_uuid: ORG.uuid,
    organization: ORG,
    role: "admin",
    status: "active",
    seat_tier: "full_member",
    joined: true,
    is_member: true,
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
  };
}

function fakeAccount() {
  const membership = localMembership();
  return {
    ...FAKE_ACCOUNT,
    tagged_id: `acct_${ACCOUNT.uuid}`,
    email_address: ACCOUNT.email,
    display_name: ACCOUNT.name,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    is_verified: true,
    age_is_verified: true,
    is_anonymous: false,
    onboarding_complete: true,
    workspace_memberships: [membership],
    invites: [],
    never_accepted_legal_document_types: [],
    accepted_clickwrap_versions: {},
    display_avatar: null,
    settings: { ...ACCOUNT.settings },
    memberships: [membership],
    memberships_incomplete: false,
  };
}

function localAccount(source = {}) {
  const account = source && typeof source === "object" && !Array.isArray(source) ? source : {};
  const normalized = {
    ...account,
    ...fakeAccount(),
    settings: { ...(account.settings || {}), ...ACCOUNT.settings },
  };
  const membership = localMembership();
  normalized.organization_uuid = ORG.uuid;
  normalized.organization = ORG;
  normalized.plan_type = plan.plan;
  normalized.plan = plan.plan;
  normalized.tier = plan.plan;
  normalized.user_tier = plan.plan;
  normalized.plan_tier = plan.plan;
  normalized.entitlement_tier = plan.plan;
  normalized.membership_plan = plan.plan;
  normalized.plan_display_name = plan.label;
  normalized.plan_display_label = plan.label;
  normalized.subscription = { plan: plan.plan, status: "active" };
  normalized.memberships = [membership];
  normalized.workspace_memberships = [membership];
  normalized.memberships_incomplete = false;
  normalized.onboarding_complete = true;
  return normalized;
}

function normalizeLocalIdentity(source = {}) {
  const payload = source && typeof source === "object" && !Array.isArray(source) ? source : {};
  const membership = localMembership();
  const account = localAccount(payload.account);
  const user = localAccount(payload.user || payload.account);
  return {
    ...payload,
    account,
    user,
    organization: ORG,
    activeOrganization: ORG,
    organizations: [ORG],
    memberships: [membership],
    resolved_org_uuid: ORG.uuid,
    org_uuid: ORG.uuid,
    active_org_uuid: ORG.uuid,
    organization_uuid: ORG.uuid,
    locale: runtime.localization ? "zh-Hans" : (payload.locale || "en-US"),
  };
}

function normalizeAccountProfile(source = {}) {
  const profile = source && typeof source === "object" && !Array.isArray(source) ? source : {};
  return {
    ...profile,
    account: localAccount(profile.account),
    organization: ORG,
    organization_uuid: ORG.uuid,
    activeOrganization: ORG,
    resolved_org_uuid: ORG.uuid,
    display_name: ACCOUNT.name,
    full_name: ACCOUNT.name,
    email_address: ACCOUNT.email,
    locale: runtime.localization ? "zh-Hans" : (profile.locale || "en-US"),
    membership: localMembership(),
    memberships: [localMembership()],
    workspace_memberships: [localMembership()],
  };
}

function localUserAccess() {
  return {
    features: [],
    account_features: [],
    organization_permissions: [],
    core_skill_names: [],
    code_execution_network_egress_enabled: false,
    code_execution_network_egress_template: "",
    code_execution_network_egress_settable: "disabled",
    artifacts_setting_blocks_classic_sharing: false,
    classic_sharing_block_reason: null,
  };
}

function localBootstrap() {
  return normalizeLocalIdentity({
    ...BOOTSTRAP,
    account: fakeAccount(),
    user: fakeAccount(),
    activeOrganization: ORG,
    resolved_org_uuid: ORG.uuid,
    locale: runtime.localization ? "zh-Hans" : "en-US",
    org_statsig: { user: {}, values: {}, values_hash: "local" },
    org_feature_flags: { user: {}, values: {}, values_hash: "local" },
    org_growthbook: { features: {}, hashing_algorithm: "fnv", user: {} },
    intercom_account_hash: null,
    intercom_user_jwt: null,
    server_localizations: null,
    system_prompts: null,
    current_user_access: localUserAccess(),
    ...modelSelectors(runtime),
    memory_mode: "melange",
    classic_mode_available: false,
    melange_available: false,
    claude_ai_available_models: { models: runtime.models.map((model) => ({ model_id: model, minimum_tier: "free" })) },
    personalized_greeting: null,
    hold_notice: null,
    served_copy: {},
    gated_messages: { messages: {}, gates: [], locale: runtime.localization ? "zh-Hans" : "en-US" },
    gated_imports: {},
    ...(officialAssets ? { gated_imports_build_id: officialAssets.buildId } : {}),
  });
}

// 选择器里显示的名称与副标题：有友好名用友好名（副标题是供应商），否则沿用模型 id。
const friendlyName = (id) => modelLabel(runtime, id).name;
const friendlyDescription = (id) => modelLabel(runtime, id).description || id;

// PoC 数据路径下：把网关池模型并入录制 bootstrap 的模型字段。
// 原生条目先拿来当新条目的模板；`hideUnroutedModels`（默认开）时最后再把它们删掉，
// 选择器里只剩池内 provider/model 条目。
function mergePoolModels(j) {
  const ids = runtime.models;
  if (!j || typeof j !== "object" || !ids.length) return;
  const cfg = j.activeOrganization?.claude_ai_bootstrap_models_config;
  if (Array.isArray(cfg)) {
    const seen = new Set(cfg.map((m) => m?.model));
    const tpl = cfg[0] ?? {};
    for (const entry of cfg) {
      if (entry && runtime.models.includes(entry.model)) {
        entry.hard_limit = modelWindow(runtime, entry.model);
        entry.context_window = modelWindow(runtime, entry.model);
      }
    }
    for (const id of ids) {
      if (!seen.has(id)) cfg.push({
        ...tpl,
        model: id,
        name: friendlyName(id),
        description: friendlyDescription(id),
        hard_limit: modelWindow(runtime, id),
        context_window: modelWindow(runtime, id),
      });
    }
  }
  const avail = j.claude_ai_available_models?.models;
  if (Array.isArray(avail)) {
    const seen = new Set(avail.map((m) => m?.model_id));
    for (const id of ids) {
      if (!seen.has(id)) avail.push({ model_id: id, minimum_tier: "free" });
    }
  }
  if (Array.isArray(j.model_selector_config)) {
    for (const sel of j.model_selector_config) {
      if (!Array.isArray(sel?.models)) continue;
      const seen = new Set(sel.models.map((m) => m?.id));
      const tpl = sel.models[0] ?? {};
      for (const entry of sel.models) {
        if (entry && runtime.models.includes(entry.id)) {
          entry.hard_limit = modelWindow(runtime, entry.id);
          entry.context_window = modelWindow(runtime, entry.id);
          entry.runtime = { max_input_tokens: modelWindow(runtime, entry.id) };
        }
      }
      for (const id of ids) {
        if (!seen.has(id)) sel.models.push({
          ...tpl,
          id,
          name: friendlyName(id),
          short_name: friendlyName(id),
          description: friendlyDescription(id),
          hard_limit: modelWindow(runtime, id),
          context_window: modelWindow(runtime, id),
          runtime: { max_input_tokens: modelWindow(runtime, id) },
        });
      }
    }
  }
  if (Array.isArray(j.model_selector_state)) {
    const thinking = { type: "effort", effort: "medium" };
    j.model_selector_state = j.model_selector_state.map((s) => s && typeof s === "object"
      ? { ...s, model: runtime.model, thinking_by_model: ids.map((id) => ({ id, thinking })) }
      : s);
  }
  if (runtime.hideUnroutedModels) dropUnroutedModels(j, runtime);
}

function mockEdgeApi(req, res, url) {
  const p = url.pathname.replace(/\/+$/, "");
  if (p.includes("/edge-api/bootstrap")) {
    return jsonWithActiveOrg(res, resolvedBootstrap());
  }
  if (p.endsWith("/edge-api/client-health/check")) return json(res, {});
  return json(res, {});
}

let resolvedBootstrapCache;
function resolvedBootstrap() {
  if (resolvedBootstrapCache) return resolvedBootstrapCache;
  let template = RECORDED_BOOTSTRAP ?? {};
  if (!dataManaged) {
    try { template = JSON.parse(fs.readFileSync(path.join(POC_MOCK, "bootstrap.json"), "utf8")); } catch {}
  }
  const result = normalizeLocalIdentity({
    ...localBootstrap(), ...structuredClone(template),
    intercom_account_hash: null, intercom_user_jwt: null,
  });
  result.served_copy = {
    ...result.served_copy,
    "web.plan_name.max_20x": runtime.localization ? "Codex++ 本地网关" : "Codex++ local gateway",
  };
  mergePoolModels(result);
  resolvedBootstrapCache = result;
  return result;
}

// REST 侧的会话条目：拿录制文件里的第一条当模板，叠动态字段
let REST_CONV_TEMPLATE = null;
function convToRest(c) {
  if (!REST_CONV_TEMPLATE) {
    try {
      if (dataManaged) throw new Error("Managed mode uses synthetic data");
      const f = path.join(POC_MOCK, "api", "conversations.json");
      REST_CONV_TEMPLATE = JSON.parse(fs.readFileSync(f, "utf8")).data[0] ?? {};
    } catch { REST_CONV_TEMPLATE = {}; }
  }
  const live = c.turns?.some((t) => !t.complete);
  return {
    ...REST_CONV_TEMPLATE,
    uuid: c.uuid,
    id: c.uuid,
    name: c.name || "New conversation",
    summary: c.name || "New conversation",
    model: c.model || runtime.model,
    created_at: c.created_at,
    updated_at: c.updated_at,
    current_leaf_message_uuid: c.leafMessageId || null,
    live_status: live ? "running" : null,
    last_settled_at: live ? null : c.updated_at,
  };
}

function mockApi(req, res, url) {
  const p = url.pathname.replace(/\/+$/, "");
  const method = req.method;

  // Code 的上下文来自 CLI model_selector，不读取网页的 hard_limit。
  const selector = p.match(/^\/api\/(?:organizations\/[^/]+\/)?model_selector\/([^/]+)$/);
  if (method === "GET" && selector) {
    const catalog = modelSelectors(runtime);
    const config = catalog.model_selector_config.find((entry) => entry.id === selector[1]);
    const state = catalog.model_selector_state.find((entry) => entry.id === selector[1]);
    return json(res, {
      model_selector_config: config ? [config] : [],
      model_selector_state: state ? [state] : [],
      organization_uuid: ORG.uuid,
    });
  }

  const convGet = p.match(/\/chat_conversations\/([0-9a-f-]{36})$/);
  if (convGet && method === "GET") {
    return json(res, convToRest(ensureConv(convGet[1])));
  }

  // 会话列表：静态模板 + 运行期动态会话（RPC 建的）合并，按更新时间倒序
  if (/\/chat_conversations_v2$/.test(p) && method === "GET") {
    const f = path.join(POC_MOCK, "api", "conversations.json");
    let base = { data: [], has_more: false };
    if (!dataManaged) {
      try { base = JSON.parse(fs.readFileSync(f, "utf8")); } catch {}
    }
    const dynamic = [...CONV_STATE.values()].map(convToRest);
    const seen = new Set(dynamic.map((c) => c.uuid));
    const merged = [...dynamic, ...(base.data ?? []).filter((c) => !seen.has(c.uuid))];
    merged.sort((a, b) => String(b.updated_at).localeCompare(String(a.updated_at)));
    return json(res, { ...base, data: merged });
  }

  // 录制下来的真实接口形状（mock/api/*.json）
  const apiMap = [
    [/\/reflections\/settings$/, "reflections_settings"],
    [/\/user_settings$/, "claude_code_user_settings"],
    [/\/mcp\/remote_servers_with_connection$/, "mcp_servers"],
    [/\/memory\/settings$/, "memory_settings"],
    [/^\/api\/account_profile$/, "account_profile"],
    [/\/trial_status$/, "trial_status"],
    [/\/skills\/list-skills$/, "skills"],
    [/\/experiences\/claude_web$/, "experiences"],
  ];
  for (const [re, slug] of apiMap) {
    if (re.test(p)) {
      const f = path.join(POC_MOCK, "api", `${slug}.json`);
      if (!dataManaged && fs.existsSync(f)) {
        try {
          const data = JSON.parse(fs.readFileSync(f, "utf8"));
          return slug === "account_profile"
            ? jsonWithActiveOrg(res, normalizeAccountProfile(data))
            : json(res, data);
        } catch {}
      }
      if (dataManaged) {
        const synthetic = {
          reflections_settings: { time_alert_daily_minutes: null, quiet_hours: null },
          claude_code_user_settings: {
            userId: ACCOUNT.uuid, version: 1, lastModified: "2026-01-01T00:00:00Z",
            checksum: "local-1", content: { entries: {} },
          },
          mcp_servers: [],
          memory_settings: {
            enabled_saffron: null, enabled_saffron_search: false, enabled_melange: null,
            memory_mode: "melange", classic_mode_available: false,
            melange_available: false, melange_imported_at: null,
          },
          account_profile: {
            work_function: "", conversation_preferences: null, cowork_global_instructions: null,
            cowork_instructions_union: false, conversation_preferences_cloud_max_chars: 10000,
            conversation_preferences_over_cloud_max_chars: false,
            locale: runtime.localization ? "zh-Hans" : "en-US", onboarding_topics: [], science_fields: [],
            avatar: null, pixel_avatar: null, avatar_image_url: null, display_avatar: null,
          },
          trial_status: { eligible: false, in_trial: false, trial_status: null },
          skills: { skills: [] },
          experiences: { experiences: [], rules: { global: { rate_limit: null, cooldown: null }, placements: {}, tiers: {} } },
        };
        return slug === "account_profile"
          ? jsonWithActiveOrg(res, normalizeAccountProfile(synthetic[slug]))
          : json(res, synthetic[slug]);
      }
    }
  }

  // Code 设置页直接对响应调用 filter，已连接实例必须返回顶层数组。
  if (method === "GET" && /^\/api\/oauth\/organizations\/[^/]+\/oauth_tokens$/.test(p)) {
    return json(res, []);
  }

  // 本地模拟账号不需要身份验证；空对象会被设置页判为尚未完成验证。
  if (method === "GET" && /^\/api\/organizations\/[^/]+\/kyc_status$/.test(p)) {
    return json(res, { status: "not_required", should_poll: false });
  }
  // 原生桌面使用已求值的 GrowthBook 结果；用户开关和系统权限仍由客户端检查。
  if (method === "GET" && p === "/api/desktop/features") {
    return json(res, { features: {
      "1291166712": {
        value: { enabled: runtime.computerUse, appScopedEnabled: runtime.computerUse && runtime.backgroundMode },
        on: runtime.computerUse, off: !runtime.computerUse, source: "force",
        experiment: null, experimentResult: null,
      },
    } });
  }

  // 账户页会直接读取 reasons.length；本地后端不开放账号删除。
  if (method === "GET" && p === "/api/account/deletion-allowed") {
    return json(res, { deletable: false, reasons: [], display_message: null });
  }
  if (method === "GET" && p === "/api/auth/trusted_devices") {
    return json(res, { devices: [] });
  }
  if (method === "GET" && p === "/api/auth/mfa") {
    return json(res, { mfa_enrolled: false, methods: [] });
  }
  if (method === "GET" && p === "/api/auth/sessions/list-active") {
    return json(res, { data: [], pagination: { has_more: false, total: 0 } });
  }
  if (method === "GET" && /^\/api\/stripe\/[^/]+\/invoices$/.test(p)) {
    return json(res, { invoices: [], next_page: null });
  }
  if (method === "GET" && /^\/api\/organizations\/[^/]+\/user_artifacts$/.test(p)) {
    return json(res, { artifacts: [] });
  }
  if (method === "GET" && /^\/api\/organizations\/[^/]+\/user_artifacts\/count$/.test(p)) {
    return json(res, { count: 0 });
  }
  if (method === "GET" && /^\/api\/frame\/frames(?:\/external)?$/.test(p)) {
    return json(res, { frames: [] });
  }
  const notifications = p.match(/^\/api\/organizations\/([^/]+)\/notification\/preferences$/);
  if (notifications && (method === "GET" || method === "PATCH")) {
    const org = notifications[1];
    const state = NOTIFICATION_SETTINGS.get(org) ?? {
      preferences: { feature_preference: {} },
      effective_preferences: {},
      push_reachability: {
        has_active_channel: false, platforms: [], most_recent_token_refresh: null,
      },
    };
    NOTIFICATION_SETTINGS.set(org, state);
    if (method === "GET") return json(res, state);
    return readBody(req, (body) => {
      let patch;
      try { patch = JSON.parse(body.toString("utf8"))?.preferences?.feature_preference; } catch {}
      if (!patch || typeof patch !== "object" || Array.isArray(patch)) {
        return json(res, { error: "Invalid notification preferences" }, 400);
      }
      // 按功能和渠道合并，保留未修改项以及显式关闭的 false。
      for (const [feature, channels] of Object.entries(patch)) {
        if (!channels || typeof channels !== "object" || Array.isArray(channels)) continue;
        for (const channel of ["push", "email"]) {
          const key = `enable_${channel}`;
          if (typeof channels[key] !== "boolean") continue;
          const previous = Object.hasOwn(state.preferences.feature_preference, feature)
            ? state.preferences.feature_preference[feature] : {};
          const effective = Object.hasOwn(state.effective_preferences, feature)
            ? state.effective_preferences[feature] : {};
          state.preferences.feature_preference = {
            ...state.preferences.feature_preference, [feature]: { ...previous, [key]: channels[key] },
          };
          state.effective_preferences = {
            ...state.effective_preferences,
            [feature]: { ...effective, [key]: channels[key], [`${channel}_source`]: "explicit" },
          };
        }
      }
      stateStore.changed();
      return json(res, state);
    });
  }

  // 分页/列表类接口：客户端会 flatMap 取 .items/.data，回 {} 会出 [undefined] 元素
  // → render 时 TypeError 进 fatal boundary（0NV6PUA 崩溃页的来源）
  // 列表页校验（shared-l-p-m-w-0 rG）：{data:[...], pagination:{has_more:boolean}} 必须成立
  const pageShape = {
    items: [], data: [], next_cursor: null, has_more: false, facets: null,
    pagination: { has_more: false, next_cursor: null, cursor: null },
  };
  if (/\/library(\/|$)/.test(p)) return json(res, pageShape);
  if (/\/projects\/count$/.test(p)) return json(res, { count: 0 });
  if (/\/projects(_v2)?$/.test(p)) return json(res, pageShape);
  if (/\/plugins\/list-plugins$/.test(p)) {
    return json(res, { plugins: [], installed_plugin_ids: [] });
  }
  // Code 模式 OAuth：刷新 token（被通用 /v1/ 兜底吃掉会导致 stale_relogin 常驻）
  if (p === "/v1/oauth/token" && req.method === "POST") {
    return json(res, {
      access_token: "claude-local-access-" + crypto.randomBytes(16).toString("hex"),
      refresh_token: "claude-local-refresh-" + crypto.randomBytes(16).toString("hex"),
      token_type: "bearer",
      expires_in: 86400,
      scope: "user:profile user:inference user:sessions:claude_code",
      created_at: Math.floor(Date.now() / 1000),
    });
  }
  // /v1/code/sessions/watch 是 EventSource：必须 text/event-stream
  // 实测桌面端也会直接对 /v1/code/sessions 开 EventSource；按 Accept 分流，
  // 否则 EventSource 收到 application/json 会中断连接，Code 面板永远停在骨架屏。
  const wantsSse = req.headers.accept?.includes("text/event-stream");
  if (p === "/v1/code/sessions/watch" || p.startsWith("/v1/code/sessions/watch") ||
      (wantsSse && (/^\/v1\//.test(p) || /\/mcp\/v2\/bootstrap$/.test(p)))) {
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-store",
      connection: "keep-alive",
    });
    res.write(`event: keep_alive\ndata: {}\n\n`);
    const ka = setInterval(() => res.write(`event: keep_alive\ndata: {}\n\n`), 25000);
    res.on("close", () => clearInterval(ka));
    return;
  }
  // 代码会话/环境（epitaxy 路由）：(n ?? Ra).filter 要求字段为数组
  if (/^\/v1\//.test(p)) {
    return json(res, {
      sessions: [], triggers: [], environments: [], data: [], items: [],
      next_cursor: null, has_more: false,
    });
  }
  if (/\/marketplaces\//.test(p)) {
    return json(res, { ...pageShape, marketplaces: [] });
  }
  if (p.startsWith("/api/frame/")) return json(res, pageShape);
  // 客户端按 plugin id 读取元数据字典，每个值都必须是对象。
  if (p.endsWith("/cowork_plugin_metadata")) return json(res, {});

  // Code 模式设备注册：POST 提交 enclave 公钥注册设备行，GET 要看到 {id, k} 行；
  // 空 {} 会触发 device_register_miss(no_row_pk) → "Couldn't verify your sign-in"
  if (/\/cowork\/remote_devices$/.test(p)) {
    if (req.method === "POST" || req.method === "PUT") {
      let body = "";
      req.on("data", (c) => (body += c));
      req.on("end", () => {
        try {
          const d = JSON.parse(body);
          DEVICE_ROW = {
            id: crypto.randomUUID(),
            ...d,
            k: d.k ?? d.public_key ?? d.enclave_public_key ?? d.key ?? d.device_key,
          };
        } catch {
          DEVICE_ROW ??= { id: crypto.randomUUID() };
        }
        return json(res, DEVICE_ROW);
      });
      return;
    }
    return json(res, {
      devices: DEVICE_ROW ? [DEVICE_ROW] : [],
      rows: DEVICE_ROW ? [DEVICE_ROW] : [],
      items: DEVICE_ROW ? [DEVICE_ROW] : [],
      data: DEVICE_ROW ? [DEVICE_ROW] : [],
    });
  }

  // /upgrade 页：缺这些会 TypeError 进 fatal boundary（toLowerCase of undefined）
  if (/\/subscription_details$/.test(p)) {
    return json(res, {
      plan: { code: `claude_${plan.plan}`, name: plan.label, tier: plan.plan },
      status: "active", billing_type: "stripe_subscription",
      interval: "month", amount: 10000, currency: "usd",
      next_payment_date: null, cancel_at_period_end: false, seats: 1,
    });
  }
  if (p.includes("/api/billing/")) {
    return json(res, { plans: [], prices: [], items: [], currency: "usd", available_plans: [] });
  }
  if (/eligible$/.test(p)) return json(res, { eligible: false });

  // 当前用户访问权限（app 用它判断能否进入主界面 + 套餐判定）
  if (/\/api\/bootstrap\/[^/]+\/current_user_access$/.test(p)) {
    let base = localUserAccess();
    if (!dataManaged) {
      try { base = JSON.parse(fs.readFileSync(path.join(POC_MOCK, "bootstrap.json"), "utf8")).current_user_access ?? base; } catch {}
    }
    return jsonWithActiveOrg(res, {
      ...base,
      has_access: true,
      can_access: true,
      status: "allowed",
      is_eligible: true,
      reason: null,
      plan: plan.plan,
      plan_type: plan.plan,
      subscription_plan: plan.plan,
      billing_type: "stripe_subscription",
      rate_limit_tier: plan.rateLimitTier,
      analytics_subscription_plan: plan.analytics,
      organization: ORG,
      organization_uuid: ORG.uuid,
      account: localAccount(ACCOUNT),
      memberships: [localMembership()],
      workspace_memberships: [localMembership()],
      membership: localMembership(),
      is_member: true,
      member: true,
      joined: true,
      activeOrganization: ORG,
      resolved_org_uuid: ORG.uuid,
      feature_flags: BOOTSTRAP.feature_flags,
    });
  }
  // 隐私同意（非 /api 前缀，单独兜底）
  if (p.startsWith("/v1/privacy-consents")) {
    return json(res, { consents: {}, data: [], prefix: "cookies." });
  }

  if (p === "/api/bootstrap" || p === "/api/account" || p === "/api/account_profile") {
    return jsonWithActiveOrg(res, resolvedBootstrap());
  }
  if (/^\/api\/bootstrap\/[^/]+\/app_start$/.test(p)) {
    return jsonWithActiveOrg(res, resolvedBootstrap());
  }
  if (p === "/api/organizations") {
    return jsonWithActiveOrg(res, { organizations: [ORG], data: [ORG], items: [ORG] });
  }
  // org 级别的设置/记忆/skills 端点：未覆盖的通用 mock 形状会触发前端 parse 失败 notice
  if (/\/memory\/settings/.test(p)) {
    return json(res, { settings: {}, entries: [], items: [] });
  }
  if (/\/skills\/download-dot-skill-file/.test(p)) {
    // 区域封锁拿不到真 zip；回空 body 让下载器安静失败而不是 parse 报错
    res.writeHead(204, { "content-length": 0 });
    return res.end();
  }
  // 登录后组织选择页的数据源；item 的 uuid 要放在客户端读的位置（否则 join 打成 /undefined/join）
  if (p === "/api/organizations/discoverable") {
    // 已经加入的 Local 组织不能出现在 discoverable 列表，否则官方桌面端会
    // 在主界面显示“加入您的团队”横幅。显式 join 路由仍保留给旧 onboarding
    // 流程重试，但正常登录态应视为没有可加入的组织。
    return jsonWithActiveOrg(res, { organizations: [], data: [], items: [] });
  }
  // Join 组织确认
  if (/^\/api\/organizations\/[^/]+\/(join|memberships?)$/.test(p) && method !== "GET") {
    return jsonWithActiveOrg(res, {
      ok: true,
      organization: ORG,
      organization_uuid: ORG.uuid,
      membership: localMembership(),
      memberships: [localMembership()],
      joined: true,
      is_member: true,
    });
  }
  // 组织详情不能带 Set-Cookie：Desktop 主进程监听 lastActiveOrg cookie，一变就重查插件白名单，
  // 而白名单检查正是请求这个接口——每次响应都重写 cookie 会形成死循环，主进程卡死、窗口点不动。
  // 活动组织的 cookie 由 bootstrap / account 等入口设置一次就够了。
  if (/^\/api\/organizations\/[^/]+$/.test(p)) {
    return json(res, ORG);
  }
  if (/\/chat_conversations$/.test(p)) {
    if (method === "GET") return json(res, [...CONV_STATE.values()].map(convToRest));
    if (method === "POST") {
      const id = crypto.randomUUID();
      const conv = ensureConv(id);
      return json(res, convToRest(conv), 201);
    }
  }
  const convMatch = p.match(/\/chat_conversations\/([^/]+)$/);
  if (convMatch) {
    if (CONV_STATE.has(convMatch[1])) return json(res, convToRest(ensureConv(convMatch[1])));
    return json(res, { error: "not found" }, 404);
  }
  if (/\/completion$/.test(p)) {
    let raw = "";
    req.on("data", (c) => (raw += c));
    req.on("end", () => {
      let body = {};
      try { body = JSON.parse(raw); } catch {}
      const prompt = typeof body.prompt === "string" ? body.prompt : "";
      const human = body.turn_message_uuids?.human_message_uuid || crypto.randomUUID();
      const asst = body.turn_message_uuids?.assistant_message_uuid || crypto.randomUUID();
      const model = gatewayModel(runtime, body.model || process.env.GATEWAY_MODEL);
      const sendEv = (ev, obj) => {
        res.write(`event: ${ev}\ndata: ${JSON.stringify(obj)}\n\n`);
      };
      res.writeHead(200, {
        "content-type": "text/event-stream; charset=utf-8",
        "cache-control": "no-cache",
        connection: "keep-alive",
      });
      sendEv("message_start", {
        type: "message_start",
        message: { uuid: asst, parent_uuid: human, model: body.model || model, role: "assistant", content: [] },
      });
      sendEv("content_block_start", { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } });
      const convUuid = (p.match(/\/chat_conversations\/([0-9a-f-]{36})\/completion/) || [])[1];
      const conv = convUuid ? ensureConv(convUuid) : null;
      if (conv && !conv.chat_messages.some((m) => m.uuid === human)) {
        pushMessage(conv, { uuid: human, sender: "human", text: prompt });
      }
      let acc = "";
      const payload = JSON.stringify({
        model,
        max_tokens: 2048,
        stream: true,
        messages: conv ? conv.chat_messages.map((message) => ({
          role: message.sender === "human" ? "user" : "assistant", content: message.text,
        })) : [{ role: "user", content: prompt }],
      });
      const target = gatewayTarget(runtime, "/v1/messages");
      const greq = (target.protocol === "https:" ? https : http).request(
        {
          hostname: target.hostname,
          port: target.port || (target.protocol === "https:" ? 443 : 80),
          path: target.pathname + target.search,
          method: "POST",
          headers: {
            "content-type": "application/json",
            "x-api-key": runtime.key,
            authorization: `Bearer ${runtime.key}`,
            "anthropic-version": "2023-06-01",
            accept: "text/event-stream",
          },
        },
        (gres) => {
          let buf = "";
          gres.on("data", (chunk) => {
            buf += chunk.toString("utf8");
            let i;
            while ((i = buf.indexOf("\n\n")) >= 0) {
              const ev = buf.slice(0, i);
              buf = buf.slice(i + 2);
              const dataLine = ev.split("\n").find((l) => l.startsWith("data:"));
              if (!dataLine) continue;
              let d;
              try { d = JSON.parse(dataLine.slice(5).trim()); } catch { continue; }
              if (d.type === "content_block_delta" && d.delta?.type === "text_delta") {
                acc += d.delta.text;
                sendEv("content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: d.delta.text } });
              } else if (d.type === "message_stop") {
                if (conv && !conv.chat_messages.some((m) => m.uuid === asst)) {
                  pushMessage(conv, { uuid: asst, sender: "assistant", parent: human, text: acc, stop_reason: "end_turn" });
                  stateStore.flush();
                }
                sendEv("message_delta", { type: "message_delta", delta: { stop_reason: "end_turn" } });
                sendEv("message_stop", { type: "message_stop" });
                res.end();
              }
            }
          });
          gres.on("end", () => { try { res.end(); } catch {} });
        },
      );
      greq.on("error", (e) => {
        sendEv("error", { type: "error", error: { message: e.message } });
        try { res.end(); } catch {}
      });
      greq.end(payload);
    });
    return;
  }
  if (/\/completion_status/.test(p)) {
    return json(res, { is_pending: false, is_error: false });
  }
  if (/\/(usage|rate_limits|limits)/.test(p)) {
    // windows 是前端由 limits 转换出的视图。本地网关没有 Anthropic 套餐额度。
    return json(res, {
      limits: [],
      five_hour: null,
      seven_day: null,
      seven_day_oauth_apps: null,
      seven_day_opus: null,
      seven_day_cowork: null,
      extra_usage: { is_enabled: false, monthly_limit: null, used_credits: null, utilization: null, currency: "USD" },
      iguana_necktie: null,
    });
  }
  if (/\/feature|flags|eligib/.test(p)) {
    return json(res, BOOTSTRAP.feature_flags);
  }
  // 未知 API 兜底；有结构要求的接口必须在上方单独实现。
  return json(res, {});
}

// ── 上游转发（带缓存） ───────────────────────────────────────
function cachePathFor(keyStr) {
  return path.join(READ_CACHE, crypto.createHash("sha1").update(keyStr).digest("hex"));
}

function isHashedStaticAsset(req, url) {
  if (req.method !== "GET" || req.headers.range || url.search) return false;
  const prefix = url.hostname === "assets-proxy.anthropic.com" ? "/claude-ai/v2/assets/"
    : /^(?:www\.)?claude\.(?:ai|com)$/.test(url.hostname) ? "/assets/" : null;
  return prefix !== null && url.pathname.startsWith(prefix) &&
    /\/[^/]+-[A-Za-z0-9_-]{8,}\.(?:m?js|css|wasm|woff2?|ttf|otf|png|jpe?g|webp|gif|svg|ico)$/.test(url.pathname);
}

function bufferedResponseHeaders(headers, length) {
  const result = { ...headers };
  const connectionHeaders = String(result.connection || "").split(",").map((name) => name.trim().toLowerCase());
  for (const name of ["connection", "transfer-encoding", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer", "upgrade", ...connectionHeaders]) {
    delete result[name];
  }
  // Buffer 未解压，保留 content-encoding 并以实际传输字节数更新长度。
  result["content-length"] = length;
  return result;
}

function serveCachedStaticAsset(req, res, url) {
  if (MODE === "passthrough" || !isHashedStaticAsset(req, url)) return false;
  const cacheKey = `${url.host}${url.pathname}${url.search}`;
  const cp = cachePathFor(cacheKey);
  try {
    const meta = JSON.parse(fs.readFileSync(`${cp}.json`, "utf8"));
    if (meta.status !== 200 || meta.url !== cacheKey ||
        typeof meta.headers?.["content-type"] !== "string" ||
        /\b(?:private|no-store)\b/i.test(String(meta.headers["cache-control"] || ""))) return false;
    const body = fs.readFileSync(cp);
    const headers = bufferedResponseHeaders(meta.headers, body.length);
    delete headers["set-cookie"];
    res.writeHead(200, headers);
    res.end(body);
    return true;
  } catch {
    return false;
  }
}

function readBody(req, cb) {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => cb(Buffer.concat(chunks)));
  req.on("error", () => cb(Buffer.alloc(0)));
}

// 文档导航判定：GET + 无扩展名 + 显式要 HTML。/api|/v1|/edge-api 等接口路径在此前分支已返回。
function isDocNav(req, url) {
  return req.method === "GET" && !path.extname(url.pathname) &&
    String(req.headers.accept || "").includes("text/html");
}

// 缓存里的真 SPA 外壳：>=4KB 的 200 HTML；登出落地页/重定向桩体（143B 等）一律不算。
function readDocShell(host) {
  for (const key of [`${host}/epitaxy`, `${host}/login`, `${host}/new`, `${host}/`]) {
    const cp = cachePathFor(key);
    try {
      if (!fs.existsSync(cp) || !fs.existsSync(`${cp}.json`)) continue;
      const meta = JSON.parse(fs.readFileSync(`${cp}.json`, "utf8"));
      const body = fs.readFileSync(cp);
      if (meta.status === 200 && body.length >= 4096) return body;
    } catch { /* ignore corrupt entries */ }
  }
  return null;
}

// 文档导航的规范外壳兜底：优先回放缓存真壳；缺失时从上游抓一次 /epitaxy 写穿。
// 绝不回放登出落地页桩体——那会让 SPA 永远卡在骨架屏。
function serveDocShell(req, res, url) {
  const host = url.host || "claude.ai";
  const cached = readDocShell(host);
  if (cached) {
    res.writeHead(200, {
      "content-type": "text/html; charset=utf-8",
      "content-length": cached.length,
      "cache-control": "no-store",
    });
    return res.end(cached);
  }
  const base = UPSTREAM_MAP[host] || `https://${host}`;
  const target = new URL("/epitaxy", base);
  // 带上 Claude Desktop 自己的浏览器 UA：只写「Claude」会被 Cloudflare 当成机器人 403。
  const userAgent = String(req.headers["user-agent"] || "") || "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Claude Chrome Safari/537.36";
  const up = upstreamRequest(
    target,
    {
      hostname: target.hostname,
      port: target.port || 443,
      path: "/epitaxy",
      method: "GET",
      headers: { accept: "text/html", host, "user-agent": userAgent },
      timeout: 15000,
    },
    (r) => {
      const chunks = [];
      r.on("data", (c) => chunks.push(c));
      r.on("end", () => {
        const body = Buffer.concat(chunks);
        if (r.statusCode === 200 && body.length >= 4096) {
          const cp = cachePathFor(`${host}/epitaxy`);
          fs.mkdirSync(path.dirname(cp), { recursive: true });
          fs.writeFileSync(cp, body);
          fs.writeFileSync(
            `${cp}.json`,
            JSON.stringify({ status: 200, headers: { "content-type": "text/html" }, url: `${host}/epitaxy` }),
          );
        }
        res.writeHead(r.statusCode || 502, {
          "content-type": "text/html; charset=utf-8",
          "content-length": body.length,
        });
        res.end(body);
      });
    },
  );
  up.on("timeout", () => up.destroy(new Error("timeout")));
  up.on("error", () => {
    res.writeHead(502);
    res.end("upstream shell unavailable\n");
  });
  up.end();
}

function forwardGateway(req, res, requestPath, raw) {
  let body = raw;
  const requested = new URL(requestPath, "http://localhost");
  if (req.method === "POST" && requested.pathname === "/v1/messages") {
    try { body = gatewayBody(runtime, raw); }
    catch { return json(res, { error: "Invalid model request" }, 400); }
  }
  const target = gatewayTarget(runtime, requestPath);
  const headers = {
    ...req.headers, host: target.host, "content-length": body.length,
    "x-api-key": runtime.key, authorization: `Bearer ${runtime.key}`,
  };
  for (const key of ["proxy-connection", "accept-encoding", "connection", "cookie", "transfer-encoding"]) delete headers[key];
  const up = (target.protocol === "https:" ? https : http).request({
    hostname: target.hostname, port: target.port || (target.protocol === "https:" ? 443 : 80),
    path: target.pathname + target.search, method: req.method, headers, timeout: 300000,
  }, (response) => {
    res.writeHead(response.statusCode, response.headers);
    response.pipe(res);
  });
  up.on("timeout", () => up.destroy(new Error("gateway timeout")));
  up.on("error", () => {
    if (res.headersSent) return res.destroy();
    json(res, { type: "error", error: { type: "api_error", message: "Local gateway unavailable" } }, 502);
  });
  req.once("aborted", () => up.destroy());
  up.end(body);
}

// 测试/运维可把某主机的上游重定向到本地 fixture；默认保留原始主机走 https。
const UPSTREAM_MAP = (() => {
  try { return JSON.parse(process.env.CLAUDE_LOCAL_UPSTREAM_MAP || "{}"); } catch { return {}; }
})();

// 抓 claude.ai 网页（首次写穿缓存的外壳、静态资源）可选经出站代理：受限地区直连会被 302 到
// app-unavailable-in-region。代理地址由后端检测系统代理后传入（只支持 http:// 的 CONNECT 代理）。
// 本机地址一律直连。
const UPSTREAM_PROXY = (() => {
  try {
    const proxy = new URL(process.env.CLAUDE_LOCAL_UPSTREAM_PROXY || "");
    return proxy.protocol === "http:" && proxy.hostname ? proxy : null;
  } catch {
    return null;
  }
})();

function isLoopbackHost(hostname) {
  return ["localhost", "127.0.0.1", "::1", "[::1]"].includes(String(hostname).toLowerCase());
}

/** 发往上游网页的请求：https 且配置了出站代理时先 CONNECT 打通隧道，再在隧道上做 TLS。 */
function upstreamRequest(target, options, callback) {
  const client = target.protocol === "http:" ? http : https;
  if (!UPSTREAM_PROXY || target.protocol !== "https:" || isLoopbackHost(target.hostname)) {
    return client.request(options, callback);
  }
  const port = Number(options.port || 443);
  const authority = `${options.hostname}:${port}`;
  const proxyAuth = UPSTREAM_PROXY.username
    ? { "proxy-authorization": `Basic ${Buffer.from(`${decodeURIComponent(UPSTREAM_PROXY.username)}:${decodeURIComponent(UPSTREAM_PROXY.password)}`).toString("base64")}` }
    : {};
  return https.request({
    ...options,
    agent: undefined,
    // ClientRequest 支持回调式 createConnection：隧道建好后再交出 TLS socket。
    createConnection: (_options, done) => {
      const connect = http.request({
        host: UPSTREAM_PROXY.hostname,
        port: Number(UPSTREAM_PROXY.port || 80),
        method: "CONNECT",
        path: authority,
        headers: { host: authority, ...proxyAuth },
        timeout: options.timeout || 20000,
      });
      connect.once("connect", (response, socket) => {
        if (response.statusCode !== 200) {
          socket.destroy();
          return done(new Error(`upstream proxy CONNECT ${response.statusCode}`));
        }
        done(null, tls.connect({ socket, servername: options.hostname }));
      });
      connect.once("timeout", () => connect.destroy(new Error("upstream proxy timeout")));
      connect.once("error", (error) => done(error));
      connect.end();
    },
  }, callback);
}

function proxyUpstream(req, res, url) {
  const cacheKey = `${url.host}${url.pathname}${url.search}`;
  const cp = cachePathFor(cacheKey);
  const metaPath = `${cp}.json`;
  const staticAsset = isHashedStaticAsset(req, url);
  const document = /^(?:www\.)?claude\.(?:ai|com)$/.test(url.hostname) && isDocNav(req, url);
  const canCache = MODE !== "passthrough" && (staticAsset || document);

  // 保留原始主机（downloads.claude.ai 等），不再统一改写到 claude.ai。
  const base = UPSTREAM_MAP[url.host] || `https://${url.host || "claude.ai"}`;
  const target = new URL(url.pathname + url.search, base);
  const headers = { ...req.headers, host: url.host || target.host };
  delete headers["proxy-connection"];
  delete headers["accept-encoding"];

  const upstreamReq = upstreamRequest(
    target,
    {
      hostname: target.hostname,
      port: target.port || 443,
      path: target.pathname + target.search,
      method: req.method,
      headers,
      timeout: 20000,
    },
    (up) => {
      const chunks = [];
      up.on("data", (c) => chunks.push(c));
      up.on("end", () => {
        const body = Buffer.concat(chunks);
        const ok = up.statusCode >= 200 && up.statusCode < 400;
        // 文档路由不缓存桩体/落地页 HTML（登出页会让 SPA 卡骨架屏）
        const docStub = isDocNav(req, url) && body.length < 4096;
        if (ok && !docStub && canCache && (!staticAsset || (
          up.statusCode === 200 && !up.headers["set-cookie"] &&
          !/\b(?:private|no-store)\b/i.test(String(up.headers["cache-control"] || ""))
        ))) {
          fs.mkdirSync(path.dirname(cp), { recursive: true });
          fs.writeFileSync(cp, body);
          fs.writeFileSync(
            metaPath,
            JSON.stringify({ status: up.statusCode, headers: bufferedResponseHeaders(up.headers, body.length), url: cacheKey }),
          );
        }
        res.writeHead(up.statusCode, bufferedResponseHeaders(up.headers, body.length));
        res.end(body);
      });
    },
  );
  upstreamReq.on("timeout", () => upstreamReq.destroy(new Error("upstream timeout")));
  upstreamReq.on("error", (err) => {
    if (serveCachedStaticAsset(req, res, url)) return;
    // 动态接口不能回放其它会话的缓存；文档仍沿用本地外壳兜底。
    if (canCache && document && fs.existsSync(cp) && fs.existsSync(metaPath)) {
      const meta = JSON.parse(fs.readFileSync(metaPath, "utf8"));
      const body = fs.readFileSync(cp);
      const docStub = isDocNav(req, url) && (meta.status !== 200 || body.length < 4096);
      if (docStub) {
        // 桩体不落回客户端；真外壳兜底已在上层分支兜底
        res.writeHead(502, { "content-type": "text/plain; charset=utf-8" });
        res.end("upstream unavailable (doc stub suppressed)\n");
        return;
      }
      res.writeHead(meta.status || 200, bufferedResponseHeaders(meta.headers, body.length));
      res.end(body);
      log(`CACHE HIT ${url.pathname}`);
      return;
    }
    log(`UPSTREAM FAIL ${url.pathname}: ${err.message}`);
    res.writeHead(502, { "content-type": "text/plain; charset=utf-8" });
    res.end(`upstream unavailable: ${err.message}\n`);
  });
  req.pipe(upstreamReq);
}

// ── 请求处理 ─────────────────────────────────────────────────
const demoHtml = MODE === "demo" ? fs.readFileSync(path.join(here, "demo", "index.html")) : null;

function handle(req, res, hostHeader) {
  const host = hostHeader || "claude.ai";
  const url = new URL(req.url, `https://${host}`);
  noteClient();
  log(`${req.method} ${url.pathname}`);

  const hostName = url.hostname.toLowerCase();
  const isMainClaudeHost = /^(claude\.ai|www\.claude\.ai|claude\.com|www\.claude\.com)$/.test(hostName);
  if (MODE === "mock" && req.method === "GET" &&
      (isMainClaudeHost || hostName === "api.anthropic.com") &&
      /^\/api\/(?:organizations\/[^/]+\/)?model_selector\/[^/]+\/?$/.test(url.pathname)) {
    return mockApi(req, res, url);
  }
  if (runtime.managed && isMainClaudeHost && url.pathname.startsWith("/__codex_plus/")) {
    if (
      url.pathname !== LOCALIZATION_SHIM_PATH ||
      !localizationShim ||
      !["GET", "HEAD"].includes(req.method)
    ) {
      res.writeHead(404, { "cache-control": "no-store", "x-content-type-options": "nosniff" });
      return res.end();
    }
    const headers = {
      "content-type": "text/javascript; charset=utf-8",
      "content-length": localizationShim.length,
      "cache-control": "no-store",
      "x-content-type-options": "nosniff",
    };
    res.writeHead(200, headers);
    return res.end(req.method === "HEAD" ? undefined : localizationShim);
  }

  const cdnI18nPrefix = "/claude-ai/v2/i18n/";
  const localI18nPath = isMainClaudeHost
    ? url.pathname
    : hostName === "assets-proxy.anthropic.com" && url.pathname.startsWith(cdnI18nPrefix)
      ? url.pathname.slice("/claude-ai/v2".length)
      : null;
  if (
    runtime.managed &&
    runtime.localization &&
    localI18nPath?.startsWith("/i18n/") &&
    ["GET", "HEAD"].includes(req.method)
  ) {
    const body = localI18n(localI18nPath);
    const headers = {
      "content-type": "application/json; charset=utf-8",
      "content-length": body.length,
      "cache-control": "no-store",
      "x-content-type-options": "nosniff",
    };
    if (hostName === "assets-proxy.anthropic.com") headers["access-control-allow-origin"] = "*";
    res.writeHead(200, headers);
    return res.end(req.method === "HEAD" ? undefined : body);
  }

  const officialResourcePath = (() => {
    if (!dataManaged || !["GET", "HEAD"].includes(req.method)) return null;
    const mapI18nPath = (pathname) => {
      const match = pathname.match(/^\/i18n\/(?:(s|dynamic)\/)?([a-zA-Z-]+)\.json$/);
      if (!match) return pathname;
      if (runtime.localization) return null;
      return `/i18n/${match[1] === "dynamic" ? "dynamic/" : ""}${match[2]}.json`;
    };
    if (isMainClaudeHost && (
      url.pathname.startsWith("/assets/") ||
      url.pathname.startsWith("/audio/") ||
      url.pathname.startsWith("/i18n/") && !runtime.localization ||
      url.pathname.startsWith("/images/") ||
      ["/client-shell-v1.html", "/favicon-internal.ico", "/favicon-staging.ico", "/favicon.ico",
        "/frame-connect.html", "/frame-manage.html", "/frame-shell.html", "/scc.json",
        "/third-party-notices.txt"].includes(url.pathname)
    )) return mapI18nPath(url.pathname);
    const cdnPrefix = "/claude-ai/v2/";
    if (hostName === "assets-proxy.anthropic.com" && url.pathname.startsWith(cdnPrefix)) {
      const pathname = `/${url.pathname.slice(cdnPrefix.length)}`;
      if (
        pathname.startsWith("/assets/") ||
        pathname.startsWith("/audio/") ||
        pathname.startsWith("/i18n/") && !runtime.localization ||
        pathname.startsWith("/images/") ||
        ["/client-shell-v1.html", "/favicon-internal.ico", "/favicon-staging.ico", "/favicon.ico",
          "/frame-connect.html", "/frame-manage.html", "/frame-shell.html", "/scc.json",
          "/third-party-notices.txt"].includes(pathname)
      ) return mapI18nPath(pathname);
    }
    return null;
  })();
  if (officialResourcePath) {
    const resource = officialAssets.read(officialResourcePath);
    if (!resource) {
      if (hostName !== "assets-proxy.anthropic.com") {
        res.writeHead(404, { "cache-control": "no-store" });
        return res.end();
      }
    } else {
      const headers = {
        "content-type": resource.contentType,
        "content-length": resource.body.length,
        "cache-control": resource.isHtml ? "no-cache" : "public, max-age=31536000, immutable",
        "x-content-type-options": "nosniff",
      };
      if (hostName === "assets-proxy.anthropic.com") headers["access-control-allow-origin"] = "*";
      if (req.method === "HEAD") {
        res.writeHead(200, headers);
        return res.end();
      }
      res.writeHead(200, headers);
      return res.end(resource.body);
    }
  }

  // Fresh受管profile直接使用安装包内同一版本的官方SPA和静态资源，不依赖旧HTML录制缓存。
  if (
    dataManaged &&
    MODE === "mock" &&
    isMainClaudeHost &&
    req.method === "GET" &&
    !path.extname(url.pathname) &&
    !/^\/(?:api|edge-api|v1|claudeai-rpc)(?:\/|$)/.test(url.pathname)
  ) {
    res.writeHead(200, {
      "content-type": "text/html; charset=utf-8",
      "content-length": Buffer.byteLength(officialShell),
      "cache-control": "no-cache",
    });
    return res.end(officialShell);
  }

  if (
    MODE === "mock" && runtime.voiceInput &&
    req.method === "GET" &&
    url.hostname.toLowerCase() === "assets-proxy.anthropic.com" &&
    (
      url.pathname === "/claude-ai/v2/assets/v1/c342ae4ef-ROR3Yba4.js" ||
      startupAssets.some((asset) => asset.pathname === url.pathname)
    ) &&
    !url.search
  ) {
    const cacheKey = `${url.host}${url.pathname}`;
    const cachedPath = cachePathFor(cacheKey);
    const metaPath = `${cachedPath}.json`;
    if (fs.existsSync(cachedPath) && fs.existsSync(metaPath)) {
      const meta = JSON.parse(fs.readFileSync(metaPath, "utf8"));
      const body = fs.readFileSync(cachedPath);
      const patched = url.pathname === "/claude-ai/v2/assets/v1/c342ae4ef-ROR3Yba4.js"
        ? body.includes(Buffer.from("D=125e3")) && !body.includes(Buffer.from("D=2e3"))
        : isPatchedStartupAsset(url.pathname, body);
      if (
        meta.status === 200 &&
        meta.url === cacheKey &&
        typeof meta.headers?.["content-type"] === "string" &&
        !meta.headers?.["content-encoding"] &&
        patched
      ) {
        res.writeHead(200, {
          ...meta.headers,
          // 主界面从 claude.ai 跨域导入 CDN 模块，本地副本也必须保留公开资源的 CORS 契约。
          "access-control-allow-origin": "*",
          "content-length": body.length,
          "cache-control": "no-store",
        });
        log(`LOCAL DICTATION ASSET CACHE HIT ${url.pathname}`);
        return res.end(body);
      }
    }
  }

  // 只有主站 claude.ai/claude.com 本身走 mock；其余子域（downloads/assets-proxy 等）
  // 和其他 MITM 域一样直通真 host —— 否则 downloads.claude.ai 会被改写成 claude.ai 而 404。
  if (!/^(claude\.ai|www\.claude\.ai|claude\.com|www\.claude\.com)$/i.test(host)) {
    if (serveCachedStaticAsset(req, res, url)) return;
    // api.anthropic.com 的 OAuth 换票链本地化：authorize 回 redirect_uri(带 code)，token 回凭据。
    // 真服务器会拒掉我们种的假 sessionKey，导致 oauth-v2 fresh exchange 失败、
    // 客户端判 session_stale_relogin → "Couldn't verify your sign-in"。
    if (MODE !== "passthrough" && /(^|\.)anthropic\.com$/i.test(host)) {
      // GET 探测：会话是否对 elevate grant 过期 → false 即新鲜
      if (req.method === "GET" && /^\/v1\/oauth\/[^/]+\/authorize$/.test(url.pathname)) {
        return json(res, { session_stale_for_elevated_grant: false });
      }
      if (req.method === "POST" && /^\/(v1\/)?oauth\/([^/]+\/)?authorize$/.test(url.pathname)) {
        return readBody(req, (raw) => {
          const authCode = "local-auth-code-" + crypto.randomUUID().slice(0, 8);
          try {
            const j = JSON.parse(raw.toString("utf8"));
            const rt = String(j.redirect_uri || "claude://oauth/callback");
            const sep = rt.includes("?") ? "&" : "?";
            const loc = `${rt}${sep}code=${authCode}${j.state ? `&state=${encodeURIComponent(j.state)}` : ""}`;
            return json(res, { redirect_uri: loc, code: authCode, state: j.state });
          } catch {
            return json(res, { redirect_uri: `claude://oauth/callback?code=${authCode}`, code: authCode });
          }
        });
      }
      if (req.method === "POST" && url.pathname === "/v1/oauth/token") {
        return readBody(req, (raw) => {
          let scope;
          try { scope = JSON.parse(raw.toString("utf8")).scope; } catch {}
          return json(res, {
            access_token: "local-access-" + crypto.randomUUID(),
            refresh_token: "local-refresh-" + crypto.randomUUID(),
            token_type: "Bearer",
            expires_in: 28800,
            ...(scope ? { scope } : {}),
          });
        });
      }
      // oauth profile：authorize 后的身份确认
      if (req.method === "GET" && url.pathname === "/api/oauth/profile") {
        return json(res, {
          account: { uuid: ACCOUNT.uuid, email_address: ACCOUNT.email, full_name: ACCOUNT.name },
          organization: { uuid: ORG.uuid, name: ORG.name },
        });
      }
      // 远端托管策略：本地空放行（不挡插件/连接器）。
      // CLI 比 Desktop 校验更严格，settings 响应必须包含 uuid/checksum/settings。
      // policy_limits → {restrictions:[],compliance_taints:[],defaults:{}}（BymwSUZ2 校验器）
      if (/^\/api\/claude_code\/settings/.test(url.pathname)) {
        const body = JSON.stringify({ uuid: ORG.uuid, settings: {}, content: { entries: {} }, checksum: "local-1" });
        res.writeHead(200, {
          "content-type": "application/json; charset=utf-8",
          "content-length": Buffer.byteLength(body),
          etag: '"local-1"',
          "cache-control": "no-store",
        });
        return res.end(body);
      }
      if (/^\/api\/claude_code\/policy_limits/.test(url.pathname)) {
        return json(res, { restrictions: [], compliance_taints: [], defaults: {}, artifact_create_notice: null });
      }
      // token 计数：网关不一定实现，本地估算
      if (req.method === "POST" && url.pathname === "/v1/messages/count_tokens") {
        return readBody(req, (raw) =>
          json(res, { input_tokens: Math.max(1, Math.ceil(raw.length / 4)), context_management: null }),
        );
      }
      if (req.method === "GET" && /^\/v1\/models/.test(url.pathname)) {
        return json(res, modelList(runtime));
      }
      // 用户自定义 API 网关（reclaude2api / ccbobi.com 等）：桌面端会多发 diagnostics
      // 等字段，严格 schema 的网关会 400。剥掉已知不兼容字段后转发到真 host。
      // 真正的模型调用：转发到本地 Anthropic 兼容网关（流式原样透传）
      if (req.method === "POST" && url.pathname === "/v1/messages") {
        return readBody(req, (raw) => forwardGateway(req, res, url.pathname + url.search, raw));
      }
    }
    // 非主站域（downloads/assets-proxy 等）：统一走 proxyUpstream，
    // 保留原始主机、写穿缓存、UPSTREAM_MAP 可重定向到本地 fixture。
    return proxyUpstream(req, res, url);
  }

  if (url.pathname.startsWith("/edge-api/")) return mockEdgeApi(req, res, url);

  // i18n：复用仓库与本机录制词表，短 ID 和动态词表按各自契约返回。
  if (MODE !== "passthrough" && runtime.localization && url.pathname.startsWith("/i18n/")) {
    const hit = localI18n(url.pathname);
    if (hit) {
      res.writeHead(200, {
        "content-type": "application/json; charset=utf-8",
        "content-length": hit.length,
        "cache-control": "no-store",
      });
      return res.end(hit);
    }
    return json(res, {});
  }

  // claude.ai 上的托管设置/policy 端点（同 api.anthropic.com 的 mock，schema 照解包校验器）
  if (MODE !== "passthrough" && /^\/api\/claude_code\/settings/.test(url.pathname)) {
    const body = JSON.stringify({ uuid: ORG.uuid, settings: {}, content: { entries: {} }, checksum: "local-1" });
    res.writeHead(200, {
      "content-type": "application/json; charset=utf-8",
      "content-length": Buffer.byteLength(body),
      etag: '"local-1"',
      "cache-control": "no-store",
    });
    return res.end(body);
  }
  if (MODE !== "passthrough" && /^\/api\/claude_code\/policy_limits/.test(url.pathname)) {
    return json(res, { restrictions: [], compliance_taints: [], defaults: {}, artifact_create_notice: null });
  }
  if (MODE !== "passthrough" && /^\/api\/claude_cli\/bootstrap/.test(url.pathname)) {
    return json(res, { client_data: {}, settings: {}, config: {}, enabled: true, environments: [], sessions: [], plugins: [], items: [], data: [] });
  }
  if (MODE !== "passthrough" && /^\/api\/claude_code\//.test(url.pathname)) {
    const body = JSON.stringify({ uuid: ORG.uuid, settings: {}, content: { entries: {} }, checksum: "local-1" });
    res.writeHead(200, { "content-type": "application/json; charset=utf-8", "content-length": Buffer.byteLength(body), etag: '"local-1"', "cache-control": "no-store" });
    return res.end(body);
  }

  // 登录页外壳：直接回放录制到的 HTML（200，而不是上游的 302）。
  // 仅 mock 模式；demo 模式的 / 走下面的演示页。
  // / 和 /login 直接跳 /new：回登出态壳会让 app 跑登录/onboarding 流程（join/org-discovery）
  if (MODE === "mock" && ["/", "/login", "/app-unavailable-in-region"].includes(url.pathname)) {
    res.writeHead(302, { location: "/new", "content-length": 0 });
    return res.end();
  }
  if (MODE === "mock" && url.pathname === "/new") {
    const key = crypto.createHash("sha1").update("claude.ai/login").digest("hex");
    const file = path.join(READ_CACHE, key);
    if (fs.existsSync(file)) {
      const body = fs.readFileSync(file);
      res.writeHead(200, {
        "content-type": "text/html; charset=utf-8",
        "content-length": body.length,
        "set-cookie": activeOrgCookies(),
      });
      return res.end(body);
    }
  }

  // ── claudeai-rpc：ConversationService 全部走 conversation-service.mjs ──
  if (url.pathname.startsWith("/claudeai-rpc/")) {
    const method = url.pathname.split("/").pop();
    const raw = [];
    req.on("data", (c) => raw.push(c));
    req.on("end", () => {
      conversationService
        .handle(method, req, res, Buffer.concat(raw))
        .catch((e) => {
          log(`RPC ${method} error${runtime.managed ? "" : `: ${e.message}`}`);
          try {
            if (!res.headersSent) res.writeHead(500, { "content-type": "text/plain" });
            res.end(`rpc error: ${e.message}\n`);
          } catch {}
        });
    });
    return;
  }

  const isApi = url.pathname.startsWith("/api/");
  if (isApi) {
    if (MODE === "demo" || MODE === "mock") return mockApi(req, res, url);
  }
  if (MODE === "demo" && (url.pathname === "/" || url.pathname === "/login")) {
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    return res.end(demoHtml);
  }
  if (url.pathname === "/manifest.json") {
    return json(res, { name: "Claude", short_name: "Claude", start_url: "/", display: "standalone", icons: [] });
  }

  // mock 模式：静态资源优先走本地缓存，避免被上游按地区 302
  if (MODE !== "passthrough" && !dataManaged && req.method === "GET") {
    const key = `${host}${url.pathname}${url.search}`;
    const cp = cachePathFor(key);
    const docNav = isDocNav(req, url);
    if (fs.existsSync(cp) && fs.existsSync(`${cp}.json`)) {
      const meta = JSON.parse(fs.readFileSync(`${cp}.json`, "utf8"));
      const body = fs.readFileSync(cp);
      // 文档导航只允许真外壳通过；桩体/重定向（登出落地页）改走外壳兜底
      if (!docNav || (meta.status === 200 && body.length >= 4096)) {
        res.writeHead(meta.status && meta.status < 400 ? 200 : meta.status, {
          ...meta.headers,
          "content-length": body.length,
          "cache-control": "no-store",
        });
        return res.end(body);
      }
    }
    if (docNav) return serveDocShell(req, res, url);
  }

  // mock 模式：接口路径回 {}，HTML 路由回 SPA 外壳
  if (MODE === "mock" && !path.extname(url.pathname)) {
    if (url.pathname.startsWith("/api/") || url.pathname.startsWith("/v1/")) return json(res, {});
    return serveDocShell(req, res, url);
  }
  if (MODE === "passthrough" || MODE === "mock") return proxyUpstream(req, res, url);
  res.writeHead(404, { "content-type": "text/plain" });
  res.end("not found (demo mode)\n");
}

// 内层 HTTP server：处理解密后的请求
const inner = http.createServer((req, res) => handle(req, res, req.headers.host));

let activeDictationWsConnections = 0;

function rejectUpgrade(socket, status, reason) {
  if (socket.destroyed) return;
  const body = Buffer.from(`${reason}\n`);
  socket.end(
    `HTTP/1.1 ${status}\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: ${body.length}\r\n\r\n${body.toString("utf8")}`,
  );
}

function isClaudeHostname(value) {
  if (typeof value !== "string") return false;
  let parsed;
  try {
    parsed = new URL(`https://${value}`);
  } catch {
    return false;
  }
  return !parsed.username && !parsed.password &&
    (!parsed.port || parsed.port === "443") &&
    DICTATION_ALLOWED_HOSTS.has(parsed.hostname.toLowerCase());
}

function dictationLanguage(requestUrl) {
  let parsed;
  try {
    parsed = new URL(requestUrl, "https://claude.ai");
  } catch {
    return null;
  }
  if (parsed.pathname !== DICTATION_WS_PATH || parsed.search.length > 2048) return null;
  const values = parsed.searchParams.getAll("language");
  if (values.length > 1) return null;
  const raw = (values[0] || "auto").trim().toLowerCase();
  const language = raw.split(/[-_]/, 1)[0];
  return ["auto", "zh", "en", "yue", "ja", "ko"].includes(language) ? language : null;
}

function readDictationReceipt() {
  const receiptPath = path.join(APP_STATE_DIR, "dictation-helper.json");
  try {
    const metadata = fs.lstatSync(receiptPath);
    if (!metadata.isFile() || metadata.isSymbolicLink() || (metadata.mode & 0o077) !== 0) return null;
    if (typeof process.getuid === "function" && metadata.uid !== process.getuid()) return null;
    const receipt = JSON.parse(fs.readFileSync(receiptPath, "utf8"));
    if (
      !Number.isInteger(receipt.port) || receipt.port < 1 || receipt.port > 65535 ||
      typeof receipt.token !== "string" ||
      !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(receipt.token)
    ) return null;
    return receipt;
  } catch {
    return null;
  }
}

function expectedWebSocketAccept(key) {
  return crypto.createHash("sha1")
    .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
    .digest("base64");
}

function validWebSocketKey(key) {
  if (typeof key !== "string") return false;
  const decoded = Buffer.from(key, "base64");
  return decoded.length === 16 && decoded.toString("base64") === key;
}

function responseUpgradeHeaders(response) {
  let raw = `HTTP/${response.httpVersion} ${response.statusCode} ${response.statusMessage || ""}\r\n`;
  for (let index = 0; index < response.rawHeaders.length; index += 2) {
    raw += `${response.rawHeaders[index]}: ${response.rawHeaders[index + 1]}\r\n`;
  }
  return `${raw}\r\n`;
}

function handleDictationUpgrade(req, clientSocket, head) {
  const language = dictationLanguage(req.url || "");
  if (language === null) return rejectUpgrade(clientSocket, "404 Not Found", "not found");
  if (MODE !== "mock" || !runtime.voiceInput) return rejectUpgrade(clientSocket, "503 Service Unavailable", "local dictation is unavailable");
  noteClient();
  if (!isClaudeHostname(req.headers.host)) return rejectUpgrade(clientSocket, "403 Forbidden", "forbidden");
  if (req.headers.origin) {
    let origin;
    try {
      origin = new URL(req.headers.origin);
    } catch {
      return rejectUpgrade(clientSocket, "403 Forbidden", "forbidden");
    }
    if (
      origin.protocol !== "https:" ||
      origin.username ||
      origin.password ||
      origin.pathname !== "/" ||
      origin.search ||
      origin.hash ||
      !DICTATION_ALLOWED_ORIGINS.has(origin.origin)
    ) {
      return rejectUpgrade(clientSocket, "403 Forbidden", "forbidden");
    }
  }
  const connection = String(req.headers.connection || "").toLowerCase().split(",").map((part) => part.trim());
  const key = req.headers["sec-websocket-key"];
  if (
    req.method !== "GET" ||
    String(req.headers.upgrade || "").toLowerCase() !== "websocket" ||
    !connection.includes("upgrade") ||
    req.headers["sec-websocket-version"] !== "13" ||
    !validWebSocketKey(key) ||
    head.length > 64 * 1024
  ) return rejectUpgrade(clientSocket, "400 Bad Request", "invalid websocket request");
  if (activeDictationWsConnections >= MAX_DICTATION_WS_CONNECTIONS) {
    return rejectUpgrade(clientSocket, "503 Service Unavailable", "local dictation is busy");
  }
  const receipt = readDictationReceipt();
  if (!receipt) return rejectUpgrade(clientSocket, "503 Service Unavailable", "local dictation is unavailable");

  activeDictationWsConnections += 1;
  let finished = false;
  let transferredBytes = head.length;
  let idleTimer;
  let durationTimer;
  const finish = () => {
    if (finished) return;
    finished = true;
    clearTimeout(idleTimer);
    clearTimeout(durationTimer);
    activeDictationWsConnections = Math.max(0, activeDictationWsConnections - 1);
  };
  const request = http.request({
    hostname: "127.0.0.1",
    port: receipt.port,
    method: "GET",
    path: `${DICTATION_HELPER_PATH}?language=${encodeURIComponent(language)}`,
    agent: false,
    headers: {
      host: `127.0.0.1:${receipt.port}`,
      connection: "Upgrade",
      upgrade: "websocket",
      "sec-websocket-key": key,
      "sec-websocket-version": "13",
      "x-codex-plus-dictation-token": receipt.token,
    },
  });
  clientSocket.pause();
  clientSocket.once("close", () => {
    finish();
    request.destroy();
  });
  const fail = () => {
    finish();
    if (!clientSocket.destroyed) rejectUpgrade(clientSocket, "503 Service Unavailable", "local dictation is unavailable");
  };
  request.setTimeout(10000, () => request.destroy());
  request.on("upgrade", (response, helperSocket, helperHead) => {
    request.setTimeout(0);
    if (
      response.statusCode !== 101 ||
      String(response.headers.upgrade || "").toLowerCase() !== "websocket" ||
      response.headers["sec-websocket-accept"] !== expectedWebSocketAccept(key)
    ) {
      helperSocket.destroy();
      return fail();
    }
    const closeNow = () => {
      finish();
      if (!clientSocket.destroyed) clientSocket.destroy();
      if (!helperSocket.destroyed) helperSocket.destroy();
    };
    const resetIdleTimer = () => {
      clearTimeout(idleTimer);
      idleTimer = setTimeout(closeNow, MAX_DICTATION_WS_IDLE_MS);
    };
    const accountBytes = (chunk) => {
      transferredBytes += chunk.length;
      if (transferredBytes > MAX_DICTATION_WS_BYTES) {
        closeNow();
        return;
      }
      resetIdleTimer();
    };
    transferredBytes += helperHead.length;
    if (transferredBytes > MAX_DICTATION_WS_BYTES) return closeNow();
    durationTimer = setTimeout(closeNow, MAX_DICTATION_WS_DURATION_MS);
    resetIdleTimer();
    clientSocket.on("error", closeNow);
    helperSocket.on("error", closeNow);
    clientSocket.once("close", () => {
      finish();
      helperSocket.destroy();
    });
    clientSocket.once("finish", () => {
      finish();
      helperSocket.destroy();
    });
    helperSocket.once("close", () => {
      if (!helperSocket.readableEnded) return closeNow();
      if (!clientSocket.destroyed && !clientSocket.writableEnded) clientSocket.end();
    });
    clientSocket.on("data", accountBytes);
    helperSocket.on("data", accountBytes);
    clientSocket.write(responseUpgradeHeaders(response));
    if (helperHead.length) clientSocket.write(helperHead);
    if (head.length) helperSocket.write(head);
    clientSocket.pipe(helperSocket);
    helperSocket.pipe(clientSocket);
    clientSocket.resume();
  });
  request.on("response", (response) => {
    response.resume();
    fail();
  });
  request.on("error", fail);
  request.end();
}

inner.on("upgrade", handleDictationUpgrade);

// 外层 HTTP server：CONNECT 做 TLS 终止；普通 HTTP（桌面 CLI 的 ANTHROPIC_BASE_URL 流量）
// 剥掉桌面端多发、上游网关不认的字段后转给本地 magpie。
const outer = http.createServer((req, res) => {
  const u = new URL(req.url, `http://${req.headers.host || "127.0.0.1"}`);
  if (runtime.managed && req.headers.origin !== undefined) return json(res, { error: "Browser origins are not allowed" }, 403);
  if (req.method === "GET" && u.pathname === "/__codex_claude_local/health") return json(res, health());
  if (runtime.managed) {
    const bearer = typeof req.headers.authorization === "string" && req.headers.authorization.startsWith("Bearer ")
      ? req.headers.authorization.slice(7) : "";
    const supplied = typeof req.headers["x-api-key"] === "string" ? req.headers["x-api-key"] : bearer;
    const actual = Buffer.from(supplied);
    const expected = Buffer.from(runtime.key);
    if (actual.length !== expected.length || !crypto.timingSafeEqual(actual, expected)) {
      return json(res, { error: "Local gateway authorization required" }, 403);
    }
  }
  noteClient();
  const fwd = (body) => forwardGateway(req, res, u.pathname + u.search, body);
  if (req.method === "GET" && u.pathname === "/v1/models") return json(res, modelList(runtime));
  if (req.method === "POST" && u.pathname === "/v1/messages") {
    return readBody(req, fwd);
  }
  // 会话 apiHost=127.0.0.1:8899 时，托管设置/policy 也会打到这里：本地 mock，别转 magpie
  if (/^\/api\/claude_cli\/bootstrap/.test(u.pathname)) {
    return json(res, { client_data: {}, settings: {}, config: {}, enabled: true, environments: [], sessions: [], plugins: [], items: [], data: [] });
  }
  if (/^\/api\/claude_code\/policy_limits/.test(u.pathname)) {
    return json(res, { restrictions: [], compliance_taints: [], defaults: {}, artifact_create_notice: null });
  }
  if (/^\/api\/claude_code\//.test(u.pathname)) {
    const body = JSON.stringify({ uuid: ORG.uuid, settings: {}, content: { entries: {} }, checksum: "local-1" });
    res.writeHead(200, { "content-type": "application/json; charset=utf-8", "content-length": Buffer.byteLength(body), etag: '"local-1"', "cache-control": "no-store" });
    return res.end(body);
  }
  if (req.method === "POST" && u.pathname === "/v1/messages/count_tokens") {
    return readBody(req, (raw) => json(res, { input_tokens: Math.max(1, Math.ceil(raw.length / 4)), context_management: null }));
  }
  // 其余 HTTP 请求（GET/POST 等）原样转发 magpie
  return readBody(req, fwd);
});

const MITM_HOSTS = runtime.managed
  ? /(^|\.)(claude\.ai|claude\.com|anthropic\.com)$/i
  : /(^|\.)(claude\.ai|claude\.com|anthropic\.com|ccbobi\.com)$/i;

outer.on("connect", (req, clientSocket, head) => {
  noteClient();
  clientSocket.on("error", () => {});
  const [host, port = "443"] = req.url.split(":");
  // 非 claude 域名：TCP 直通，不终止 TLS（保证 CDN / 其他服务不受影响）
  if (!MITM_HOSTS.test(host)) {
    log(`PASS-THROUGH CONNECT ${host}:${port}`);
    const upstream = net.connect(Number(port), host, () => {
      clientSocket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
      if (head?.length) upstream.write(head);
      upstream.pipe(clientSocket);
      clientSocket.pipe(upstream);
    });
    upstream.on("error", () => clientSocket.destroy());
    return;
  }
  clientSocket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
  const tlsSocket = new tls.TLSSocket(clientSocket, {
    isServer: true,
    secureContext: ctx,
    requestCert: false,
  });
  tlsSocket.on("error", () => {});
  if (head?.length) tlsSocket.unshift(head);
  inner.emit("connection", tlsSocket);
});

outer.listen(PORT, "127.0.0.1", () => {
  listeningPort = outer.address().port;
  writeReady();
  console.log(`claude-local proxy: 127.0.0.1:${listeningPort}  mode=${MODE}`);
});

const clientSockets = new Set();
outer.on("connection", (socket) => {
  clientSockets.add(socket);
  socket.once("close", () => clientSockets.delete(socket));
});
let stopping = false;
function shutdown() {
  if (stopping) return;
  stopping = true;
  clearTimeout(readyWriteTimer);
  stateStore.flush();
  outer.close(() => process.exit(0));
  for (const socket of clientSockets) socket.destroy();
  setTimeout(() => process.exit(0), 1000).unref();
}
process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
if (runtime.managed) {
  process.stdin.on("end", shutdown);
  process.stdin.resume();
}
