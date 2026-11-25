// 用当前源码构建管理器网页演示版（演示数据，不读本机配置），按 1440×900@2x 截取宣传片要用的界面。
// 用法：node tools/promo-video/capture-shots.mjs [--skip-build] [--only name1,name2]
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { launchChrome, serveStatic, sleep } from "./lib/cdp.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../..");
const out = path.join(repo, "dist/promo");
const previewDir = path.join(out, "preview");
const shotsDir = path.join(out, "shots");
const args = process.argv.slice(2);
const only = args.includes("--only") ? args[args.indexOf("--only") + 1].split(",") : null;

if (!args.includes("--skip-build")) {
  const manager = path.join(repo, "apps/codex-plus-manager");
  const result = spawnSync(process.execPath, [
    path.join(manager, "node_modules/vite/bin/vite.js"), "build",
    "--config", "web-preview/vite.config.ts", "--outDir", previewDir, "--emptyOutDir", "--logLevel", "warn",
  ], { cwd: manager, stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
mkdirSync(shotsDir, { recursive: true });

// 页面里按文字 / aria-label 点元素；找不到就报错，免得截到错的页面。
const CLICK = `(text, opts = {}) => {
  const pool = [...document.querySelectorAll(opts.selector ?? 'button,[role=tab],[role=button],[role=radio],[role=switch],a,[role=menuitem]')]
    .filter(el => el.getClientRects().length);
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

const page = await launchChrome({ width: 1440, height: 900, scale: 2 });
const server = await serveStatic({ "/": previewDir });

async function click(text, opts) {
  await page.evaluate(`(${CLICK})(${JSON.stringify(text)}, ${JSON.stringify(opts ?? {})})`);
  await sleep(500);
}

// 关掉入门引导、侧栏「开始使用」卡片和右下角的推荐提示，画面只留功能本身。
async function tidy() {
  await page.evaluate(`(() => {
    const buttons = [...document.querySelectorAll('button')];
    for (const b of buttons) {
      const label = (b.getAttribute('aria-label') ?? '') + ' ' + b.textContent;
      if (/不再显示启动引导/.test(label)) b.click();
    }
    const dialog = document.querySelector('[role=dialog]');
    if (dialog && /连接模型|开始使用/.test(dialog.textContent)) {
      const later = [...dialog.querySelectorAll('button')].find(b => b.textContent.trim() === '稍后设置');
      later?.click();
    }
    for (const el of document.querySelectorAll('div,aside,section')) {
      if (el.children.length < 8 && /^正在找合适的供应商/.test(el.textContent.trim()) && el.getBoundingClientRect().width < 500) {
        const close = el.querySelector('button[aria-label]') ?? [...el.querySelectorAll('button')].at(-1);
        close?.click();
        break;
      }
    }
  })()`);
  await sleep(400);
}

async function fresh(hash = "") {
  await page.goto(`${server.url}/?lang=zh${hash}`);
  await sleep(1200);
  await tidy();
}

// 找出包含某段文字的最小元素，再向上找到够宽 / 够高的容器（整行、整张卡片），返回它在页面里的位置。
const LOCATE = `(specs) => {
  const result = {};
  const all = [...document.querySelectorAll('body *')].filter(el => el.getClientRects().length && !el.closest('aside'));
  for (const [key, [text, opts = {}]] of Object.entries(specs)) {
    const hits = all.filter(el => el.textContent.includes(text));
    let el = hits.find(h => ![...h.children].some(c => c.textContent.includes(text))) ?? null;
    while (el && el.parentElement && (el.getBoundingClientRect().width < (opts.minW ?? 0) || el.getBoundingClientRect().height < (opts.minH ?? 0))) el = el.parentElement;
    if (!el) { result[key] = null; continue; }
    const r = el.getBoundingClientRect();
    result[key] = { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
  }
  return result;
}`;

const regionsFile = path.join(shotsDir, "regions.json");
const regions = (() => { try { return JSON.parse(readFileSync(regionsFile, "utf8")); } catch { return {}; } })();

async function shoot(name, { clip, settle = 900, mark = {} } = {}) {
  await sleep(settle);
  await tidy();
  const png = await page.screenshot({ format: "png", clip });
  writeFileSync(path.join(shotsDir, `${name}.png`), png);
  regions[name] = await page.evaluate(`(${LOCATE})(${JSON.stringify(mark)})`);
  const missing = Object.entries(regions[name]).filter(([, value]) => !value).map(([key]) => key);
  console.log(`✓ ${name}${missing.length ? `（没找到：${missing.join("、")}）` : ""}`);
}

const ROW = { minW: 900 };
const SHOTS = {
  "home": async () => { await fresh(); await click("首页"); await shoot("home"); },
  "providers": async () => {
    await fresh(); await click("本地服务"); await click("共享供应商");
    await shoot("providers", { mark: { chatgpt: ["ChatGPT 订阅", ROW], quota: ["已用 23%"], glm: ["智谱 GLM", ROW], limited: ["被限流"], relay: ["倍率 0.28x"], add: ["添加供应商"], list: ["ChatGPT 订阅", { minW: 900, minH: 300 }] } });
  },
  "provider-detail": async () => {
    await fresh(); await click("本地服务"); await click("共享供应商");
    await click("详情", { nth: 1 });
    await shoot("provider-detail", { settle: 1200, mark: { models: ["deepseek-v4-pro", { minW: 400 }], keys: ["Key", { minW: 900 }], card: ["api.deepseek.com", { minW: 900, minH: 120 }] } });
  },
  "routing": async () => {
    await fresh(); await click("本地服务"); await click("路由");
    await shoot("routing", { mark: { main: ["group/主力编程", { minW: 900, minH: 200 }], cheap: ["group/便宜轮换", { minW: 900, minH: 200 }], mode: ["缓存期内固定", { minW: 400 }], limited: ["被限流"] } });
  },
  "request-routing": async () => { await fresh(); await click("本地服务"); await click("路由"); await click("请求路由"); await shoot("request-routing"); },
  "agents": async () => {
    await fresh(); await click("本地服务"); await click("Agent 接入"); await click("未接入");
    await shoot("agents", { mark: { codex: ["26.1029", ROW], claude: ["opus · sonnet · haiku", ROW], grok: ["未接入网关", ROW] } });
  },
  "ext-featured": async () => {
    await fresh(); await click("万物皆拓展");
    await shoot("ext-featured", { mark: { chat: ["对话造拓展"], tabs: ["导入 dsh 插件", { minW: 500 }], list: ["会话搜索", { minW: 900, minH: 250 }], guard: ["命令守卫", ROW] } });
  },
  "ext-local": async () => { await fresh(); await click("万物皆拓展"); await click("本地", { selector: "[role=tab]" }); await shoot("ext-local"); },
  "ext-community": async () => { await fresh(); await click("万物皆拓展"); await click("社区"); await shoot("ext-community", { settle: 1500 }); },
  "ext-chat": async () => { await fresh(); await click("万物皆拓展"); await click("对话造拓展"); await shoot("ext-chat"); },
  "usage": async () => {
    await fresh(); await click("用量");
    await shoot("usage", { settle: 1500, mark: { kpi: ["估算费用", { minW: 900 }], chart: ["每天 token", { minW: 900, minH: 300 }], table: ["占比", { minW: 900, minH: 120 }] } });
  },
  "requests": async () => {
    await fresh(); await click("用量"); await click("请求");
    await shoot("requests", { settle: 1500, mark: { limited: ["429", ROW], retry: ["重试", ROW], running: ["进行中", ROW] } });
  },
  "sessions": async () => { await fresh(); await click("会话"); await shoot("sessions", { settle: 1500, mark: { tabs: ["Claude Code", { minW: 300 }] } }); },
  "mcp": async () => { await fresh(); await click("MCP 与 Skills"); await shoot("mcp", { settle: 1200, mark: { servers: ["context7", { minW: 900, minH: 100 }] } }); },
  "codex": async () => { await fresh(); await click("Codex", { selector: "aside button, nav button, [class*=sidebar] button" }); await shoot("codex", { settle: 1200, mark: { mode: ["聚合", { minW: 120 }], login: ["Codex 登录身份", ROW] } }); },
  "codex-plugins": async () => { await fresh(); await click("Codex", { selector: "aside button, nav button, [class*=sidebar] button" }); await click("Codex 插件"); await shoot("codex-plugins", { settle: 1500 }); },
  "codex-multi": async () => { await fresh(); await click("Codex", { selector: "aside button, nav button, [class*=sidebar] button" }); await click("多开"); await shoot("codex-multi", { settle: 1200 }); },
  "claude": async () => { await fresh(); await click("Claude", { selector: "aside button, nav button, [class*=sidebar] button" }); await shoot("claude", { settle: 1200 }); },
  "palette": async () => { await fresh(); await click("本地服务"); await click("搜索页面和操作"); await page.evaluate(`(() => { const input = document.querySelector('[role=dialog] input, input[type=search]'); if (input) { const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; set.call(input, '路由'); input.dispatchEvent(new Event('input', { bubbles: true })); } })()`); await shoot("palette"); },
  "flow": async () => { await fresh("#flow-demo"); await shoot("flow", { settle: 2500 }); },
  "tray": async () => {
    await page.setViewport(400, 640, 2);
    await fresh("#tray-panel"); await shoot("tray", { settle: 1500 });
    await page.setViewport(1440, 900, 2);
  },
};

try {
  for (const [name, run] of Object.entries(SHOTS)) {
    if (only && !only.includes(name)) continue;
    try { await run(); } catch (error) { console.error(`✗ ${name}: ${error.message}`); }
  }
} finally {
  writeFileSync(regionsFile, JSON.stringify(regions, null, 2));
  server.close();
  await page.close();
}
