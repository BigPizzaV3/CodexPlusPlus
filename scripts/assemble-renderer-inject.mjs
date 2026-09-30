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
import { readFile, writeFile } from "node:fs/promises";
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

let assembled = "";
for (const fragment of manifest.fragments) {
  const body = await readFile(path.join(fragmentDir, fragment.name), "utf8");
  if (!body.endsWith("\n")) {
    throw new Error(`分片 ${fragment.name} 未以换行结尾，拼接会粘连`);
  }
  assembled += body;
}

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
  console.log(`产物已是最新 (${hash})`);
} else {
  await writeFile(artifactPath, assembled, "utf8");
  console.log(`已写入产物 ${manifest.fragments.length} 个分片 (${hash})`);
}
