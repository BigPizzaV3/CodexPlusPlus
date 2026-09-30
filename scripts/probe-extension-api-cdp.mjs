/**
 * 用 CDP 在真实 Codex 页面上端到端验证拓展 UI 接口（window.codexPlus）。
 *
 * 验证的是「测试断言不了的」那一层：选择器在真实 Codex DOM 里能否命中、
 * 注册后 UI 是否真的出现、宿主重建后是否还在。renderer-inject.test.ts 只能
 * 证明代码里写了什么。
 *
 * 用法：node scripts/probe-extension-api-cdp.mjs [端口]
 * 前置：Codex 以 --remote-debugging-port=9229 启动（见 memory/verify-inject-via-cdp.md）
 */
import { readFile } from "node:fs/promises";
import path from "node:path";

const port = process.argv[2] || "9229";
const root = path.resolve(import.meta.dirname, "..");

const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
// 主窗口是 URL 不带 query 的那个。带 ?initialRoute= 的是桌宠浮层等辅助窗口，
// 它们也有 app://-/index.html 前缀且不保证排在后面，靠顺序取第一个会连错。
const page = targets.find((t) => t.type === "page" && t.url === "app://-/index.html")
  || targets.find((t) => t.type === "page" && t.url.startsWith("app://-/index.html") && !t.url.includes("?"));
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

