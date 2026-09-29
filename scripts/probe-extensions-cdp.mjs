/**
 * 用 CDP 核对「拓展」页真实 DOM 与布局尺寸。
 *
 * 只做两件事：注入当前产物 assets/inject/renderer-inject.js，然后读回
 * 左列表条目与右侧详情面板的实际结构/计算样式。不写任何状态。
 *
 * 用法：node scripts/probe-extensions-cdp.mjs [端口]
 */
import { readFile } from "node:fs/promises";
import path from "node:path";

const port = process.argv[2] || "9229";
const root = path.resolve(import.meta.dirname, "..");

const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = targets.find((t) => t.type === "page" && t.url.startsWith("app://-/index.html"));
if (!page) throw new Error("未找到 app://-/index.html 页面目标");

const ws = new WebSocket(page.webSocketDebuggerUrl);
let nextId = 0;
const pending = new Map();

ws.addEventListener("message", (event) => {
  const msg = JSON.parse(event.data);
  const resolver = pending.get(msg.id);
  if (!resolver) return;
  pending.delete(msg.id);
  if (msg.error) resolver.reject(new Error(JSON.stringify(msg.error)));
  else resolver.resolve(msg.result);
});

const send = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });

await new Promise((resolve) => ws.addEventListener("open", resolve, { once: true }));

async function evaluate(expression) {
  const result = await send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
    userGesture: true,
  });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.exception?.description || "evaluate 抛错");
  }
  return result.result.value;
}

// 1. 先重载页面，清掉上一轮注入留下的 IIFE 闭包与事件监听。
//    不重载就再注一次，会同时挂着新旧两套 handler，rail 点一下开一次关一次。
await send("Page.enable");
await send("Page.reload", { ignoreCache: true });
await new Promise((r) => setTimeout(r, 2500));

// 2. 装一个 fixture 桥接桩。
//    真实桥接是 launcher 侧 Rust 回调，页面重载后就断了，看门狗要等一轮才补，
//    期间拓展页只会显示「桥接不可用」。这里用固定数据顶上，专门把布局渲染出来。
await evaluate(`(() => {
  const marketScripts = [
    {
      id: "translator",
      name: "翻译助手",
      description: "在会话里选中文本即可调用翻译，支持多语言互译与整段润色。",
      version: "1.4.0",
      author: "BigPizzaV3",
      tags: ["翻译"],
      requirements: ["需要配置翻译服务"],
      limitations: ["纯离线环境不可用"],
      icon: "",
      installed: false,
      installedVersion: "",
      updateAvailable: false,
    },
    {
      id: "with-icon",
      name: "带图标的插件",
      description: "清单里给了 icon 字段的条目，走 img 分支。",
      version: "0.2.0",
      author: "社区",
      tags: [],
      icon: "data:image/svg+xml;utf8,<svg xmlns=\\"http://www.w3.org/2000/svg\\" viewBox=\\"0 0 16 16\\"><rect width=\\"16\\" height=\\"16\\" fill=\\"%234a9eff\\"/></svg>",
      installed: false,
      installedVersion: "",
      updateAvailable: false,
    },
  ];
  const inventory = {
    status: "ok",
    message: "",
    scripts: [
      { key: "user:translator.js", name: "翻译助手", enabled: true, market_id: "translator", version: "1.4.0", description: "在会话里选中文本即可调用翻译。" },
      { key: "user:local-only.js", name: "本地脚本", enabled: false, description: "只有本地清单里有的脚本。" },
    ],
  };
  window.__codexSessionDeleteCallbacks = new Map();
  window.__codexSessionDeleteSeq = 0;
  window.__codexSessionDeleteResolve = (id, result) => {
    const cb = window.__codexSessionDeleteCallbacks.get(id);
    if (!cb) return;
    window.__codexSessionDeleteCallbacks.delete(id);
    cb.resolve(result);
  };
  window.__codexSessionDeleteBridge = (path) => Promise.resolve(
    path === "/script-market/list"
      ? { status: "ok", message: "", indexUrl: "", updatedAt: "2026-09-30", scripts: marketScripts }
      : path === "/user-scripts/list" || path === "/user-scripts/load"
        ? inventory
        : path === "/settings/get"
          ? { status: "ok", settings: { userScriptsEnabled: true, enhancementsEnabled: true } }
          : { status: "ok" },
  );
  window.__codexPlusBridgeHealth = { lastInjectionAt: Date.now(), lastSuccessAt: Date.now(), lastAttemptAt: Date.now() };
  return true;
})()`);

// 3. 注入当前产物
const source = await readFile(path.join(root, "assets/inject/renderer-inject.js"), "utf8");
await evaluate(`${source}\n;undefined`);
console.log("已注入产物，长度", source.length);

