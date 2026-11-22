#!/usr/bin/env node
/* ============================================================
   官网构建期数据脚本
   从仓库 CHANGELOG.md 提取最近版本条目，写入 site/data/releases.json，
   作为「最新动态」板块的内容（版本权威来源是 GitHub Releases）。
   赞助商不在这里：页面运行时直接读远端清单，没有构建期快照。
   用法：node site/scripts/build-data.mjs
   ============================================================ */

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const outDir = join(root, "site", "data");

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

buildReleases();