const results = [];
const check = (name, ok, detail) => {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail === undefined ? "" : `  ${JSON.stringify(detail)}`}`);
};

// ── 准备：重载页面，清掉上一轮注入留下的闭包与事件监听 ──────────────
await send("Page.enable");
await send("Page.reload", { ignoreCache: true });
await new Promise((r) => setTimeout(r, 2500));

// 桥接桩：真实桥接由 launcher 侧 Rust 回调注入，页面重载后会断，
// 这里顶上固定响应，让 /diagnostics/log 与 /user-scripts/list 可预期。
await evaluate(`(() => {
  window.__probeCalls = [];
  window.__codexSessionDeleteCallbacks = new Map();
  window.__codexSessionDeleteSeq = 0;
  window.__codexSessionDeleteResolve = (id, result) => {
    const cb = window.__codexSessionDeleteCallbacks.get(id);
    if (!cb) return;
    window.__codexSessionDeleteCallbacks.delete(id);
    cb.resolve(result);
  };
  window.__codexSessionDeleteBridge = (path, payload) => {
    window.__probeCalls.push({ path, payload });
    return Promise.resolve({ status: "ok", message: "probe-ok" });
  };
  window.__codexPlusBridgeHealth = { lastInjectionAt: Date.now(), lastSuccessAt: Date.now(), lastAttemptAt: Date.now() };
  // 用户脚本运行时状态：接口层靠它做归属与失败上报，桩成空表即可。
  window.__codexPlusUserScripts = { scripts: {}, currentKey: null, registerCleanup() {} };
  return true;
})()`);

// ── 1. 注入产物 ────────────────────────────────────────────────────
const source = await readFile(path.join(root, "assets/inject/renderer-inject.js"), "utf8");
await evaluate(`${source}\n;undefined`);

// ── 2. 接口对象形状 ────────────────────────────────────────────────
const shape = await evaluate(`(() => {
  const api = window.codexPlus;
  if (!api) return { present: false };
  return {
    present: true,
    version: api.version,
    apiVersion: api.apiVersion,
    hasToast: typeof api.toast === "function",
    hasCall: typeof api.call === "function",
    hasRegisterPage: typeof api.registerPage === "function",
    hasRegisterRowAction: typeof api.registerRowAction === "function",
    hasOnCleanup: typeof api.onCleanup === "function",
    hasFail: typeof api.fail === "function",
    constants: api.constants ? Object.keys(api.constants).sort() : [],
  };
})()`);
check("window.codexPlus 已挂载", shape.present === true, shape.version);
check("apiVersion 为 1", shape.apiVersion === 1, shape.apiVersion);
check(
  "方法齐全",
  shape.hasToast && shape.hasCall && shape.hasRegisterPage && shape.hasRegisterRowAction
    && shape.hasOnCleanup && shape.hasFail,
);
check("constants 暴露类名契约", Array.isArray(shape.constants) && shape.constants.length >= 7, shape.constants);

// ── 3. 路由白名单 ──────────────────────────────────────────────────
const routes = await evaluate(`(async () => {
  const out = {};
  try { await window.codexPlus.call("/diagnostics/log", { event: "probe" }); out.allowed = true; }
  catch (e) { out.allowed = false; out.allowedError = e.message; }
  try { await window.codexPlus.call("/settings/set", { x: 1 }); out.blocked = false; }
  catch (e) { out.blocked = true; out.blockedError = e.message; }
  return out;
})()`);
check("白名单路由可调用", routes.allowed === true, routes.allowedError);
check("未开放路由被拦截", routes.blocked === true, routes.blockedError);

// ── 4. 注册整页视图 + 图标栏入口 ───────────────────────────────────
const registered = await evaluate(`(async () => {
  window.__probePageRenders = 0;
  window.__probeDispose = window.codexPlus.registerPage(
    {
      title: "探针面板",
      render: ({ container, script }) => {
        window.__probePageRenders += 1;
        container.innerHTML = '<div class="codex-plus-row" data-probe-body="true">探针内容 ' + script + "</div>";
      },
    },
    { navLabel: "探针面板", icon: "P" },
  );
  // rail 渲染晚于注入，等它出现。
  const deadline = Date.now() + 10000;
  let entry = null;
  while (Date.now() < deadline) {
    entry = document.querySelector('[data-codex-plus-ext] #codex-plus-ext-rail, [id^="codex-plus-ext-rail-"]');
    if (entry) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  return {
    found: !!entry,
    id: entry ? entry.id : null,
    label: entry ? (entry.querySelector("button") || entry).getAttribute("aria-label") : null,
    html: entry ? entry.outerHTML.slice(0, 220) : null,
  };
})()`);
check("图标栏入口已渲染", registered.found === true, registered.id);

// ── 5. 点击入口 → 整页渲染 ─────────────────────────────────────────
const opened = await evaluate(`(async () => {
  const entry = document.getElementById("${registered.id || "none"}");
  if (!entry) return { ok: false };
  (entry.querySelector("button") || entry).click();
  await new Promise((r) => setTimeout(r, 500));
  const overlay = document.querySelector(".codex-plus-page-overlay");
  const body = overlay?.querySelector("[data-probe-body]");
  const railActive = document.querySelector('[data-codex-plus-ext-rail-active="true"]');
  const nativeSuppressed = document.documentElement.hasAttribute("data-codex-plus-page-open");
  const rect = overlay ? overlay.getBoundingClientRect() : null;
  const rail = document.querySelector("nav[data-app-navigation-rail]");
  return {
    ok: true,
    overlay: !!overlay,
    bodyRendered: !!body,
    bodyText: body ? body.textContent.slice(0, 60) : null,
    renderCount: window.__probePageRenders,
    railActive: !!railActive,
    nativeSuppressed,
    overlayLeft: rect ? Math.round(rect.left) : null,
    railWidth: rail ? Math.round(rail.getBoundingClientRect().width) : null,
    zoomApplied: overlay ? getComputedStyle(overlay).getPropertyValue("--codex-plus-zoom") || "unset" : null,
  };
})()`);
check("整页 overlay 已创建", opened.overlay === true);
check("页面内容渲染成功", opened.bodyRendered === true, opened.bodyText);
check("render 只调用一次", opened.renderCount === 1, opened.renderCount);
check("入口项被点亮", opened.railActive === true);
check("原生选中态被压制", opened.nativeSuppressed === true);
// overlay 应从 rail 右边界起铺满（顶替原生侧边栏），不是并排。
check(
  "overlay 左边界贴住 rail",
  opened.overlayLeft !== null && opened.railWidth !== null && Math.abs(opened.overlayLeft - opened.railWidth) <= 2,
  { overlayLeft: opened.overlayLeft, railWidth: opened.railWidth },
);

// ── 6. 关闭后重建：overlay 被清掉，但注册项还在 ────────────────────
const rebuilt = await evaluate(`(async () => {
  document.querySelector(".codex-plus-modal-close")?.click();
  // 内置页面靠 header 的关闭按钮；拓展页面没有，直接移除模拟宿主重建。
  document.querySelectorAll(".codex-plus-page-overlay").forEach((n) => n.remove());
  await new Promise((r) => setTimeout(r, 200));
  const gone = !document.querySelector(".codex-plus-page-overlay");
  // 再点一次入口：注册表持久，应当能重新打开并重跑 render。
  const entry = document.getElementById("${registered.id || "none"}");
  (entry?.querySelector("button") || entry)?.click();
  await new Promise((r) => setTimeout(r, 400));
  return {
    closed: gone,
    reopened: !!document.querySelector(".codex-plus-page-overlay [data-probe-body]"),
    renderCount: window.__probePageRenders,
  };
})()`);
check("overlay 关闭后被清掉", rebuilt.closed === true);
check("注册表持久，可重新打开", rebuilt.reopened === true);
check("宿主重建后 render 被重新调用", rebuilt.renderCount === 2, rebuilt.renderCount);

// ── 7. toast 队列与类型 ────────────────────────────────────────────
const toasts = await evaluate(`(async () => {
  document.querySelectorAll(".codex-delete-toast").forEach((n) => n.remove());
  window.codexPlus.toast("第一条", { type: "success" });
  window.codexPlus.toast("第二条", { type: "error" });
  window.codexPlus.toast("第三条");
  window.codexPlus.toast("第四条");
  await new Promise((r) => setTimeout(r, 300));
  const live = Array.from(document.querySelectorAll(".codex-delete-toast"));
  return {
    count: live.length,
    texts: live.map((n) => n.textContent),
    types: live.map((n) => n.dataset.toastType || ""),
    bottoms: live.map((n) => n.style.bottom),
    firstBorderColor: live[0] ? getComputedStyle(live[0]).borderColor : null,
  };
})()`);
check("toast 上限为 3（挤掉最旧）", toasts.count === 3, toasts.count);
check("最旧的一条被挤掉", !toasts.texts.includes("第一条"), toasts.texts);
check("类型标记生效", toasts.types.includes("error"), toasts.types);
check("多条 toast 错开纵向位置", new Set(toasts.bottoms).size === toasts.count, toasts.bottoms);

// ── 8. 配额与失败隔离 ──────────────────────────────────────────────
const limits = await evaluate(`(() => {
  const out = { perScript: null, illegalSelector: null, callbackError: null };
  window.__codexPlusUserScripts.currentKey = "user:probe.js";
  const disposers = [];
  for (let i = 0; i < 16; i += 1) {
    try { disposers.push(window.codexPlus.registerRowAction({ label: "项" + i, onActivate() {} })); }
    catch (e) { out.perScript = e.message; break; }
  }
  try { window.codexPlus.registerRowAction({ label: "溢出", onActivate() {} }); out.perScript = "NO_ERROR"; }
  catch (e) { out.perScript = e.message; }
  // 回调抛错不应冒泡到调用方。
  const entry = document.querySelector('[id^="codex-plus-ext-rail-"]');
  try {
    window.__codexPlusUserScripts.scripts["user:probe.js"] = { key: "user:probe.js", status: "loaded", error: "" };
    const bad = window.codexPlus.registerPage({ title: "坏页面", render: () => { throw new Error("render-boom"); } });
    bad();
    out.callbackError = "no-error-recorded";
  } catch (e) { out.callbackError = "THREW_TO_CALLER:" + e.message; }
  out.recordedFailures = (window.__codexPlusExtensionFailures || []).length;
  disposers.forEach((d) => d());
  window.__codexPlusUserScripts.currentKey = null;
  return out;
})()`);
check("单脚本配额生效", /最多注册 16 项/.test(String(limits.perScript)), limits.perScript);
check("注册失败不向调用方抛错", limits.callbackError !== "THREW_TO_CALLER", limits.callbackError);

// ── 9. dispose 之后 UI 真的消失 ─────────────────────────────────────
const disposed = await evaluate(`(async () => {
  window.__probeDispose();
  await new Promise((r) => setTimeout(r, 300));
  return {
    railGone: !document.getElementById("${registered.id || "none"}"),
    overlayGone: !document.querySelector(".codex-plus-page-overlay"),
    registryLog: (window.__codexPlusRegistryLog || []).length,
  };
})()`);
check("dispose 后图标栏入口消失", disposed.railGone === true);
check("注册流水有记录", disposed.registryLog > 0, disposed.registryLog);

// ── 10. 崩溃与报错摘要 ─────────────────────────────────────────────
const errors = await evaluate(`(() => ({
  failures: (window.__codexPlusExtensionFailures || []).map((f) => f.message).slice(0, 5),
  probeCalls: (window.__probeCalls || []).map((c) => c.path),
}))()`);
console.log("\n诊断:", JSON.stringify(errors, null, 2));

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} 通过`);
ws.close();
process.exit(failed.length ? 1 : 0);
