// 用当前源码构建管理器网页演示版（演示数据，不读本机配置），重截 README 与官网用的管理器截图。
// 深色主题、1440×900 @2x、WebP，中英文各一套：
//   整窗图 → docs/images/readme-preview/<name>[-en].webp 与 site/assets/<name>[-en].webp
//   使用教程的内容区截图 → site/assets/guide/<name>.webp（只有中文）
// 用法：node tools/promo-video/capture-readme-shots.mjs [--skip-build] [--only name1,name2] [--lang zh|en]
import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { launchChrome, serveStatic, sleep } from "./lib/cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../..");
const previewDir = path.join(repo, "dist/promo/preview");
const readmeDir = path.join(repo, "docs/images/readme-preview");
const siteDir = path.join(repo, "site/assets");
const guideDir = path.join(siteDir, "guide");
const args = process.argv.slice(2);
const argValue = (flag) => (args.includes(flag) ? args[args.indexOf(flag) + 1] : null);
const only = argValue("--only")?.split(",") ?? null;
const langs = argValue("--lang") ? [argValue("--lang")] : ["zh", "en"];
const QUALITY = 88;

if (!args.includes("--skip-build")) {
  const manager = path.join(repo, "apps/codex-plus-manager");
  const result = spawnSync(process.execPath, [
    path.join(manager, "node_modules/vite/bin/vite.js"), "build",
    "--config", "web-preview/vite.config.ts", "--outDir", previewDir, "--emptyOutDir", "--logLevel", "warn",
  ], { cwd: manager, stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
mkdirSync(guideDir, { recursive: true });

// 按文字 / aria-label 点元素；找不到就报错，免得截到错的页面。
const CLICK = `(text, opts = {}) => {
  const pool = [...document.querySelectorAll(opts.selector ?? 'button,[role=tab],[role=button],[role=radio],[role=switch],a,[role=menuitem]')]
    .filter(el => el.getClientRects().length && Boolean(opts.sidebar) === Boolean(el.closest('aside')));
  const label = el => (el.getAttribute('aria-label') ?? '').trim();
  const words = el => el.textContent.replace(/\\s+/g, ' ').trim();
  const hits = pool.filter(el => label(el) === text || words(el) === text);
  const loose = hits.length ? hits : pool.filter(el => words(el).startsWith(text) || label(el).startsWith(text));
  const el = loose[opts.nth ?? 0];
  if (!el) throw new Error('找不到可点的「' + text + '」');
  el.scrollIntoView({ block: 'center' });
  el.click();
  return true;
}`;

// 关掉入门引导、导览和右下角的推荐提示，画面只留功能本身。
const TIDY = `(() => {
  for (const b of document.querySelectorAll('button')) {
    const label = (b.getAttribute('aria-label') ?? '') + ' ' + b.textContent;
    if (/不再显示启动引导|getting-started guide again/.test(label)) b.click();
  }
  for (const dialog of document.querySelectorAll('[role=dialog]')) {
    const later = [...dialog.querySelectorAll('button')].find(b => /^(稍后设置|Set up later|跳过|Skip)$/.test(b.textContent.trim()));
    later?.click();
  }
  document.activeElement?.blur?.();
  window.scrollTo(0, 0);
})()`;

// 内容区（不含侧栏）里实际有内容的范围，给使用教程裁图用。
const CONTENT_BOX = `((maxBottom) => {
  const screen = document.querySelector('main section.screen') ?? document.querySelector('main');
  const head = screen.querySelector('.cx-page-head') ?? screen.firstElementChild;
  let left = Infinity, right = 0, bottom = 0;
  for (const el of screen.querySelectorAll('*')) {
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height || r.top > window.innerHeight) continue;
    left = Math.min(left, r.left); right = Math.max(right, r.right); bottom = Math.max(bottom, r.bottom);
  }
  const top = head.getBoundingClientRect().top;
  const pad = 20;
  return { x: Math.max(0, left - pad), y: Math.max(0, top - pad), width: Math.min(window.innerWidth, right + pad) - Math.max(0, left - pad), height: Math.min(maxBottom ?? window.innerHeight, window.innerHeight, bottom + pad) - Math.max(0, top - pad) };
})`;

const page = await launchChrome({ width: 1440, height: 900, scale: 2 });
const server = await serveStatic({ "/": previewDir });

let lang = "zh";
const L = (zh, en) => (lang === "en" ? en : zh);

async function open(route, settle = 1600, query = "") {
  await page.goto(`${server.url}/?lang=${lang}&previewRoute=${route}${query}`);
  await sleep(settle);
  await page.evaluate(TIDY);
  await sleep(300);
}

async function click(zh, en, opts) {
  await page.evaluate(`(${CLICK})(${JSON.stringify(L(zh, en))}, ${JSON.stringify(opts ?? {})})`);
  await sleep(700);
}

// 从侧栏进 Codex → 万物皆拓展（直接落地时拓展图标不加载）。
async function openExtensions() {
  await open("home"); await sidebar("Codex"); await sidebar("万物皆拓展", "Everything is an extension");
}

// 点侧栏里的一项（Agent 行、Agent 下的页面、顶部的全局页面）。
async function sidebar(zh, en = zh) {
  await click(zh, en, { sidebar: true, selector: "aside button, aside a, aside [role=button], aside [role=link]" });
  await sleep(600);
}

// 等页面里的图片（拓展作者头像等联网图片）加载完，最多等 10 秒。
const IMAGES_READY = `new Promise((resolve) => {
  const deadline = Date.now() + 10000;
  const tick = () => {
    const pending = [...document.images].filter((img) => img.getClientRects().length && !img.complete);
    if (!pending.length || Date.now() > deadline) resolve(pending.length); else setTimeout(tick, 200);
  };
  tick();
})`;

async function save(name, { settle = 900 } = {}) {
  await sleep(settle);
  await page.evaluate(IMAGES_READY);
  await page.evaluate(TIDY);
  await sleep(200);
  const webp = await page.screenshot({ format: "webp", quality: QUALITY });
  const file = `${name}${lang === "en" ? "-en" : ""}.webp`;
  writeFileSync(path.join(readmeDir, file), webp);
  writeFileSync(path.join(siteDir, file), webp);
  console.log(`✓ ${file}`);
}

// 使用教程只有中文版：按内容区裁一张，maxBottom 限制裁到页面哪里。
async function guide(name, { maxBottom } = {}) {
  if (lang !== "zh") return;
  const clip = await page.evaluate(`(${CONTENT_BOX})(${JSON.stringify(maxBottom ?? null)})`);
  const webp = await page.screenshot({ format: "webp", quality: QUALITY, clip });
  writeFileSync(path.join(guideDir, `${name}.webp`), webp);
  console.log(`✓ guide/${name}.webp ${Math.round(clip.width * 2)}×${Math.round(clip.height * 2)}`);
}

const SHOTS = {
  "home": async () => { await open("home"); await save("home"); await guide("home"); },
  "ext-featured": async () => { await openExtensions(); await save("ext-featured", { settle: 2500 }); },
  "ext-community": async () => { await openExtensions(); await click("社区", "Community"); await save("ext-community", { settle: 1600 }); },
  "ext-dsh": async () => { await openExtensions(); await click("导入 dsh 插件", "Import dsh plugin"); await save("ext-dsh", { settle: 1400 }); },
  // Codex 的供应商页在「聚合」：共享供应商每家一个开关，加上模型组和默认模型。
  "gateway-providers": async () => {
    await open("relay", 2200, "&codexMode=aggregate");
    await save("gateway-providers"); await guide("gatewayProviders");
  },
  "claude-providers": async () => { await open("home"); await sidebar("Claude Code"); await save("claude-providers"); },
  "gateway-agents": async () => { await open("gateway", 2000); await save("gateway-agents"); await guide("gateway"); },
  "gateway-routing": async () => { await open("gatewayRouting", 2000); await save("gateway-routing"); await guide("gatewayRouting"); },
  "usage-stats": async () => { await open("gatewayUsage", 2200); await save("usage-stats", { settle: 1400 }); },
  "usage-requests": async () => { await open("requestLog", 2200); await save("usage-requests", { settle: 1400 }); await guide("requestLog"); },
  "computer-use": async () => { await open("computerUse", 2000); await save("computer-use", { settle: 1400 }); await guide("computerUse", { maxBottom: 640 }); },
  // 「万物皆拓展 → 本地」：本地拓展和内置拓展摘要（内置拓展的开关在 Codex 里的 Codex++ 设置页）。
  "ext-local": async () => {
    await openExtensions(); await click("本地", "Local", { selector: "[role=tab],button" });
    await save("ext-local", { settle: 1200 }); await guide("extLocal");
  },
  "codex-plugins": async () => { await open("pluginMarket", 2400); await save("codex-plugins", { settle: 1400 }); },
  // Claude Desktop 的页面从侧栏点进去，侧栏才展开 Claude Desktop 这一组。
  "claude-local-settings": async () => {
    await open("home"); await sidebar("Claude Desktop"); await sidebar("Desktop 增强", "Desktop enhancements");
    await save("claude-local-settings", { settle: 1200 });
  },
  // 补丁 Desktop 侧栏不单列（启动模式选补丁副本时并进 Desktop 增强页），只给使用教程裁内容区。
  "claude-patch": async () => { await open("claudePatch", 2000); await guide("claude"); },
  // 会话页直接落地时演示数据读不出来，从侧栏点进去。
  "sessions": async () => { await open("home"); await sidebar("会话", "Session"); await save("sessions", { settle: 2200 }); await guide("sessions"); },
  "agent-cache": async () => { await open("agentCache", 2400); await save("agent-cache"); },
  "codex-multi-launch": async () => { await open("multiLaunch", 2000); await save("codex-multi-launch"); },
};

try {
  for (const next of langs) {
    lang = next;
    for (const [name, run] of Object.entries(SHOTS)) {
      if (only && !only.includes(name)) continue;
      try { await run(); } catch (error) { console.error(`✗ ${name} (${lang}): ${error.message}`); }
    }
  }
} finally {
  server.close();
  await page.close();
}
