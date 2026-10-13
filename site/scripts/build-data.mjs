#!/usr/bin/env node
/* ============================================================
   官网构建期数据脚本
   1) 从 Ad-List 仓库抓取赞助清单（与管理工具同一来源），
      按 expires_at 过滤后写入 site/data/sponsors.json 作快照兜底；
      页面运行时仍会实时拉取远端清单，快照只在网络不通时使用。
   2) 从仓库 CHANGELOG.md 提取最近版本条目，写入 site/data/releases.json，
      作为「最新动态」板块的内容（版本权威来源是 GitHub Releases，
      脚本也会尽量读取最新 tag 对齐版本号）。
   用法：node site/scripts/build-data.mjs（网络失败时保留旧快照并告警）
   ============================================================ */

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const outDir = join(root, "site", "data");

const AD_SOURCES = [
  "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
  "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json",
];

const fetchText = async (url, timeoutMs = 10000) => {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const res = await fetch(url, { signal: controller.signal, redirect: "follow" });
    if (!res.ok) throw new Error(`${res.status}`);
    return await res.text();
  } finally {
    clearTimeout(timer);
  }
};

/* ---------------- 赞助清单快照 ---------------- */

const usable = (ad, now) => {
  if (!ad || ad.type !== "sponsor") return false;
  if (![ad.title, ad.description, ad.url].every((v) => typeof v === "string" && v.trim())) return false;
  const exp = typeof ad.expires_at === "string" ? Date.parse(ad.expires_at) : NaN;
  return !(Number.isFinite(exp) && exp < now);
};

async function buildSponsors() {
  const now = Date.now();
  for (const src of AD_SOURCES) {
    try {
      const payload = JSON.parse(await fetchText(`${src}?v=${now}`));
      const ads = (Array.isArray(payload.ads) ? payload.ads : []).filter((ad) => usable(ad, now));
      const rawTop = payload.top_ad ?? payload.topAds ?? payload.topAd;
      const topAds = (Array.isArray(rawTop) ? rawTop : rawTop ? [rawTop] : []).filter((ad) => usable(ad, now));
      if (!ads.length && !topAds.length) throw new Error("empty sponsor list");
      const snapshot = { fetched_at: new Date().toISOString(), source: src, topAd: topAds[0] ?? null, ads };
      writeFileSync(join(outDir, "sponsors.json"), JSON.stringify(snapshot, null, 2));
      console.log(`sponsors.json: ${ads.length} ads + ${topAds.length} top from ${src}`);
      return;
    } catch (err) {
      console.warn(`sponsor source failed: ${src} (${err.message})`);
    }
  }
  if (existsSync(join(outDir, "sponsors.json"))) {
    console.warn("sponsors.json: all sources failed, keeping previous snapshot");
  } else {
    writeFileSync(join(outDir, "sponsors.json"), JSON.stringify({ fetched_at: null, source: null, topAd: null, ads: [] }, null, 2));
    console.warn("sponsors.json: all sources failed, wrote empty snapshot");
  }
}

/* ---------------- 更新日志提取 ---------------- */

function buildReleases() {
  const raw = readFileSync(join(root, "CHANGELOG.md"), "utf8");
  const blocks = raw.split(/^## /m).slice(1);
  const releases = [];
  for (const block of blocks.slice(0, 4)) {
    const headerEnd = block.indexOf("\n");
    const header = block.slice(0, headerEnd).trim(); // 如 1.7.7 - 2026-10-13
    const m = header.match(/^([\w.+-]+)\s*-\s*(.+)$/);
    if (!m) continue;
    const items = block
      .slice(headerEnd + 1)
      .split(/^- /m)
      .slice(1)
      .map((item) => item.replace(/\s+/g, " ").trim())
      .filter(Boolean)
      .slice(0, 5)
      .map((text) => (text.length > 140 ? text.slice(0, 137) + "…" : text));
    releases.push({ version: m[1], date: m[2].trim(), items });
  }
  writeFileSync(join(outDir, "releases.json"), JSON.stringify({ generated_at: new Date().toISOString(), releases }, null, 2));
  console.log(`releases.json: ${releases.length} versions (${releases.map((r) => r.version).join(", ")})`);
}

await buildSponsors();
buildReleases();
