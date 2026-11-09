/**
 * 组装脚本：把 assets/inject/renderer-inject/*.js 分片按 manifest.json 的顺序
 * 拼回单体产物 assets/inject/renderer-inject.js。
 *
 * 为什么保留单体产物：
 *   - crates/codex-plus-core/src/assets.rs 用 include_str! 按路径读取
 *   - apps/codex-plus-manager/src/renderer-inject.test.ts（19 处）、dream-skin.test.ts（4 处）
 *     也按路径读取，改成读分片要动大量既有断言
 * 所以分片是唯一真实来源，单体文件是生成物；漂移由 `assemble --check` 与
 * renderer-inject.test.ts 里的 drift 用例把守。
 *
 * 用法：
 *   node scripts/assemble-renderer-inject.mjs           # 写回产物
 *   node scripts/assemble-renderer-inject.mjs --check   # 只校验，不写（CI 用）
 */
import { readFile, readdir, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const fragmentDir = path.join(root, "assets/inject/renderer-inject");
const artifactPath = path.join(root, "assets/inject/renderer-inject.js");
const checkOnly = process.argv.includes("--check");

const manifest = JSON.parse(
  await readFile(path.join(fragmentDir, "manifest.json"), "utf8"),
);
if (!Array.isArray(manifest.fragments) || manifest.fragments.length === 0) {
  throw new Error("manifest.json 里没有 fragments");
}

// 不能只检查已登记的分片：合并时同时漏掉清单项与生成代码也会看似一致。
const fragmentNames = manifest.fragments.map((fragment) => fragment.name);
if (fragmentNames.some((name) => typeof name !== "string" || !/^[^/\\]+\.js$/.test(name))) {
  throw new Error("manifest.json 的分片名必须是当前目录内的 .js 文件名");
}
if (new Set(fragmentNames).size !== fragmentNames.length) {
  throw new Error("manifest.json 不得重复引用同一分片");
}
const diskNames = (await readdir(fragmentDir, { withFileTypes: true }))
  .filter((entry) => entry.isFile() && entry.name.endsWith(".js"))
  .map((entry) => entry.name);
const unlisted = diskNames.filter((name) => !fragmentNames.includes(name)).sort();
const missing = fragmentNames.filter((name) => !diskNames.includes(name)).sort();
if (unlisted.length || missing.length) {
  throw new Error(
    "manifest.json 与分片目录不一致\n" +
      (unlisted.length ? `  未登记的分片: ${unlisted.join(", ")}\n` : "") +
      (missing.length ? `  不存在的分片: ${missing.join(", ")}\n` : "") +
      "  合并时请保留两侧新增分片，确认顺序后重新组装；移除功能时同步移除源码分片。",
  );
}

let assembled = "";
const fragmentLines = new Map();
for (const fragment of manifest.fragments) {
  const body = await readFile(path.join(fragmentDir, fragment.name), "utf8");
  if (!body.endsWith("\n")) {
    throw new Error(`分片 ${fragment.name} 未以换行结尾，拼接会粘连`);
  }
  // manifest 里的 lines 是产物行数的来源，测试拿它跟产物对账。分片一改就得同步，
  // 否则每次改动都要人手数一遍——这里直接按磁盘上的分片算，写回 manifest。
  fragmentLines.set(fragment.name, body.split("\n").length - 1);
  assembled += body;
}
let manifestChanged = false;
for (const fragment of manifest.fragments) {
  const lines = fragmentLines.get(fragment.name);
  if (fragment.lines !== lines) {
    fragment.lines = lines;
    manifestChanged = true;
  }
}
const manifestPath = path.join(fragmentDir, "manifest.json");
const manifestText = `${JSON.stringify(manifest, null, 2)}\n`;
const currentManifestText = await readFile(manifestPath, "utf8");

// 自检：拼出来的东西必须是完整可解析的，且 IIFE 有始有终
if (!assembled.startsWith("(() => {\n")) {
  throw new Error("组装结果未以 IIFE 开头");
}
if (!assembled.includes("\n})();\n")) {
  throw new Error("组装结果里找不到主 IIFE 的收尾 `})();`");
}
try {
  // 只做语法解析，不执行；能抓出分片边界切坏导致的括号不匹配
  new Function(assembled);
} catch (error) {
  throw new Error(`组装结果语法错误: ${error.message}`);
}

const hash = createHash("sha256").update(assembled).digest("hex");
const current = await readFile(artifactPath, "utf8").catch(() => null);

if (checkOnly) {
  if (manifestChanged) {
    throw new Error(
      "manifest.json 的分片行数与分片实际行数不一致\n" +
        "  修复: node scripts/assemble-renderer-inject.mjs",
    );
  }
  if (current === null) {
    throw new Error("产物文件不存在，先跑一次不带 --check 的组装");
  }
  if (current !== assembled) {
    const currentHash = createHash("sha256").update(current).digest("hex");
    throw new Error(
      `产物与分片不一致，分片被改过但没重新组装\n` +
        `  产物: ${currentHash}\n  分片: ${hash}\n` +
        `  修复: node scripts/assemble-renderer-inject.mjs`,
    );
  }
  console.log(`产物与分片一致 (${hash})`);
} else if (current === assembled) {
  if (manifestChanged) {
    await writeFile(manifestPath, manifestText, "utf8");
    console.log("已同步 manifest.json 的分片行数");
  }
  console.log(`产物已是最新 (${hash})`);
} else {
  await writeFile(artifactPath, assembled, "utf8");
  if (manifestChanged) await writeFile(manifestPath, manifestText, "utf8");
  console.log(`已写入产物 ${manifest.fragments.length} 个分片 (${hash})`);
}
