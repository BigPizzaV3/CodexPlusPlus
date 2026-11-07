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
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";
import { createConversationService } from "./conversation-service.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i === -1 ? fallback : args[i + 1] ?? fallback;
};

const PORT = Number(arg("port", "8899"));
const MODE = arg("mode", "mock"); // mock | passthrough | demo
const UPSTREAM = arg("upstream", "https://claude.ai");
const CA_DIR = arg("ca", path.join(here, "ca"));
const CACHE_DIR = path.join(here, ".cache");
const LOG = arg("log", "1") !== "0";
const GATEWAY_URL = process.env.CLAUDE_LOCAL_GATEWAY_URL || "http://127.0.0.1:3425";
const GATEWAY_KEY = process.env.CLAUDE_LOCAL_GATEWAY_KEY || "magpie";

fs.mkdirSync(CACHE_DIR, { recursive: true });

const key = fs.readFileSync(path.join(CA_DIR, "server.key"));
const cert = fs.readFileSync(path.join(CA_DIR, "server.crt"));
const ctx = tls.createSecureContext({ key, cert });

// ── mock 数据 ────────────────────────────────────────────────
const ACCOUNT = {
  id: "c46bd1eb-fbbf-4e5a-9bb3-5ebe8521eb54",
  uuid: "c46bd1eb-fbbf-4e5a-9bb3-5ebe8521eb54",
  email: "local@claude.local",
  name: "Local User",
  full_name: "Local User",
  plan_type: "max",
  plan: "max",
  subscription: { plan: "max", status: "active" },
  is_max: true,
  has_claude_max: true,
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
  settings: { locale: "zh-CN", theme: "system" },
};

