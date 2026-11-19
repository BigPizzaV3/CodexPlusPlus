#!/usr/bin/env node
// 检查 / 升级 codex-plus-core 钉住的 claude-local（ClaudeMom 仓库里 Claude Desktop 完整功能模式的核心）。
//
// claude-local 以 git 依赖按 rev 钉在 crates/codex-plus-core/Cargo.toml 里，上游更新后这里不会自己跟上。
// 本脚本拿上游 main 的最新提交和钉住的 rev 比，只看改到 crates/claude-local 的提交
// （ClaudeMom 自己的程序、文档的提交和这里无关）。
//
// 用法：
//   node scripts/bump-claude-local.mjs           # 落后就改 Cargo.toml 的 rev 并更新 Cargo.lock，列出带进来的提交
//   node scripts/bump-claude-local.mjs --check   # 只检查：落后时列出提交并以 1 退出（定时任务用）
//
// 升级之后照常跑 cargo test、构建启动，再提交。
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifest = path.join(root, "crates", "codex-plus-core", "Cargo.toml");
const BRANCH = "main";
const CRATE_PATH = "crates/claude-local";

const check = process.argv.includes("--check");
const git = (args) => execFileSync("git", args, { encoding: "utf8" }).trim();

const text = readFileSync(manifest, "utf8");
const pin = /^claude-local = \{ git = "([^"]+)", rev = "([0-9a-f]{40})" \}$/m.exec(text);
if (!pin) {
  console.error(`${path.relative(root, manifest)} 里没有找到按 rev 钉住的 claude-local 依赖。`);
  process.exit(2);
}
const [line, url, current] = pin;

const head = git(["ls-remote", url, `refs/heads/${BRANCH}`]).split(/\s+/)[0];
if (!/^[0-9a-f]{40}$/.test(head)) {
  console.error(`读不到 ${url} 的 ${BRANCH} 分支。`);
  process.exit(2);
}
if (head === current) {
  console.log(`claude-local 已是上游 ${BRANCH} 的最新提交 ${current.slice(0, 7)}。`);
  process.exit(0);
}

// 在临时裸仓库里取两个提交之间的历史（不取文件内容），列出改到 claude-local 的提交。
const scratch = mkdtempSync(path.join(tmpdir(), "claude-local-bump-"));
let commits;
let onBranch = true;
try {
  git(["init", "--bare", "--quiet", scratch]);
  git(["-C", scratch, "remote", "add", "origin", url]);
  git(["-C", scratch, "fetch", "--quiet", "--no-tags", "--filter=blob:none", "origin", head, current]);
  commits = git([
    "-C", scratch, "log", "--reverse", "--date=short", "--format=%h %ad %s", `${current}..${head}`, "--", CRATE_PATH,
  ]).split("\n").filter(Boolean);
  try {
    git(["-C", scratch, "merge-base", "--is-ancestor", current, head]);
  } catch {
    onBranch = false;
  }
} finally {
  rmSync(scratch, { recursive: true, force: true });
}

if (!onBranch) {
  console.log(`注意：现在钉住的 ${current.slice(0, 7)} 不在上游 ${BRANCH} 的历史里（可能来自某个分支），升级会换到 ${BRANCH}。`);
}
if (commits.length === 0) {
  console.log(`上游 ${BRANCH} 到了 ${head.slice(0, 7)}，但没有改到 ${CRATE_PATH}，不需要升级。`);
  process.exit(0);
}

const listing = commits.map((commit) => `  ${commit}`).join("\n");
if (check) {
  console.log(`claude-local 落后上游 ${BRANCH} ${commits.length} 个提交（${current.slice(0, 7)} → ${head.slice(0, 7)}）：\n${listing}`);
  console.log("\n运行 node scripts/bump-claude-local.mjs 升级。");
  process.exit(1);
}

writeFileSync(manifest, text.replace(line, line.replace(current, head)));
// `--workspace` 只让 Cargo.lock 跟上清单的变化；`-p claude-local` 会把它的间接依赖也重选一遍，带出无关改动。
execFileSync("cargo", ["update", "--workspace"], {
  cwd: root,
  stdio: "inherit",
  env: { ...process.env, PATH: `${process.env.HOME}/.cargo/bin${path.delimiter}${process.env.PATH}` },
});
console.log(`\nclaude-local：${current.slice(0, 7)} → ${head.slice(0, 7)}，带进 ${commits.length} 个提交：\n${listing}`);
console.log("\n接着跑 cargo test、构建启动，确认没问题再提交。");
