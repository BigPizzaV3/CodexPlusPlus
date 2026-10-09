/**
 * 一次性切分脚本：把单体 assets/inject/renderer-inject.js 按顶层声明边界切成分片。
 *
 * 边界已经人工核对过，全部落在 `  function xxx()` / `  const xxx =` 这类顶层声明行上，
 * 保证任何一片单独看都是合法的声明序列（虽然它们共享同一个 IIFE 作用域）。
 *
 * 切完立刻拼回去比对 sha256，不一致就报错退出。
 */
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const sourcePath = path.join(root, "assets/inject/renderer-inject.js");
const outDir = path.join(root, "assets/inject/renderer-inject");

const manifestPath = path.join(outDir, "manifest.json");
const manifestSource = JSON.parse(await readFile(manifestPath, "utf8"));
if (!Array.isArray(manifestSource.fragments) || manifestSource.fragments.length === 0) {
  throw new Error("manifest.json 里没有 fragments");
}
const plan = manifestSource.fragments.map((fragment) => {
  if (!fragment?.name || !Number.isInteger(fragment.lines) || fragment.lines <= 0) {
    throw new Error(`manifest.json 的分片行数无效: ${JSON.stringify(fragment)}`);
  }
  return [fragment.name, fragment.description || "", fragment.lines];
});

const raw = await readFile(sourcePath, "utf8");
const lines = raw.split("\n");
// split("\n") 会把末尾换行后的空串算成一行，去掉它以便按行号切片
if (lines.at(-1) === "") lines.pop();

const expectedLines = plan.reduce((total, [, , count]) => total + count, 0);
if (expectedLines !== lines.length) {
  throw new Error(
    `manifest.json 行数与产物不一致: manifest=${expectedLines}, artifact=${lines.length}；` +
    "请先从分片组装产物，或人工同步 manifest 后再切分。",
  );
}

const fragments = [];
let offset = 0;
for (const [name, description, count] of plan) {
  const start = offset + 1;
  const end = offset + count;
  const body = lines.slice(offset, end).join("\n");
  if (!body) throw new Error(`分片 ${name} 为空`);
  // 每片都以换行结尾，拼接时天然还原原始行结构
  fragments.push({ name, description, start, end, body: `${body}\n` });
  offset = end;
}

// 校验一：分片逐个必须是顶层声明开头，不允许从函数体中间下刀
for (const fragment of fragments) {
  if (fragment.name === "99-tail.js" || fragment.name === "zz-paste-fix.js") continue;
  const first = fragment.body.split("\n")[0];
  if (first.startsWith("  ")) continue;
  if (first === "(() => {") continue;
  throw new Error(`分片 ${fragment.name} 未从顶层声明开始: ${first.slice(0, 80)}`);
}

// 校验二：拼回去必须与原文逐字节一致
const rejoined = fragments.map((fragment) => fragment.body).join("");
const originalHash = createHash("sha256").update(raw).digest("hex");
const rejoinedHash = createHash("sha256").update(rejoined).digest("hex");
if (originalHash !== rejoinedHash) {
  throw new Error(`拼接不一致\n  原文: ${originalHash}\n  重组: ${rejoinedHash}`);
}

await mkdir(outDir, { recursive: true });
const manifest = [];
for (const fragment of fragments) {
  await writeFile(path.join(outDir, fragment.name), fragment.body, "utf8");
  manifest.push({
    name: fragment.name,
    description: fragment.description,
    lines: fragment.end - fragment.start + 1,
  });
}
await writeFile(
  path.join(outDir, "manifest.json"),
  `${JSON.stringify({ fragments: manifest }, null, 2)}\n`,
  "utf8",
);

console.log(`已切分 ${fragments.length} 个分片，sha256 一致: ${originalHash}`);
for (const item of manifest) {
  console.log(`  ${item.name.padEnd(26)} ${String(item.lines).padStart(6)} 行  ${item.description}`);
}
