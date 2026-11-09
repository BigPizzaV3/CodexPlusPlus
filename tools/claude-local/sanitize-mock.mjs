#!/usr/bin/env node
// 从本机录制的 mock/ 数据生成可提交的“模板”副本（mock-template/）：
// 保留 GrowthBook/statsig/模型选择器/计划与能力形状，替换所有个人字段为合成常量。
// 用法：node sanitize-mock.mjs [srcDir=mock] [outDir=mock-template]
import fs from "node:fs";
import path from "node:path";

const SRC = process.argv[2] ?? path.join(path.dirname(new URL(import.meta.url).pathname), "mock");
const OUT = process.argv[3] ?? path.join(path.dirname(new URL(import.meta.url).pathname), "mock-template");

const ACCOUNT_UUID = "c46bd1eb-fbbf-4e5a-9bb3-5ebe8521eb54";
const ORG_UUID = "e0e877d3-8475-44a1-a501-4fc431b085ab";
const FIXED_TS = "2026-01-01T00:00:00.000000Z";

// 视为个人字段的键名 → 替换值（按名字段优先，再按键名）。
const PERSONAL_KEYS = {
  email_address: "local@claude.local",
  email: "local@claude.local",
  full_name: "Local User",
  display_name: "Local User",
  first_name: "Local",
  last_name: "User",
  phone_number: null,
  phone: null,
  avatar_url: null,
  profile_picture_url: null,
  intercom_account_hash: null,
  intercom_user_jwt: null,
  sessionKey: null,
  token: null,
  invite_token: null,
  stripe_customer_uuid: null,
  stripe_subscription_id: null,
  billing_name: null,
  billing_email: null,
  location: null,
  ip_address: null,
  user_agent: null,
  locale: undefined, // 保留由调用方决定
};

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function scrub(value, keyPath) {
  if (Array.isArray(value)) return value.map((item) => scrub(item, keyPath));
  if (value === null || typeof value !== "object") {
    return value;
  }
  const out = {};
  for (const [key, val] of Object.entries(value)) {
    const sub = `${keyPath}/${key}`;
    if (key === "locale") { out[key] = val; continue; } // locale 由运行时代码决定
    // uuid 映射：只替换用户/组织归属可判定的字段；membership 自身的 uuid 保留
    // 源值（源数据本身就是合成账号的占位 uuid，换掉会破坏 account↔org 解析）。
    if (/organization_uuid$/.test(key) || key === "parent_organization_uuid") {
      if (typeof val === "string" && UUID_RE.test(val)) { out[key] = ORG_UUID; continue; }
    }
    if (key === "user_uuid" || key === "userId") {
      if (typeof val === "string" && UUID_RE.test(val)) { out[key] = ACCOUNT_UUID; continue; }
    }
    if (key === "uuid" && /^\/account$/.test(keyPath)) {
      if (typeof val === "string" && UUID_RE.test(val)) { out[key] = ACCOUNT_UUID; continue; }
    }
    if (key === "checksum") { out[key] = null; continue; }
    if ((key === "created_at" || key === "updated_at" || key === "lastModified" || key === "last_used_at") && typeof val === "string") {
      out[key] = FIXED_TS;
      continue;
    }
    if (key in PERSONAL_KEYS) { out[key] = PERSONAL_KEYS[key]; continue; }
    if (typeof val === "string" && EMAIL_RE.test(val) && !val.endsWith("@claude.local")) { out[key] = "local@claude.local"; continue; }
    out[key] = scrub(val, sub);
  }
  return out;
}

function sanitizeFile(file, rel) {
  const data = JSON.parse(fs.readFileSync(file, "utf8"));
  let out = scrub(data, rel === "bootstrap.json" ? "" : `api/${rel}`);
  if (rel === "bootstrap.json") {
    // 账号/组织字段直接替换为合成常量
    if (out.account) {
      out.account = { ...out.account, uuid: ACCOUNT_UUID, email_address: "local@claude.local", full_name: "Local User", display_name: "Local User" };
    }
    if (out.activeOrganization) {
      out.activeOrganization = { ...out.activeOrganization, uuid: ORG_UUID, id: ORG_UUID, name: "Local" };
    }
  }
  // 会话列表只保留结构，条目是用户数据 → 清空
  if (rel === "api/conversations.json" && Array.isArray(out.data)) out.data = [];
  return out;
}

function run() {
  const jobs = ["bootstrap.json"];
  const apiDir = path.join(SRC, "api");
  if (fs.existsSync(apiDir)) {
    for (const f of fs.readdirSync(apiDir).filter((f) => f.endsWith(".json"))) jobs.push(`api/${f}`);
  }
  fs.rmSync(OUT, { recursive: true, force: true });
  for (const rel of jobs) {
    const file = path.join(SRC, rel);
    if (!fs.existsSync(file)) continue;
    const sanitized = sanitizeFile(file, rel);
    const dest = path.join(OUT, rel);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.writeFileSync(dest, JSON.stringify(sanitized, null, 2));
  }
  console.log(`sanitized ${jobs.length} files -> ${OUT}`);
}

if (import.meta.url === `file://${process.argv[1]}`) run();
export { scrub, sanitizeFile, ACCOUNT_UUID, ORG_UUID };