const ORG = {
  uuid: "e0e877d3-8475-44a1-a501-4fc431b085ab",
  id: "e0e877d3-8475-44a1-a501-4fc431b085ab",
  name: "Local",
  organization_type: "personal",
  raven_type: "personal",
  plan_type: "max",
  capabilities: [
    "chat",
    "claude_pro",
    "claude_max",
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
  created_at: "2026-01-01T00:00:00Z",
};

const BOOTSTRAP = {
  account: ACCOUNT,
  user: ACCOUNT,
  organization: ORG,
  organizations: [ORG],
  memberships: [{ uuid: "11111111-2222-4333-8444-555555555556", organization: ORG, role: "admin", status: "active", seat_tier: "full_member", email_address: "local@claude.local", created_at: "2025-01-01T00:00:00Z", updated_at: "2025-01-01T00:00:00Z" }],
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

// 读取录制到的真实 bootstrap 作为模板（capture.mjs 产出）
function loadRecordedBootstrap() {
  try {
    for (const f of fs.readdirSync(CACHE_DIR)) {
      if (!f.endsWith(".json")) continue;
      const meta = JSON.parse(fs.readFileSync(path.join(CACHE_DIR, f), "utf8"));
      if (typeof meta.url === "string" && meta.url.includes("/edge-api/bootstrap")) {
        const body = fs.readFileSync(path.join(CACHE_DIR, f.replace(/\.json$/, "")));
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
  try {
    for (const f of fs.readdirSync(CACHE_DIR)) {
      if (!f.endsWith(".json")) continue;
      const meta = JSON.parse(fs.readFileSync(path.join(CACHE_DIR, f), "utf8"));
      const u = String(meta.url || "");
      const bodyFile = path.join(CACHE_DIR, f.replace(/\.json$/, ""));
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
const WEB_I18N_DIR = path.resolve(here, "../../assets/claude-localization/catalogs");
const i18nResponses = new Map();
function localI18n(pathname) {
  const match = pathname.match(/^\/i18n\/(?:(s|dynamic)\/)?([a-zA-Z-]+)\.json$/);
  if (!match) return null;
  if (i18nResponses.has(pathname)) return i18nResponses.get(pathname);
  const [, kind, locale] = match;
  const base = `${locale}.json`;
  const catalog = kind === "dynamic" ? {
    ...readI18n(path.join(WEB_I18N_DIR, "dynamic", base)),
    ...readI18n(i18nMap.get(pathname)),
  } : {
    ...readI18n(path.join(I18N_DIR, base)),
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

function json(res, obj, code = 200) {
  const body = JSON.stringify(obj);
  res.writeHead(code, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store",
  });
  res.end(body);
}

const CONV_STATE = new Map();
const NOTIFICATION_SETTINGS = new Map();
let DEVICE_ROW = null; // Code 模式注册的设备行 {id, k, ...}

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
    });
  }
  return CONV_STATE.get(uuid);
}

// claudeai-rpc 服务：StreamTimeline / PerformAction / 其余 ConversationService 方法
const conversationService = createConversationService({
  ensureConv,
  listConvs: () => [...CONV_STATE.values()],
  log,
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
  return m;
}

const FAKE_ACCOUNT = {
  uuid: ACCOUNT.id,
  id: ACCOUNT.id,
  email: ACCOUNT.email,
  name: ACCOUNT.name,
  full_name: ACCOUNT.name,
  plan_type: "max",
  plan: "max",
  is_max: true,
  has_claude_max: true,
  membership: { plan: "max", status: "active", plan_type: "max" },
  subscription: { plan: "max", status: "active" },
  organization_uuid: ORG.uuid,
  organization: ORG,
  capabilities: ACCOUNT.capabilities,
  settings: {},
};

function fakeAccount() {
  return {
    ...FAKE_ACCOUNT,
    memberships: [{ uuid: "11111111-2222-4333-8444-555555555556", organization: ORG, role: "admin", status: "active", seat_tier: "full_member", created_at: "2025-01-01T00:00:00Z" }],
    memberships_incomplete: false,
  };
}

function mockEdgeApi(req, res, url) {
  const p = url.pathname.replace(/\/+$/, "");
  if (p.includes("/edge-api/bootstrap")) {
    const fakeFile = path.join(here, "mock", "bootstrap.json");
    const body = fs.existsSync(fakeFile)
      ? fs.readFileSync(fakeFile, "utf8")
      : null;
    if (body) {
      // 解包代码显示 org 解析依赖 savedOrgCookie/org_uuid cookie，必须种上
      res.writeHead(200, {
        "content-type": "application/json; charset=utf-8",
        "content-length": Buffer.byteLength(body),
        "cache-control": "no-store",
        "set-cookie": [
          `org_uuid=${ORG.uuid}; Path=/; SameSite=Lax`,
          `active_org_uuid=${ORG.uuid}; Path=/; SameSite=Lax`,
          `last_active_org=${ORG.uuid}; Path=/; SameSite=Lax`,
          `sessionKey=${ACCOUNT.uuid}-local-session; Path=/; HttpOnly; Secure; SameSite=None`,
        ],
      });
      return res.end(body);
    }
    const base = RECORDED_BOOTSTRAP ?? {};
    return json(res, {
      ...base,
      account: fakeAccount(),
      activeOrganization: ORG,
      locale: base.locale ?? "zh-Hans",
      intercom_account_hash: null,
      intercom_user_jwt: null,
    });
  }
  if (p.endsWith("/edge-api/client-health/check")) return json(res, {});
  return json(res, {});
}

// REST 侧的会话条目：拿录制文件里的第一条当模板，叠动态字段
let REST_CONV_TEMPLATE = null;
function convToRest(c) {
  if (!REST_CONV_TEMPLATE) {
    try {
      const f = path.join(here, "mock", "api", "conversations.json");
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
    model: "claude-local",
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

  const convGet = p.match(/\/chat_conversations\/([0-9a-f-]{36})$/);
  if (convGet && method === "GET") {
    return json(res, convToRest(ensureConv(convGet[1])));
  }

  // 会话列表：静态模板 + 运行期动态会话（RPC 建的）合并，按更新时间倒序
  if (/\/chat_conversations_v2$/.test(p) && method === "GET") {
    const f = path.join(here, "mock", "api", "conversations.json");
    let base = { data: [], has_more: false };
    try { base = JSON.parse(fs.readFileSync(f, "utf8")); } catch {}
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
      const f = path.join(here, "mock", "api", `${slug}.json`);
      if (fs.existsSync(f)) {
        try { return json(res, JSON.parse(fs.readFileSync(f, "utf8"))); } catch {}
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
        value: { enabled: true, appScopedEnabled: true }, on: true, off: false, source: "force",
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
  if (p === "/v1/code/sessions/watch" || (p.startsWith("/v1/code/sessions/watch"))) {
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
      plan: { code: "claude_max", name: "Max", tier: "max" },
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
    let base = {};
    try { base = JSON.parse(fs.readFileSync(path.join(here, "mock", "bootstrap.json"), "utf8")).current_user_access ?? {}; } catch {}
    return json(res, {
      ...base,
      has_access: true,
      can_access: true,
      status: "allowed",
      is_eligible: true,
      reason: null,
      plan: "max",
      plan_type: "max",
      subscription_plan: "max",
      billing_type: "stripe_subscription",
      rate_limit_tier: "default_claude_max_20x",
      analytics_subscription_plan: "claude_max",
      organization: ORG,
      account: ACCOUNT,
      memberships: [{ uuid: "11111111-2222-4333-8444-555555555556", organization: ORG, role: "admin", status: "active", seat_tier: "full_member" }],
      membership: { organization: ORG, role: "admin", status: "active" },
      is_member: true,
      member: true,
      feature_flags: BOOTSTRAP.feature_flags,
    });
  }
  // 隐私同意（非 /api 前缀，单独兜底）
  if (p.startsWith("/v1/privacy-consents")) {
    return json(res, { consents: {}, data: [], prefix: "cookies." });
  }

  if (p === "/api/bootstrap" || p === "/api/account" || p === "/api/account_profile") {
    return json(res, BOOTSTRAP);
  }
  if (p === "/api/organizations") {
    return json(res, { organizations: [ORG], data: [ORG], items: [ORG] });
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
    const item = {
      ...ORG,
      uuid: ORG.uuid,
      id: ORG.uuid,
      organization_uuid: ORG.uuid,
      organization: ORG,
      joined: true,
      is_member: true,
      membership: { organization: ORG, role: "admin", status: "active" },
    };
    return json(res, { organizations: [item], data: [item], items: [item] });
  }
  // Join 组织确认
  if (/^\/api\/organizations\/[^/]+\/(join|memberships?)$/.test(p) && method !== "GET") {
    return json(res, { ok: true, organization: ORG, membership: { organization: ORG, role: "admin" }, joined: true });
  }
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
      const model = process.env.GATEWAY_MODEL || "claude-sonnet-4-6";
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
        messages: [{ role: "user", content: prompt }],
      });
      const greq = https.request(
        {
          host: "127.0.0.1",
          port: 3425,
          path: "/v1/messages",
          method: "POST",
          headers: {
            "content-type": "application/json",
            "x-api-key": "magpie",
            authorization: "Bearer magpie",
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
    return json(res, {
      plan: "max",
      usage: { messages_used: 0, messages_limit: null },
      rate_limit: { allowed: true },
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
  return path.join(CACHE_DIR, crypto.createHash("sha1").update(keyStr).digest("hex"));
}

function readBody(req, cb) {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => cb(Buffer.concat(chunks)));
  req.on("error", () => cb(Buffer.alloc(0)));
}

function proxyUpstream(req, res, url) {
  const cacheKey = `${url.host}${url.pathname}${url.search}`;
  const cp = cachePathFor(cacheKey);
  const metaPath = `${cp}.json`;

  const target = new URL(url.pathname + url.search, UPSTREAM);
  const headers = { ...req.headers, host: target.host };
  delete headers["proxy-connection"];
  delete headers["accept-encoding"];

  const upstreamReq = https.request(
    {
      hostname: target.hostname,
      port: 443,
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
        if (ok && req.method === "GET") {
          fs.writeFileSync(cp, body);
          fs.writeFileSync(
            metaPath,
            JSON.stringify({ status: up.statusCode, headers: up.headers, url: cacheKey }),
          );
        }
        res.writeHead(up.statusCode, { ...up.headers, "content-length": body.length });
        res.end(body);
      });
    },
  );
  upstreamReq.on("timeout", () => upstreamReq.destroy(new Error("upstream timeout")));
  upstreamReq.on("error", (err) => {
    // 上游不可达：回退缓存
    if (fs.existsSync(cp) && fs.existsSync(metaPath)) {
      const meta = JSON.parse(fs.readFileSync(metaPath, "utf8"));
      const body = fs.readFileSync(cp);
      res.writeHead(meta.status || 200, { ...meta.headers, "content-length": body.length });
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
const demoHtml = fs.readFileSync(path.join(here, "demo", "index.html"));

function handle(req, res, hostHeader) {
  const host = hostHeader || "claude.ai";
  const url = new URL(req.url, `https://${host}`);
  log(`${req.method} ${url.pathname}${url.search}`);

  // 只有主站 claude.ai/claude.com 本身走 mock；其余子域（downloads/assets-proxy 等）
  // 和其他 MITM 域一样直通真 host —— 否则 downloads.claude.ai 会被改写成 claude.ai 而 404。
  if (!/^(claude\.ai|www\.claude\.ai|claude\.com|www\.claude\.com)$/i.test(host)) {
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
        return json(res, {
          data: [
            { id: "claude-sonnet-4-6", type: "model", display_name: "Claude Sonnet 4.6", created_at: "2025-01-01T00:00:00Z" },
            { id: "claude-opus-4-6", type: "model", display_name: "Claude Opus 4.6", created_at: "2025-01-01T00:00:00Z" },
          ],
          has_more: false,
          first_id: "claude-sonnet-4-6",
          last_id: "claude-opus-4-6",
        });
      }
      // 用户自定义 API 网关（reclaude2api / ccbobi.com 等）：桌面端会多发 diagnostics
      // 等字段，严格 schema 的网关会 400。剥掉已知不兼容字段后转发到真 host。
      if (req.method === "POST" && /^\/v1\/messages/.test(url.pathname) && !/(^|\.)anthropic\.com$/i.test(host)) {
        return readBody(req, (raw) => {
          let body = raw;
          try {
            const j = JSON.parse(raw.toString("utf8"));
            delete j.diagnostics;
            delete j.metadata?.diagnostics;
            body = Buffer.from(JSON.stringify(j));
          } catch {}
          const target = new URL(url.pathname + url.search, `https://${host}`);
          const headers = { ...req.headers, host: target.host, "content-length": body.length };
          delete headers["proxy-connection"];
          delete headers["accept-encoding"];
          const up = https.request(
            { hostname: target.hostname, port: 443, path: target.pathname + target.search, method: "POST", headers, timeout: 300000 },
            (r) => { res.writeHead(r.statusCode, r.headers); r.pipe(res); },
          );
          up.on("timeout", () => up.destroy(new Error("timeout")));
          up.on("error", (e) => { res.writeHead(502); res.end(String(e.message)); });
          up.end(body);
        });
      }
      // 真正的模型调用：转发到本地 Anthropic 兼容网关（流式原样透传）
      if (req.method === "POST" && /^\/v1\/messages/.test(url.pathname)) {
        const gw = new URL(GATEWAY_URL);
        const headers = { ...req.headers, host: gw.host, "x-api-key": GATEWAY_KEY, authorization: `Bearer ${GATEWAY_KEY}` };
        delete headers["content-length"];
        delete headers["accept-encoding"];
        const up = http.request(
          { hostname: gw.hostname, port: gw.port || 80, path: url.pathname + url.search, method: "POST", headers, timeout: 300000 },
          (r) => {
            if (r.statusCode >= 400) {
              const chunks = [];
              r.on("data", (c) => chunks.push(c));
              r.on("end", () => {
                const body = Buffer.concat(chunks);
                log(`gateway ${r.statusCode} ${url.pathname}: ${body.slice(0, 300)}`);
                res.writeHead(r.statusCode, { ...r.headers, "content-length": body.length });
                res.end(body);
              });
              return;
            }
            res.writeHead(r.statusCode, r.headers);
            r.pipe(res);
          },
        );
        up.on("timeout", () => up.destroy(new Error("gateway timeout")));
        up.on("error", (e) => { res.writeHead(502, { "content-type": "application/json" }); res.end(JSON.stringify({ type: "error", error: { type: "api_error", message: `local gateway: ${e.message}` } })); });
        req.pipe(up);
        return;
      }
    }
    const target = new URL(url.pathname + url.search, `https://${host}`);
    const headers = { ...req.headers, host: target.host };
    delete headers["proxy-connection"];
    delete headers["accept-encoding"];
    const up = https.request(
      { hostname: target.hostname, port: 443, path: target.pathname + target.search, method: req.method, headers, timeout: 20000 },
      (r) => {
        const chunks = [];
        r.on("data", (c) => chunks.push(c));
        r.on("end", () => {
          const body = Buffer.concat(chunks);
          res.writeHead(r.statusCode, { ...r.headers, "content-length": body.length });
          res.end(body);
        });
      },
    );
    up.on("timeout", () => up.destroy(new Error("timeout")));
    up.on("error", (e) => { res.writeHead(502); res.end(String(e.message)); });
    req.pipe(up);
    return;
  }

  if (url.pathname.startsWith("/edge-api/")) return mockEdgeApi(req, res, url);

  // i18n：复用仓库与本机录制词表，短 ID 和动态词表按各自契约返回。
  if (MODE !== "passthrough" && url.pathname.startsWith("/i18n/")) {
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
    const file = path.join(CACHE_DIR, key);
    if (fs.existsSync(file)) {
      const body = fs.readFileSync(file);
      res.writeHead(200, {
        "content-type": "text/html; charset=utf-8",
        "content-length": body.length,
        "set-cookie": [
          `org_uuid=${ORG.uuid}; Path=/; SameSite=Lax`,
          `active_org_uuid=${ORG.uuid}; Path=/; SameSite=Lax`,
          `sessionKey=${ACCOUNT.uuid}-local-session; Path=/; HttpOnly; Secure; SameSite=None`,
        ],
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
          log(`RPC ${method} error: ${e.message}`);
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
  if (MODE !== "passthrough" && req.method === "GET") {
    const key = `${host}${url.pathname}${url.search}`;
    const cp = cachePathFor(key);
    if (fs.existsSync(cp) && fs.existsSync(`${cp}.json`)) {
      const meta = JSON.parse(fs.readFileSync(`${cp}.json`, "utf8"));
      const body = fs.readFileSync(cp);
      res.writeHead(meta.status && meta.status < 400 ? 200 : meta.status, {
        ...meta.headers,
        "content-length": body.length,
        "cache-control": "no-store",
      });
      return res.end(body);
    }
  }

  // mock 模式：接口路径回 {}，HTML 路由回 SPA 外壳
  if (MODE === "mock" && !path.extname(url.pathname)) {
    if (url.pathname.startsWith("/api/") || url.pathname.startsWith("/v1/")) return json(res, {});
    const shellKey = crypto.createHash("sha1").update("claude.ai/login").digest("hex");
    const shell = path.join(CACHE_DIR, shellKey);
    if (fs.existsSync(shell)) {
      const body = fs.readFileSync(shell);
      res.writeHead(200, { "content-type": "text/html; charset=utf-8", "content-length": body.length });
      return res.end(body);
    }
  }
  if (MODE === "passthrough" || MODE === "mock") return proxyUpstream(req, res, url);
  res.writeHead(404, { "content-type": "text/plain" });
  res.end("not found (demo mode)\n");
}

// 内层 HTTP server：处理解密后的请求
const inner = http.createServer((req, res) => handle(req, res, req.headers.host));

// 外层 HTTP server：CONNECT 做 TLS 终止；普通 HTTP（桌面 CLI 的 ANTHROPIC_BASE_URL 流量）
// 剥掉桌面端多发、上游网关不认的字段后转给本地 magpie。
const outer = http.createServer((req, res) => {
  const u = new URL(req.url, `http://${req.headers.host || "127.0.0.1"}`);
  const gw = new URL(GATEWAY_URL);
  const fwd = (body) => {
    const headers = { ...req.headers, host: gw.host, "content-length": body.length };
    delete headers["proxy-connection"];
    delete headers["accept-encoding"];
    delete headers["connection"];
    const up = http.request(
      { hostname: gw.hostname, port: gw.port || 80, path: u.pathname + u.search, method: req.method, headers, timeout: 300000 },
      (r) => { res.writeHead(r.statusCode, r.headers); r.pipe(res); },
    );
    up.on("timeout", () => up.destroy(new Error("timeout")));
    up.on("error", (e) => { res.writeHead(502); res.end(String(e.message)); });
    up.end(body);
  };
  if (req.method === "POST" && u.pathname === "/v1/messages") {
    return readBody(req, (raw) => {
      try {
        const j = JSON.parse(raw.toString("utf8"));
        delete j.diagnostics;
        if (j.metadata) delete j.metadata.diagnostics;
        return fwd(Buffer.from(JSON.stringify(j)));
      } catch {
        return fwd(raw);
      }
    });
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

const MITM_HOSTS = /(^|\.)(claude\.ai|claude\.com|anthropic\.com|ccbobi\.com)$/i;

outer.on("connect", (req, clientSocket, head) => {
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
  console.log(`claude-local proxy: 127.0.0.1:${PORT}  mode=${MODE}  upstream=${UPSTREAM}`);
  console.log(`CA: ${path.join(CA_DIR, "ca.pem")}`);
});
