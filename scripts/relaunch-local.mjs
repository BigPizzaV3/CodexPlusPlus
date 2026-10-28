#!/usr/bin/env node
// 本地构建 + 重启固定开发实例 + 清理旧构建（AGENTS.md「改完即构建启动」的推荐入口）。
//
// 固定使用 dist/local-build/Codex++.app：
//   - 只替换 Contents/MacOS/CodexPlusPlus，保留 bundle 的 Info.plist / LSEnvironment / state，
//     这样每次构建都复用同一个应用身份和测试配置，不会在 Finder / Launchpad 里多出一个 Codex++。
//   - 图标源（src-tauri/icons/icon-macos.png）变了（按内容指纹判断），按打包脚本同样的方式重新生成 .icns，
//     否则换了 logo 之后 Dock 里还是旧图标。只换这一个文件，不动 Info.plist 与 bundle 身份。
//   - 旧的历史构建交给 scripts/prune-local-builds.mjs 移入废纸篓。
//
// 用法：
//   node scripts/relaunch-local.mjs                 # 构建前端 + cargo，替换固定 bundle，重启，清理旧构建
//   node scripts/relaunch-local.mjs --skip-build    # 跳过构建，用现有 target/release 二进制重装并重启
//   node scripts/relaunch-local.mjs --no-open       # 不自动启动（只替换 bundle）
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, cpSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
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

/**
 * 图标源内容变了就重新生成 .icns（尺寸与 scripts/installer/macos/package-dmg.sh 一致）。返回是否更新了。
 * 不能比较修改时间：每次构建都会整体复制 bundle，复制出来的 .icns 时间总是比图标源新。
 * 所以在 Resources 里记一份图标源的 sha256，内容不同才重新生成。
 */
function refreshBundleIcon(target) {
  const source = path.join(root, "apps", "codex-plus-manager", "src-tauri", "icons", "icon-macos.png");
  if (!existsSync(source)) return false;
  let iconName = "codex-plus-plus.icns";
  try {
    iconName = execFileSync("/usr/libexec/PlistBuddy", ["-c", "Print :CFBundleIconFile", path.join(target, "Contents", "Info.plist")], { encoding: "utf8" }).trim() || iconName;
  } catch {
    // Info.plist 没写图标名时用默认名
  }
  if (!iconName.endsWith(".icns")) iconName += ".icns";
  const icns = path.join(target, "Contents", "Resources", iconName);
  const marker = path.join(target, "Contents", "Resources", ".icon-source.sha256");
  const digest = createHash("sha256").update(readFileSync(source)).digest("hex");
  if (existsSync(icns) && existsSync(marker) && readFileSync(marker, "utf8").trim() === digest) return false;
  const iconset = path.join(stagingDir, "codex-plus-plus.iconset");
  mkdirSync(iconset, { recursive: true });
  for (const [name, size] of [
    ["icon_16x16", 16], ["icon_16x16@2x", 32], ["icon_32x32", 32], ["icon_32x32@2x", 64],
    ["icon_128x128", 128], ["icon_128x128@2x", 256], ["icon_256x256", 256], ["icon_256x256@2x", 512],
    ["icon_512x512", 512], ["icon_512x512@2x", 1024],
  ]) {
    execFileSync("sips", ["-z", String(size), String(size), source, "--out", path.join(iconset, `${name}.png`)], { stdio: "ignore" });
  }
  mkdirSync(path.dirname(icns), { recursive: true });
  execFileSync("iconutil", ["-c", "icns", iconset, "-o", icns], { stdio: "inherit" });
  writeFileSync(marker, `${digest}\n`);
  console.log(`已更新应用图标：${path.relative(root, icns)}`);
  return true;
}

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
const iconUpdated = refreshBundleIcon(stagingBundle);
// Developer ID 证书按 Team ID 记账，重建后 TCC 授权（屏幕录制等）不会因 cdhash 变化而失效。
// ad-hoc（"-"）则每次都产生新 cdhash，必须在系统设置里重新授权。
// 只在 /usr/bin/security 能找到证书时用 Developer ID，否则回退 ad-hoc。
let signingId = "-";
try {
  const identities = execFileSync("security", ["find-identity", "-v", "-p", "codesigning"], { encoding: "utf8" });
  const match = identities.match(/"(Developer ID Application:[^"]+)"/);
  if (match) signingId = match[1];
} catch {
  // 找不到证书，沿用 ad-hoc
}
console.log(`签名身份：${signingId === "-" ? "ad-hoc（TCC 授权在重建后会失效）" : signingId}`);
run("codesign", ["--force", "--deep", "--sign", signingId, stagingBundle]);

// 停掉固定目录里的旧实例（只匹配固定路径，不要误杀 /Applications 里的正式版）。
//
// 同一路径下还有各客户端（Claude Code 等）拉起的 MCP 服务与钩子进程，它们不属于管理器：
// 一并结束会让开着的 Claude 会话里 codex-plus-cu / codex-plus-browser 断开、只能手动 /mcp 重连。
// 这些进程继续跑旧二进制没有问题（替换 bundle 不影响已经在跑的进程），新会话会用上新二进制。
const CLIENT_OWNED_FLAGS = [
  "--codex-plus-cu-mcp",
  "--codex-plus-browser-mcp",
  "--codex-plus-agent-mcp",
  "--codex-plus-browser-turn-end",
  "--codex-plus-cu-cli",
];

/** 固定实例里属于管理器的进程（排除客户端拉起的 MCP / 钩子进程）。 */
const instancePids = () => {
  let output = "";
  try {
    output = execFileSync("pgrep", ["-lf", localBundleProcessPattern(bundle)], { encoding: "utf8" });
  } catch (error) {
    if (error.status === 1) return []; // 没有匹配的进程
    throw error;
  }
  return output
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .filter((line) => !CLIENT_OWNED_FLAGS.some((flag) => line.split(/\s+/).includes(flag)))
    .map((line) => Number(line.split(/\s+/)[0]))
    .filter((pid) => Number.isInteger(pid) && pid !== process.pid);
};

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

const stopOldInstance = () => {
  const pids = instancePids();
  for (const pid of pids) {
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // 已经退出
    }
  }
  for (let attempt = 0; attempt < 25; attempt += 1) {
    if (!pids.some(alive)) return;
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
if (iconUpdated) {
  // 换了图标：更新 bundle 时间戳并重新登记，Finder / Dock 才会丢掉缓存的旧图标。
  const now = new Date();
  utimesSync(bundle, now, now);
  try {
    execFileSync("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister", ["-f", bundle], { stdio: "ignore" });
  } catch {
    // 登记失败不影响启动，最多 Dock 晚一点刷新
  }
}

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
