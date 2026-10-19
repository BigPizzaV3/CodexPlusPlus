#!/usr/bin/env node
// 本地构建 + 重启固定开发实例 + 清理旧构建（AGENTS.md「改完即构建启动」的推荐入口）。
//
// 固定使用 dist/local-build/Codex++.app：
//   - 只替换 Contents/MacOS/CodexPlusPlus，保留 bundle 的 Info.plist / LSEnvironment / state，
//     这样每次构建都复用同一个应用身份和测试配置，不会在 Finder / Launchpad 里多出一个 Codex++。
//   - 旧的历史构建交给 scripts/prune-local-builds.mjs 移入废纸篓。
//
// 用法：
//   node scripts/relaunch-local.mjs                 # 构建前端 + cargo，替换固定 bundle，重启，清理旧构建
//   node scripts/relaunch-local.mjs --skip-build    # 跳过构建，用现有 target/release 二进制重装并重启
//   node scripts/relaunch-local.mjs --no-open       # 不自动启动（只替换 bundle）
import { execFileSync, spawn } from "node:child_process";
import { chmodSync, cpSync, existsSync, mkdirSync, renameSync, rmSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const distDir = path.join(root, "dist");
const bundleDir = path.join(distDir, "local-build");
const bundle = path.join(bundleDir, "Codex++.app");
const bundleBinary = path.join(bundle, "Contents", "MacOS", "CodexPlusPlus");
const builtBinary = path.join(root, "target", "release", "codex-plus-plus");
const stagingDir = path.join(distDir, ".local-build-staging");
const stagingBundle = path.join(stagingDir, "Codex++.app");
const pruneScript = path.join(root, "scripts", "prune-local-builds.mjs");

function localBundleProcessPattern(bundle) {
  // pgrep/pkill 使用扩展正则；Codex++.app 的加号必须按字面量匹配。
  return `${bundle}/Contents/MacOS/`.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const args = new Set(process.argv.slice(2));
const skipBuild = args.has("--skip-build");
const noOpen = args.has("--no-open");

const run = (command, argv, options = {}) => {
  console.log(`$ ${command} ${argv.join(" ")}`);
  execFileSync(command, argv, { stdio: "inherit", ...options });
};

if (!existsSync(bundle)) {
  console.error(
    `缺少固定开发实例：${bundle}\n` +
      "按 AGENTS.md「改完即构建启动」从 /Applications/Codex++.app 复制一份并补 LSEnvironment 后再试。",
  );
  process.exit(1);
}

if (!skipBuild) {
  console.log("1/4 构建前端");
  run("npm", ["run", "vite:build"], { cwd: path.join(root, "apps/codex-plus-manager") });
  console.log("2/4 构建 release 二进制");
  run(path.join(process.env.HOME ?? "", ".cargo", "bin", "cargo"), [
    "build",
    "--release",
    "-p",
    "codex-plus-manager",
  ], { cwd: root, env: { ...process.env, PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH}` } });
}

if (!existsSync(builtBinary)) {
  console.error(`找不到构建产物：${builtBinary}，先执行 cargo build --release -p codex-plus-manager`);
  process.exit(1);
}

console.log(`3/4 原地替换 ${bundle}`);
// 先在 staging 里改好再整体换目录，避免构建/签名中途失败留下半坏的 bundle。
rmSync(stagingDir, { recursive: true, force: true });
mkdirSync(stagingDir, { recursive: true });
cpSync(bundle, stagingBundle, { recursive: true, verbatimSymlinks: true });
cpSync(builtBinary, path.join(stagingBundle, "Contents", "MacOS", "CodexPlusPlus"));
chmodSync(path.join(stagingBundle, "Contents", "MacOS", "CodexPlusPlus"), 0o755);
run("codesign", ["--force", "--deep", "--sign", "-", stagingBundle]);

// 停掉固定目录里的旧实例（只匹配固定路径，不要误杀 /Applications 里的正式版）。
const stopOldInstance = () => {
  const pattern = localBundleProcessPattern(bundle);
  try {
    execFileSync("pkill", ["-f", pattern], { stdio: "ignore" });
  } catch (error) {
    if (error.status === 1) return; // 没有旧实例在跑
    throw error;
  }
  for (let attempt = 0; attempt < 25; attempt += 1) {
    try {
      execFileSync("pgrep", ["-f", pattern], { stdio: "ignore" });
    } catch (error) {
      if (error.status === 1) return;
      throw error;
    }
    execFileSync("sleep", ["0.2"]);
  }
  throw new Error("固定开发实例未退出，已停止替换，请先结束该实例后重试。");
};
stopOldInstance();

// 只换 .app，保留 bundle 目录下的 state / 日志，测试配置不丢。
const previousBundle = path.join(bundleDir, ".Codex++.app.previous");
rmSync(previousBundle, { recursive: true, force: true });
renameSync(bundle, previousBundle);
renameSync(stagingBundle, bundle);
rmSync(previousBundle, { recursive: true, force: true });
rmSync(stagingDir, { recursive: true, force: true });

if (!noOpen) {
  console.log("4/4 启动");
  // 正式版可同时运行且 bundle id 相同；强制启动这个路径，避免仅激活正式版。
  spawn("open", ["-n", bundle], { detached: true, stdio: "ignore" }).unref();
} else {
  console.log("4/4 已跳过启动（--no-open）");
}

console.log("清理历史本地构建");
run(process.execPath, [pruneScript, "--apply"]);

const stat = statSync(bundleBinary);
console.log(`完成：${bundleBinary}（${(stat.size / 1024 / 1024).toFixed(1)} MB）`);
