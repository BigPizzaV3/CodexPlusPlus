/**
 * 为按需下载市场生成轻量索引。只读取 Git 已提交内容，不复制插件、不读取账号凭据。
 * 用法：node scripts/build-plugin-market-index.mjs <CodexPlusPlusFullPluginCache> <output.json>
 * 输出提交到缓存仓库的 plugin-index-v2.json；仓库里的 plugin-index.json 是旧版本读的，不要覆盖。
 *
 * 仓库根目录有 directory-layout.json 时（官方目录的分组、组内顺序和图标地址，见 docs/plugin-market.md），
 * 把其中属于本索引的部分写进索引，原生插件页据此排成和官方目录一样的分组。
 */
import { readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";

const [directory, output] = process.argv.slice(2);
if (!directory || !output) {
  throw new Error("用法：build-plugin-market-index.mjs <CodexPlusPlusFullPluginCache> <output.json>");
}
const root = path.resolve(directory);
const repository = "BigPizzaV3/CodexPlusPlusFullPluginCache";
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
// .app.json 是 ChatGPT 托管的连接器（要 ChatGPT 登录），.mcp.json 是可在本机登录的 MCP 服务。
const appPackages = new Set();
const mcpPackages = new Set();
const trackedFiles = new Set();
for (const line of git("ls-tree", "-r", "-l", revision).trim().split("\n")) {
  const [metadata, file] = line.split("\t");
  trackedFiles.add(file);
  const parts = file.split("/");
  const key = parts.slice(0, 4).join("/");
  if (!key.startsWith("markets/")) continue;
  const bytes = Number(metadata.trim().split(/\s+/)[3]);
  sizes.set(key, (sizes.get(key) ?? 0) + (Number.isFinite(bytes) ? bytes : 0));
  if (file.endsWith("/SKILL.md")) skillCounts.set(key, (skillCounts.get(key) ?? 0) + 1);
  if (parts.at(-1) === ".app.json") appPackages.add(key);
  if (parts.at(-1) === ".mcp.json") mcpPackages.add(key);
}
const catalog = JSON.parse(await readFile(path.join(root, "directory.json"), "utf8"));
const entries = catalog.entries.filter((item) => item.marketState === "ready");
let layout = null;
try { layout = JSON.parse(await readFile(path.join(root, "directory-layout.json"), "utf8")); }
catch (error) { if (error.code !== "ENOENT") throw error; }
const httpsUrl = (value) => typeof value === "string" && /^https:\/\/[^\s"'<>\\]{1,600}$/.test(value) ? value : "";
const text = (value, max = 2000) => typeof value === "string" ? value.slice(0, max) : "";
const plugins = [];
for (const entry of entries) {
  const name = entry.manifestName;
  const id = entry.id ?? entry.remotePluginId ?? name;
  const pluginPath = `markets/${path.basename(entry.marketplaceRoot)}/plugins/${entry.archiveFile.slice(0, 16)}`;
  if (!trees.has(pluginPath)) throw new Error(`Git 提交内缺少插件目录：${pluginPath}`);
  let manifest;
  for (const file of [".codex-plugin/plugin.json", ".claude-plugin/plugin.json", "plugin.json"]) {
    if (!trackedFiles.has(`${pluginPath}/${file}`)) continue;
    try { manifest = JSON.parse(git("show", `${revision}:${pluginPath}/${file}`)); break; }
    catch { /* 只在已知清单位置寻找，不执行包内代码。 */ }
  }
  if (!manifest || manifest.name !== name) throw new Error(`插件清单与目录不一致：${pluginPath}`);
  const author = manifest.author ?? entry.author;
  const [iconUrl, iconUrlDark] = Array.isArray(layout?.icons?.[id]) ? layout.icons[id] : [];
  plugins.push({
    id, name,
    displayName: text(manifest.interface?.displayName ?? manifest.displayName ?? name, 200),
    description: text(manifest.description),
    // 列表里显示的一句话简介；包里没写的留空，界面退回完整描述。
    shortDescription: text(manifest.interface?.shortDescription, 300),
    version: text(manifest.version ?? entry.manifestVersion ?? entry.version ?? entry.directoryVersion ?? "0.0.0", 128),
    author: text(typeof author === "string" ? author : author?.name, 200),
    tags: Array.isArray(manifest.keywords) ? manifest.keywords.filter((tag) => typeof tag === "string").slice(0, 20) : [],
    // 原生插件页按它分组；包里没写的留空。
    category: text(manifest.interface?.category, 80).trim(),
    license: text(typeof manifest.license === "string" ? manifest.license : entry.declaredLicense ?? "", 300),
    path: pluginPath, tree: trees.get(pluginPath),
    bytes: sizes.get(pluginPath) ?? 0,
    skills: skillCounts.get(pluginPath) ?? 0,
    requiresAuth: appPackages.has(pluginPath) || mcpPackages.has(pluginPath),
    hasApps: appPackages.has(pluginPath),
    hasMcp: mcpPackages.has(pluginPath),
    ...(httpsUrl(iconUrl) ? { iconUrl } : {}),
    ...(httpsUrl(iconUrlDark) && iconUrlDark !== iconUrl ? { iconUrlDark } : {}),
  });
}
plugins.sort((a, b) => a.displayName.localeCompare(b.displayName) || a.id.localeCompare(b.id));
// 分组只留本索引里有的插件，没有成员的分组不写。
const known = new Set(plugins.map((plugin) => plugin.id));
const title = (value) => Object.fromEntries(Object.entries(value && typeof value === "object" ? value : {})
  .filter(([lang, name]) => /^[a-z]{2}(-[A-Za-z]{2,4})?$/.test(lang) && typeof name === "string" && name.trim())
  .map(([lang, name]) => [lang, name.trim().slice(0, 40)]));
const sections = (Array.isArray(layout?.sections) ? layout.sections : []).map((section) => ({
  slug: text(section.slug, 80), title: title(section.title), curated: section.curated === true,
  plugins: [...new Set(Array.isArray(section.plugins) ? section.plugins : [])].filter((id) => known.has(id)),
})).filter((section) => section.slug && section.title.en && section.plugins.length > 0);
const index = {
  schemaVersion: 1, repository, revision, updatedAt: catalog.snapshotDate, plugins,
  ...(sections.length > 0 ? { layoutDate: text(layout.capturedAt, 10), sections } : {}),
};
const contents = JSON.stringify(index) + "\n";
await writeFile(output, contents, "utf8");
console.log(JSON.stringify({repository, revision, plugins: plugins.length, indexBytes: Buffer.byteLength(contents), output}));
