/**
 * 为按需下载市场生成轻量索引。只读取 Git 已提交内容，不复制插件、不读取账号凭据。
 * 用法：node scripts/build-plugin-market-index.mjs <cache-repo> <public|full> <output.json>
 */
import { readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";

const [directory, source, output] = process.argv.slice(2);
if (!directory || !output || !["public", "full"].includes(source)) {
  throw new Error("用法：build-plugin-market-index.mjs <cache-repo> <public|full> <output.json>");
}
const root = path.resolve(directory);
const repository = source === "full"
  ? "BigPizzaV3/CodexPlusPlusFullPluginCache"
  : "BigPizzaV3/CodexPlusPlusPluginCache";
const git = (...args) => execFileSync("git", ["-C", root, ...args], {
  encoding: "utf8", maxBuffer: 64 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"],
});
const revision = git("rev-parse", "HEAD").trim();
const trees = new Map(git("ls-tree", "-r", "-d", revision).trim().split("\n").map((line) => {
  const [metadata, name] = line.split("\t");
  return [name, metadata.split(" ")[2]];
}));
const sizes = new Map();
const skillCounts = new Map();
const authPackages = new Set();
const trackedFiles = new Set();
for (const line of git("ls-tree", "-r", "-l", revision).trim().split("\n")) {
  const [metadata, file] = line.split("\t");
  trackedFiles.add(file);
  const parts = file.split("/");
  const key = source === "full" ? parts.slice(0, 4).join("/") : parts.slice(0, 2).join("/");
  if (!key.startsWith(source === "full" ? "markets/" : "plugins/")) continue;
  const bytes = Number(metadata.trim().split(/\s+/)[3]);
  sizes.set(key, (sizes.get(key) ?? 0) + (Number.isFinite(bytes) ? bytes : 0));
  if (file.endsWith("/SKILL.md")) skillCounts.set(key, (skillCounts.get(key) ?? 0) + 1);
  if (parts.at(-1) === ".app.json" || parts.at(-1) === ".mcp.json") authPackages.add(key);
}
const catalog = JSON.parse(await readFile(path.join(root, source === "full" ? "directory.json" : "catalog.json"), "utf8"));
const entries = source === "full" ? catalog.entries.filter((item) => item.marketState === "ready") : catalog.plugins;
const plugins = [];
for (const entry of entries) {
  const name = source === "full" ? entry.manifestName : entry.name;
  const id = entry.id ?? entry.remotePluginId ?? name;
  const pluginPath = source === "full"
    ? `markets/${path.basename(entry.marketplaceRoot)}/plugins/${entry.archiveFile.slice(0, 16)}`
    : `plugins/${name}`;
  if (!trees.has(pluginPath)) throw new Error(`Git 提交内缺少插件目录：${pluginPath}`);
  let manifest;
  for (const file of [".codex-plugin/plugin.json", ".claude-plugin/plugin.json", "plugin.json"]) {
    if (!trackedFiles.has(`${pluginPath}/${file}`)) continue;
    try { manifest = JSON.parse(git("show", `${revision}:${pluginPath}/${file}`)); break; }
    catch { /* 只在已知清单位置寻找，不执行包内代码。 */ }
  }
  if (!manifest || manifest.name !== name) throw new Error(`插件清单与目录不一致：${pluginPath}`);
  const text = (value, max = 2000) => typeof value === "string" ? value.slice(0, max) : "";
  const author = manifest.author ?? entry.author;
  plugins.push({
    id, name,
    displayName: text(manifest.interface?.displayName ?? manifest.displayName ?? name, 200),
    description: text(manifest.description),
    version: text(manifest.version ?? entry.manifestVersion ?? entry.version ?? entry.directoryVersion ?? "0.0.0", 128),
    author: text(typeof author === "string" ? author : author?.name, 200),
    tags: Array.isArray(manifest.keywords) ? manifest.keywords.filter((tag) => typeof tag === "string").slice(0, 20) : [],
    license: text(typeof manifest.license === "string" ? manifest.license : entry.declaredLicense ?? "", 300),
    path: pluginPath, tree: trees.get(pluginPath),
    bytes: sizes.get(pluginPath) ?? 0,
    skills: skillCounts.get(pluginPath) ?? 0,
    requiresAuth: authPackages.has(pluginPath),
  });
}
plugins.sort((a, b) => a.displayName.localeCompare(b.displayName) || a.id.localeCompare(b.id));
const index = {schemaVersion: 1, repository, revision, updatedAt: catalog.snapshotDate, plugins};
const contents = JSON.stringify(index) + "\n";
await writeFile(output, contents, "utf8");
console.log(JSON.stringify({source, repository, revision, plugins: plugins.length, indexBytes: Buffer.byteLength(contents), output}));