// 3. 等 rail 入口挂上。注入后入口不是同步出现的，原生导航栏要等 React 渲染到位。
const opened = await evaluate(`(async () => {
  const deadline = Date.now() + 10000;
  let rail = null;
  while (Date.now() < deadline) {
    rail = document.getElementById("codex-plus-rail-extensions");
    if (rail) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  if (!rail) {
    const ids = Array.from(document.querySelectorAll("[id^='codex-plus-rail']")).map((n) => n.id);
    return { ok: false, railIds: ids };
  }
  // 入口是 DIV 包着一个原生 button，真正的点击目标与状态载体是里面那个 button。
  const target = rail.querySelector("button") || rail;
  target.click();
  return { ok: true, tag: target.tagName, label: target.getAttribute("aria-label") };
})()`);
console.log("点击 rail 入口:", JSON.stringify(opened));

// 4. 给渲染一点时间再读结构
await new Promise((r) => setTimeout(r, 800));

// 5. 选中第一条，把右侧详情面板也渲染出来，才能一并核对。
const selected = await evaluate(`(async () => {
  const deadline = Date.now() + 8000;
  let first = null;
  while (Date.now() < deadline) {
    first = document.querySelector(".codex-plus-page-nav-item");
    if (first) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  if (!first) return { ok: false };
  first.click();
  await new Promise((r) => setTimeout(r, 400));
  return { ok: true, label: first.getAttribute("data-codex-extensions-select") };
})()`);
console.log("选中条条目:", JSON.stringify(selected));

const report = await evaluate(`(() => {
  const out = { rows: [], detail: null, layout: null };

  const layout = document.querySelector(".codex-plus-page-layout");
  if (layout) {
    const nav = layout.querySelector(".codex-plus-page-nav");
    const main = layout.querySelector(".codex-plus-page-main");
    const lw = layout.getBoundingClientRect();
    out.layout = {
      totalWidth: Math.round(lw.width),
      navWidth: nav ? Math.round(nav.getBoundingClientRect().width) : null,
      mainWidth: main ? Math.round(main.getBoundingClientRect().width) : null,
      flexDirection: getComputedStyle(layout).flexDirection,
    };
  }

  const rows = document.querySelectorAll(".codex-plus-page-nav-item");
  rows.forEach((row, index) => {
    if (index >= 4) return;
    const body = row.querySelector(".codex-plus-extensions-item-body");
    const icon = row.querySelector(".codex-plus-extensions-icon");
    const name = row.querySelector(".codex-plus-extensions-item-name");
    const desc = row.querySelector(".codex-plus-extensions-item-description");
    const footer = row.querySelector(".codex-plus-extensions-item-footer");
    const publisher = row.querySelector(".codex-plus-extensions-item-publisher");
    const button = row.querySelector(".codex-plus-extensions-item-button");
    const img = row.querySelector(".codex-plus-extensions-icon-img");
    const svg = row.querySelector(".codex-plus-extensions-icon svg");
    const rect = row.getBoundingClientRect();
    out.rows.push({
      active: row.getAttribute("data-active"),
      height: Math.round(rect.height),
      hasBody: !!body,
      bodyFlexDirection: body ? getComputedStyle(body).flexDirection : null,
      iconSize: icon ? Math.round(icon.getBoundingClientRect().width) : null,
      iconKind: img ? "img" : (svg ? "svg" : "none"),
      nameWeight: name ? getComputedStyle(name).fontWeight : null,
      nameText: name ? name.textContent.trim().slice(0, 40) : null,
      descriptionText: desc ? desc.textContent.trim().slice(0, 50) : null,
      footerHeight: footer ? Math.round(footer.getBoundingClientRect().height) : null,
      publisherText: publisher ? publisher.textContent.trim().slice(0, 30) : null,
      buttonText: button ? button.textContent.trim() : null,
    });
  });

  const detail = document.querySelector("[data-codex-plus-extensions-detail]");
  if (detail) {
    const head = detail.querySelector(".codex-plus-extensions-detail-head");
    const title = detail.querySelector(".codex-plus-extensions-detail-title");
    const body = detail.querySelector(".codex-plus-extensions-detail-body");
    out.detail = {
      hasHead: !!head,
      hasBody: !!body,
      title: title ? title.textContent.trim() : null,
      iconSize: (() => {
        const el = detail.querySelector(".codex-plus-extensions-detail-icon svg, .codex-plus-extensions-detail-icon img");
        return el ? Math.round(el.getBoundingClientRect().width) : null;
      })(),
      actions: Array.from(detail.querySelectorAll(".codex-plus-extensions-detail-actions button")).map((b) => b.textContent.trim()),
    };
  }
  return out;
})()`);

console.log(JSON.stringify(report, null, 2));
ws.close();
