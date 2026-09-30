/**
 * 用 CDP 在真实 Codex 页面上验证拓展菜单项（registerMenuItem）。
 *
 * 补的是 renderer-inject.test.ts 够不到的一层：菜单是打开时一次性构建的
 * innerHTML，注册发生在菜单打开前后行为不同，只有真跑起来才能确认。
 *
 * 用法：node scripts/probe-extension-menu-cdp.mjs [端口]
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
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail === undefined ? "" : `  ${JSON.stringify(detail)}`}`);
};

await send("Page.enable");
await send("Page.reload", { ignoreCache: true });
await new Promise((r) => setTimeout(r, 2500));

await evaluate(`(() => {
  window.__codexSessionDeleteCallbacks = new Map();
  window.__codexSessionDeleteSeq = 0;
  window.__codexSessionDeleteResolve = (id, result) => {
    const cb = window.__codexSessionDeleteCallbacks.get(id);
    if (!cb) return;
    window.__codexSessionDeleteCallbacks.delete(id);
    cb.resolve(result);
  };
  window.__codexSessionDeleteBridge = (path) => Promise.resolve(
    path === "/settings/get"
      ? { status: "ok", settings: { enhancementsEnabled: true, userScriptsEnabled: true } }
      : { status: "ok", message: "" });
  window.__codexPlusBridgeHealth = { lastInjectionAt: Date.now(), lastSuccessAt: Date.now(), lastAttemptAt: Date.now() };
  window.__codexPlusUserScripts = { scripts: { "user:menu-probe.js": { key: "user:menu-probe.js", status: "loaded", error: "" } }, currentKey: null, registerCleanup() {} };
  return true;
})()`);

await evaluate(`${await readFile(path.join(root, "assets/inject/renderer-inject.js"), "utf8")}\n;undefined`);

// rail 渲染晚于注入（实测 3~5 秒），不等它的话下面点击入口会落空，
// 表现为「菜单打不开」，实际只是还没渲染出来。
const navReady = await evaluate(`(async () => {
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    if (document.getElementById("codex-plus-rail-nav")
      || document.getElementById("codex-plus-sidebar-nav")) return true;
    await new Promise((r) => setTimeout(r, 300));
  }
  return false;
})()`);
check("导航入口已渲染", navReady === true);

// ── 1. 菜单未打开时注册：打开后应当出现 ─────────────────────────────
const beforeOpen = await evaluate(`(async () => {
  window.__toggleState = false;
  window.__activations = 0;
  window.__disposeMenu = window.codexPlus.registerMenuItem(
    { label: "探针开关", description: "一个开关", onChange: (v) => { window.__toggleState = v; }, toggleValue: () => window.__toggleState },
    { id: "probe-toggle" },
  );
  window.__disposeButton = window.codexPlus.registerMenuItem(
    { label: "探针按钮", description: "一个动作", buttonLabel: "执行", onActivate: () => { window.__activations += 1; } },
    { id: "probe-button" },
  );
  // 打开 Codex++ 菜单
  document.getElementById("codex-plus-rail-nav")?.querySelector("button")?.click();
  await new Promise((r) => setTimeout(r, 600));
  const panel = document.querySelector('[data-codex-plus-panel="home"]');
  const block = panel?.querySelector("[data-codex-plus-ext-menu]");
  return {
    menuOpened: !!panel,
    blockPresent: !!block,
    rowCount: block ? block.querySelectorAll("[data-codex-plus-ext-row]").length : 0,
    toggle: !!block?.querySelector("[data-codex-plus-ext-setting]"),
    action: !!block?.querySelector("[data-codex-plus-ext-action]"),
    labels: block ? Array.from(block.querySelectorAll(".codex-plus-row-title")).map((n) => n.textContent) : [],
    // 拓展项必须排在所有内置项之后
    lastBuiltinBeforeExt: (() => {
      if (!block || !panel) return null;
      const all = Array.from(panel.querySelectorAll(".codex-plus-row"));
      const extIdx = all.findIndex((r) => r.hasAttribute("data-codex-plus-ext-row"));
      const builtinsAfter = all.slice(extIdx).filter((r) => !r.hasAttribute("data-codex-plus-ext-row"));
      return builtinsAfter.length;
    })(),
  };
})()`);
check("菜单可打开", beforeOpen.menuOpened === true);
check("拓展块已渲染", beforeOpen.blockPresent === true);
check("两个拓展项都在", beforeOpen.rowCount === 2, beforeOpen.rowCount);
check("开关与按钮控件都渲染", beforeOpen.toggle && beforeOpen.action);
check("标题正确", JSON.stringify(beforeOpen.labels) === JSON.stringify(["探针开关", "探针按钮"]), beforeOpen.labels);
check("拓展项排在所有内置项之后", beforeOpen.lastBuiltinBeforeExt === 0, beforeOpen.lastBuiltinBeforeExt);

// ── 2. 开关可切换 ──────────────────────────────────────────────────
const toggled = await evaluate(`(async () => {
  const btn = document.querySelector("[data-codex-plus-ext-setting]");
  const before = btn.getAttribute("data-enabled");
  btn.click();
  await new Promise((r) => setTimeout(r, 200));
  return { before, after: btn.getAttribute("data-enabled"), state: window.__toggleState, aria: btn.getAttribute("aria-pressed") };
})()`);
check("开关初始为关", toggled.before === "false", toggled.before);
check("点击后翻转为开", toggled.after === "true", toggled.after);
check("onChange 收到新值", toggled.state === true);
check("aria-pressed 同步", toggled.aria === "true");

// ── 3. 动作按钮可点击 ──────────────────────────────────────────────
const activated = await evaluate(`(async () => {
  const btn = document.querySelector("[data-codex-plus-ext-action]");
  btn.click();
  await new Promise((r) => setTimeout(r, 200));
  return { count: window.__activations };
})()`);
check("onActivate 被调用", activated.count === 1, activated.count);

// ── 4. 菜单已打开时注册：应当立刻出现 ──────────────────────────────
const late = await evaluate(`(async () => {
  window.codexPlus.registerMenuItem({ label: "后注册项", buttonLabel: "走", onActivate: () => {} }, { id: "probe-late" });
  await new Promise((r) => setTimeout(r, 300));
  const block = document.querySelector("[data-codex-plus-ext-menu]");
  return {
    rowCount: block ? block.querySelectorAll("[data-codex-plus-ext-row]").length : 0,
    hasLate: !!document.querySelector('[data-codex-plus-ext-row*="probe-late"]'),
    blocks: document.querySelectorAll("[data-codex-plus-ext-menu]").length,
  };
})()`);
check("已打开的菜单会即时补上", late.hasLate === true);
check("补上后共三项", late.rowCount === 3, late.rowCount);
check("没有重复的拓展块", late.blocks === 1, late.blocks);

// ── 5. 菜单重新打开时状态从 toggleValue 重读 ───────────────────────
const reopened = await evaluate(`(async () => {
  document.querySelector(".codex-plus-modal-close")?.click();
  await new Promise((r) => setTimeout(r, 300));
  document.getElementById("codex-plus-rail-nav")?.querySelector("button")?.click();
  await new Promise((r) => setTimeout(r, 600));
  const btn = document.querySelector("[data-codex-plus-ext-setting]");
  return { present: !!btn, enabled: btn?.getAttribute("data-enabled") };
})()`);
check("重新打开后拓展项仍在", reopened.present === true);
check("开关状态从 toggleValue 恢复", reopened.enabled === "true", reopened.enabled);

// ── 6. dispose 后从菜单移除 ────────────────────────────────────────
const disposed = await evaluate(`(async () => {
  window.__disposeMenu();
  window.__disposeButton();
  await new Promise((r) => setTimeout(r, 300));
  // 按 id 精确定位，不要用 [data-codex-plus-ext-action] —— probe-late 也是动作
  // 按钮，取「第一个」会拿到它，断言就失去意义了。
  const rowOf = (id) => document.querySelector('[data-codex-plus-ext-row$="' + id + '"]');
  return {
    menuGone: !rowOf("probe-toggle"),
    buttonGone: !rowOf("probe-button"),
    lateStillThere: !!rowOf("probe-late"),
    blockStillThere: !!document.querySelector("[data-codex-plus-ext-menu]"),
  };
})()`);
check("dispose 后开关消失", disposed.menuGone === true);
check("dispose 后按钮消失", disposed.buttonGone === true);
check("未 dispose 的项保留", disposed.lateStillThere === true);

// ── 7. 回调抛错不冒泡，且记入失败通道 ──────────────────────────────
// 先让菜单重新打开一次，把剩下那项拿回来
const resilient = await evaluate(`(async () => {
  document.querySelector(".codex-plus-modal-close")?.click();
  await new Promise((r) => setTimeout(r, 300));
  document.getElementById("codex-plus-rail-nav")?.querySelector("button")?.click();
  await new Promise((r) => setTimeout(r, 600));
  window.__codexPlusUserScripts.currentKey = "user:menu-probe.js";
  window.codexPlus.registerMenuItem(
    { label: "会炸的项", buttonLabel: "炸", onActivate: () => { throw new Error("menu-boom"); } },
    { id: "probe-boom" },
  );
  await new Promise((r) => setTimeout(r, 300));
  const before = (window.__codexPlusExtensionFailures || []).length;
  // 同样要精确定位到 boom 那一行：页面上此刻有多个拓展项。
  const boomRow = document.querySelector('[data-codex-plus-ext-row$="probe-boom"]');
  const btn = boomRow?.querySelector("[data-codex-plus-ext-action]");
  if (!btn) return { noBoomButton: true };
  let threw = false;
  try { btn.click(); } catch { threw = true; }
  await new Promise((r) => setTimeout(r, 200));
  window.__codexPlusUserScripts.currentKey = null;
  const after = (window.__codexPlusExtensionFailures || []).length;
  return {
    threwToCaller: threw,
    failuresDelta: after - before,
    lastMessage: (window.__codexPlusExtensionFailures || []).slice(-1)[0]?.message || "",
    menuStillUsable: !!document.querySelector("[data-codex-plus-panel='home']"),
  };
})()`);
check("回调抛错不冒泡到点击处理", resilient.threwToCaller === false);
check("失败被记入通道", resilient.failuresDelta > 0, resilient.lastMessage);
check("菜单在报错后仍可用", resilient.menuStillUsable === true);

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} 通过`);
ws.close();
process.exit(failed.length ? 1 : 0);
