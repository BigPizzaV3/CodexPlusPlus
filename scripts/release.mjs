#!/usr/bin/env node
// 一条命令发版：改版本号 → 提交 → 打 tag → 推送。推送 tag 后由 CI（Release assets）
// 从 CHANGELOG.md 截取说明建草稿 Release、构建各平台附件，附件齐了再自动发布。
//
// 用法：
//   node scripts/release.mjs 2.2.2            # 正式发版
//   node scripts/release.mjs 2.2.2 --dry-run  # 只做检查、打印将要改的内容，不写文件不推送
//
// 发版前先在 CHANGELOG.md 顶部写好「## 2.2.2 - YYYY-MM-DD」这一节（可以不提交，脚本会一起提交）。
// 版本号早已改好、只差打 tag 的情况（比如草稿一直没发）也能直接用，脚本会跳过改号。
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import path from "node:path";
import {
  VERSION_FILES,
  VERSION_PATTERN,
  changelogSection,
  compareVersions,
  readText,
  readVersions,
  root,
} from "./release-meta.mjs";

const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run");
const version = (args.find((arg) => !arg.startsWith("--")) || "").replace(/^v/i, "");
const tag = `v${version}`;

function fail(message) {
  console.error(`\n✗ ${message}`);
  process.exit(1);
}

function run(command, commandArgs, options = {}) {
  return execFileSync(command, commandArgs, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], ...options }).trim();
}

function tryRun(command, commandArgs) {
  try {
    return { ok: true, out: run(command, commandArgs) };
  } catch (error) {
    return { ok: false, out: String(error.stderr || error.message || "").trim() };
  }
}

function step(message) {
  console.log(`• ${message}`);
}

if (!VERSION_PATTERN.test(version)) {
  fail("用法：node scripts/release.mjs <版本号，如 2.2.2> [--dry-run]");
}

// 1. 工作区：必须在 main 上，除 CHANGELOG.md 外不能有未提交的改动（未跟踪文件不管）。
const branch = run("git", ["rev-parse", "--abbrev-ref", "HEAD"]);
if (branch !== "main") fail(`当前在 ${branch} 分支，发版只从 main 发`);
// -z 输出不能 trim：行首的状态列是空格，trim 掉会把文件名切掉一个字符。
const dirty = execFileSync("git", ["status", "--porcelain", "-z", "--untracked-files=no"], { cwd: root, encoding: "utf8" })
  .split("\0")
  .filter(Boolean)
  .map((entry) => entry.slice(3))
  .filter((file) => file !== "CHANGELOG.md");
if (dirty.length) fail(`有未提交的改动，先提交或还原：\n  ${dirty.join("\n  ")}`);
step("工作区干净（CHANGELOG.md 除外）");

// 2. 与远端同步：本地不能落后 origin/main，否则推送会被拒或发出旧代码。
run("git", ["fetch", "origin", "main", "--tags", "--quiet"]);
if (!tryRun("git", ["merge-base", "--is-ancestor", "origin/main", "HEAD"]).ok) {
  fail("本地 main 落后于 origin/main，先 git pull --rebase");
}
step("本地 main 不落后 origin/main");

// 3. tag 不能已存在。
if (tryRun("git", ["rev-parse", "-q", "--verify", `refs/tags/${tag}`]).ok) fail(`本地已有 tag ${tag}`);
if (run("git", ["ls-remote", "--tags", "origin", `refs/tags/${tag}`])) fail(`远端已有 tag ${tag}`);
step(`tag ${tag} 尚未使用`);

// 4. 别的版本还有没发的草稿时先停下：多半是上次发版没走完，再升一版会跳号。
const releases = tryRun("gh", ["release", "list", "--limit", "30", "--json", "tagName,isDraft"]);
if (releases.ok) {
  const otherDrafts = JSON.parse(releases.out || "[]")
    .filter((release) => release.isDraft && release.tagName !== tag)
    .map((release) => release.tagName);
  if (otherDrafts.length) {
    fail(`还有没发布的草稿 Release：${otherDrafts.join("、")}。先发布或删除它们，或者直接发那个版本号`);
  }
  step("没有其他版本的草稿 Release");
} else {
  console.warn(`! 查不到 Release 列表（gh 不可用或未登录），跳过草稿检查：${releases.out.split("\n")[0]}`);
}

// 5. CHANGELOG.md 里要有这一版的说明，CI 会原样作为 Release 说明。
const notes = changelogSection(version);
if (!notes) fail(`CHANGELOG.md 里没有「## ${version}」这一节，先写好更新日志`);
step(`CHANGELOG.md 有 ${version} 的说明（${notes.split("\n").length} 行）`);

// 6. 版本号：各文件必须读得出来且彼此一致，新版本不能比当前旧。
const current = readVersions();
const unreadable = current.filter((entry) => !entry.version);
if (unreadable.length) fail(`读不出版本号：${unreadable.map((entry) => entry.file).join("、")}`);
const distinct = [...new Set(current.map((entry) => entry.version))];
if (distinct.length > 1) {
  fail(`各文件版本号不一致，先手动对齐：\n  ${current.map((entry) => `${entry.file}: ${entry.version}`).join("\n  ")}`);
}
const from = distinct[0];
if (compareVersions(version, from) < 0) fail(`新版本 ${version} 比当前 ${from} 还旧`);

const changedFiles = [];
if (from === version) {
  step(`版本号已经是 ${version}，跳过改号`);
} else {
  for (const { file, bump, read } of VERSION_FILES) {
    const before = readText(file);
    const after = bump(before, from, version);
    if (read(after) !== version) fail(`改 ${file} 的版本号失败，检查文件格式`);
    if (!dryRun) writeFileSync(path.join(root, file), after);
    changedFiles.push(file);
  }
  step(`版本号 ${from} → ${version}：${changedFiles.length} 个文件`);
}

const changelogDirty = run("git", ["status", "--porcelain", "--untracked-files=no", "--", "CHANGELOG.md"]) !== "";
const toCommit = [...changedFiles, ...(changelogDirty ? ["CHANGELOG.md"] : [])];

if (dryRun) {
  console.log(`\n[dry-run] 将提交：${toCommit.length ? toCommit.join("、") : "（无改动，直接给 HEAD 打 tag）"}`);
  console.log(`[dry-run] 将创建并推送 tag ${tag}，CI 随后建草稿 Release、构建附件、自动发布`);
  console.log(`\n—— Release 说明预览 ——\n${notes}`);
  process.exit(0);
}

// 7. 提交、打 tag、原子推送（main 和 tag 要么都上去，要么都不上去）。
if (toCommit.length) {
  run("git", ["add", "--", ...toCommit]);
  run("git", ["commit", "-m", `release: ${tag}`], { stdio: ["ignore", "pipe", "inherit"] });
  step(`已提交 release: ${tag}`);
}
run("git", ["tag", "-a", tag, "-m", `Codex++ ${tag}`]);
step(`已打 tag ${tag} → ${run("git", ["rev-parse", "--short", "HEAD"])}`);
execFileSync("git", ["push", "--atomic", "origin", "HEAD:refs/heads/main", `refs/tags/${tag}`], { cwd: root, stdio: "inherit" });

const repo = tryRun("gh", ["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"]);
const slug = repo.ok ? repo.out : "BigPizzaV3/CodexPlusPlus";
console.log(`\n✓ 已推送 ${tag}。CI 会建草稿 Release → 构建附件 → 附件齐了自动发布（约 10 分钟）：`);
console.log(`  https://github.com/${slug}/actions/workflows/release-assets.yml`);
