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

// [起始行(1-based, 含), 分片文件名, 说明]
const PLAN = [
  [1, "00-prelude.js", "IIFE 开头、环境守卫、平台探测、常量与工具函数"],
  [603, "10-style.js", "样式注入、默认设置、后端设置映射、梦皮主题变量与持久化"],
  [2640, "20-menu.js", "Codex++ 菜单、后端设置加载、插件解锁策略与版本判定"],
  [3112, "30-service-tier.js", "服务等级（service tier）读写、线程覆盖与控件同步"],
  [4196, "40-backend-settings.js", "后端设置读写、心跳同步、用户脚本与市场状态、拓展 UI"],
  [5687, "50-navigation.js", "导航入口安装、rail 按钮、页面布局与选中态同步"],
  [5930, "60-plugin-marketplace.js", "插件市场请求/响应补丁、本地兜底与合并"],
  [7549, "70-model-catalog.js", "全局状态、模型目录、模型白名单与解锁"],
  [8482, "80-session-share.js", "会话分享、项目区、toast、工作区路径归一"],
  [9940, "90-action-groups.js", "会话行操作按钮布局、更多菜单、工具提示"],
  [10748, "95-conversation-view.js", "会话视图运行时、官方用量策略重写、扫描主循环"],
  [11988, "98-scan-schedule.js", "扫描相关性判定与调度，启动重试"],
  [12108, "99-startup.js", "启动序列：resize 处理、MutationObserver 与事件绑定"],
  [12138, "99-tail.js", "主 IIFE 收尾 `})();`"],
  [12141, "zz-paste-fix.js", "粘贴修复块：Word 粘贴降级为纯文本"],
];

const raw = await readFile(sourcePath, "utf8");
const lines = raw.split("\n");
// split("\n") 会把末尾换行后的空串算成一行，去掉它以便按行号切片
if (lines.at(-1) === "") lines.pop();

const fragments = [];
for (let i = 0; i < PLAN.length; i += 1) {
  const [start, name, description] = PLAN[i];
  const end = i + 1 < PLAN.length ? PLAN[i + 1][0] - 1 : lines.length;
  if (end < start) throw new Error(`分片 ${name} 行范围非法: ${start}-${end}`);
  const body = lines.slice(start - 1, end).join("\n");
  if (!body) throw new Error(`分片 ${name} 为空`);
  // 每片都以换行结尾，拼接时天然还原原始行结构
  fragments.push({ name, description, start, end, body: `${body}\n` });
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
