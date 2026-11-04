#!/usr/bin/env node
// 发版元数据：版本号散落的文件、从 CHANGELOG.md 截取某一版的说明。
// 本地发版脚本（scripts/release.mjs）和 CI（.github/workflows/release-assets.yml）共用这一份，
// 两边对「版本号在哪」「说明怎么截」的理解不会分叉。
//
// 命令行用法（CI 用）：
//   node scripts/release-meta.mjs check <版本号>   # 所有版本号文件都等于该版本，否则退出码 1
//   node scripts/release-meta.mjs notes <版本号>   # 把 CHANGELOG.md 里该版的说明打到 stdout，没有则退出码 1
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export const VERSION_PATTERN = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

const escapeRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

// 每个版本号文件：读出当前版本，以及把旧版本换成新版本。
// Cargo.lock 只改工作区自己的包（没有 source 字段的条目），不碰第三方依赖里碰巧同号的版本。
export const VERSION_FILES = [
  {
    file: "Cargo.toml",
    read: (text) => text.match(/\[workspace\.package\][^[]*?\nversion = "([^"]+)"/)?.[1],
    bump: (text, _from, to) => text.replace(/(\[workspace\.package\][^[]*?\nversion = ")[^"]+(")/, `$1${to}$2`),
  },
  {
    file: "Cargo.lock",
    read: (text) => {
      const versions = new Set(workspaceLockBlocks(text).map((block) => block.match(/\nversion = "([^"]+)"/)?.[1]));
      return versions.size === 1 ? [...versions][0] : undefined;
    },
    bump: (text, from, to) =>
      text
        .split("\n[[package]]\n")
        .map((block) =>
          isWorkspaceLockBlock(block) ? block.replace(`\nversion = "${from}"`, `\nversion = "${to}"`) : block,
        )
        .join("\n[[package]]\n"),
  },
  {
    file: "apps/codex-plus-manager/package.json",
    read: (text) => JSON.parse(text).version,
    bump: (text, from, to) => text.replace(`"version": "${from}"`, `"version": "${to}"`),
  },
  {
    file: "apps/codex-plus-manager/package-lock.json",
    read: (text) => {
      const lock = JSON.parse(text);
      return lock.version === lock.packages?.[""]?.version ? lock.version : undefined;
    },
    // 只有顶层和 packages[""] 两处是本项目版本，依赖的版本字段缩进更深。
    bump: (text, from, to) =>
      text
        .replace(new RegExp(`^(  "version": )"${escapeRegExp(from)}"`, "m"), `$1"${to}"`)
        .replace(new RegExp(`("": \\{\\n\\s+"name": "[^"]+",\\n\\s+"version": )"${escapeRegExp(from)}"`), `$1"${to}"`),
  },
  {
    file: "apps/codex-plus-manager/src-tauri/tauri.conf.json",
    read: (text) => JSON.parse(text).version,
    bump: (text, from, to) => text.replace(`"version": "${from}"`, `"version": "${to}"`),
  },
  {
    // 浏览器演示页里写死的「当前版本」，不影响打包，但发版时一直跟着改。
    file: "apps/codex-plus-manager/web-preview/tauri-web-shim.ts",
    read: (text) => text.match(/current_version: "([^"]+)"/)?.[1],
    bump: (text, from, to) => text.replaceAll(`"${from}"`, `"${to}"`),
  },
];

function isWorkspaceLockBlock(block) {
  return /^name = "codex-plus[^"]*"/m.test(block) && !/\nsource = /.test(block);
}

function workspaceLockBlocks(text) {
  return text.split("\n[[package]]\n").filter(isWorkspaceLockBlock);
}

export function readText(file) {
  return readFileSync(path.join(root, file), "utf8");
}

// 返回 [{ file, version }]，version 为 undefined 表示读不出来（格式变了或内部不一致）。
export function readVersions() {
  return VERSION_FILES.map(({ file, read }) => ({ file, version: read(readText(file)) }));
}

// 截取 CHANGELOG.md 里「## <版本号> ...」到下一个二级标题之间的内容，去掉标题行与首尾空行。
export function changelogSection(version, text = readText("CHANGELOG.md")) {
  const lines = text.split("\n");
  const heading = new RegExp(`^## \\[?v?${escapeRegExp(version)}\\]?(?:\\s|$)`);
  const start = lines.findIndex((line) => heading.test(line));
  if (start < 0) return "";
  let end = lines.findIndex((line, index) => index > start && /^## /.test(line));
  if (end < 0) end = lines.length;
  return lines.slice(start + 1, end).join("\n").trim();
}

// 语义版本比较：返回正数表示 a 比 b 新。预发布版本比同号正式版旧。
export function compareVersions(a, b) {
  const parse = (v) => {
    const [core, pre = ""] = v.replace(/^v/i, "").split("-", 2);
    return { parts: core.split(".").map(Number), pre };
  };
  const x = parse(a);
  const y = parse(b);
  for (let i = 0; i < 3; i += 1) {
    const diff = (x.parts[i] || 0) - (y.parts[i] || 0);
    if (diff) return diff;
  }
  if (x.pre === y.pre) return 0;
  if (!x.pre) return 1;
  if (!y.pre) return -1;
  return x.pre < y.pre ? -1 : 1;
}

function main(argv) {
  const [command, rawVersion] = argv;
  const version = (rawVersion || "").replace(/^v/i, "");
  if (!["check", "notes"].includes(command) || !VERSION_PATTERN.test(version)) {
    console.error("用法：node scripts/release-meta.mjs check|notes <版本号>");
    return 2;
  }
  if (command === "check") {
    const mismatched = readVersions().filter((entry) => entry.version !== version);
    for (const { file, version: found } of mismatched) {
      console.error(`版本号不一致：${file} 是 ${found ?? "（读不出来）"}，期望 ${version}`);
    }
    if (mismatched.length) return 1;
    console.error(`版本号一致：${version}`);
    return 0;
  }
  const notes = changelogSection(version);
  if (!notes) {
    console.error(`CHANGELOG.md 里没有 ${version} 这一节`);
    return 1;
  }
  process.stdout.write(`${notes}\n`);
  return 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}
