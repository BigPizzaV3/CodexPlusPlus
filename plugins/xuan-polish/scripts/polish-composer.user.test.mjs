import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const source = fs.readFileSync(path.join(import.meta.dirname, "polish-composer.user.js"), "utf8");
const routes = [
  ["/prompt-optimize/settings", "/v1/polish/settings", "GET"],
  ["/prompt-optimize/generate", "/v1/polish", "POST"],
  ["/settings/set", "/v1/polish/settings/set", "POST"],
];

function adapter(pageBridge, fetch = () => { throw new Error("不应直连本地端口"); }) {
  const timers = new Map();
  const context = vm.createContext({
    window: {
      __xuanPluginBridge: { "xuan-polish": pageBridge },
      __codexSessionDeleteBridge: () => { throw new Error("不应调用宿主扩展路由"); },
      __XUAN_BRIDGE_TOKEN__: "test-local-token",
      setTimeout(callback) { timers.set(1, callback); return 1; },
      clearTimeout(id) { timers.delete(id); },
    },
    fetch,
    BRIDGE_KEY: "__xuanPluginBridge",
    BRIDGE_TIMEOUT_MS: 75000,
  });
  const start = source.indexOf("  function bridgeCall(");
  const end = source.indexOf("  async function refreshSettings()", start);
  assert.ok(start >= 0 && end > start);
  vm.runInContext(source.slice(start, end), context);
  return { call: context.bridgeCall, timers };
}

function functionSource(name) {
  const start = source.indexOf(`  function ${name}(`);
  const end = source.indexOf("\n  function ", start + 1);
  assert.ok(start >= 0 && end > start, `未找到 ${name}`);
  return source.slice(start, end);
}

function composerAnchorAdapter() {
  class ComposerElement {
    constructor(tagName, { attributes = {}, textContent = "" } = {}) {
      this.tagName = tagName;
      this.attributes = new Map(Object.entries(attributes));
      this.textContent = textContent;
      this.children = [];
      this.parentElement = null;
      this.parentNode = null;
    }

    append(child) {
      child.parentElement = this;
      child.parentNode = this;
      this.children.push(child);
      return child;
    }

    get nextSibling() {
      const siblings = this.parentElement?.children || [];
      return siblings[siblings.indexOf(this) + 1] || null;
    }

    getAttribute(name) {
      return this.attributes.get(name) ?? null;
    }

    hasAttribute(name) {
      return this.attributes.has(name);
    }

    contains(node) {
      return this === node || this.children.some((child) => child.contains(node));
    }

    querySelectorAll() {
      const descendants = [];
      const visit = (node) => {
        for (const child of node.children) {
          descendants.push(child);
          visit(child);
        }
      };
      visit(this);
      return descendants.filter((node) => node.tagName === "BUTTON" || node.getAttribute("role") === "button");
    }

    getBoundingClientRect() {
      return { width: 24, height: 24, top: 0, bottom: 24, left: 0, right: 24 };
    }
  }

  const document = { body: new ComposerElement("BODY") };
  const context = vm.createContext({
    BUTTON_ATTR: "data-cpo-button-test",
    HTMLElement: ComposerElement,
    Element: ComposerElement,
    document,
    window: {
      getComputedStyle: () => ({ display: "block", visibility: "visible", opacity: "1", fontSize: "12px" }),
    },
  });
  const functions = [
    "collapseWs",
    "normalizeText",
    "isVisible",
    "isSendLikeLabel",
    "isAccessPermissionLikeLabel",
    "accessPermissionBeforeSend",
    "controlAfterAnchor",
    "composerInsertAnchor",
  ];
  vm.runInContext(functions.map(functionSource).join("\n"), context);
  return { ComposerElement, document, composerInsertAnchor: context.composerInsertAnchor };
}

test("polish script keeps the original composer workflow in an independent bridge adapter", () => {
  assert.match(source, /MutationObserver/);
  assert.match(source, /\/v1\/polish/);
  assert.match(source, /停止/);
  assert.match(source, /恢复/);
  assert.match(source, /Ctrl\+Enter|ctrlKey/);
  assert.doesNotMatch(source, /127\.0\.0\.1:57324|__codexSessionDeleteBridge/);
  assert.match(source, /润色中|正在润色|loading/);
});

test("polish script keeps Ctrl+Enter as a toggle shortcut", () => {
  assert.match(source, /if \(event\.ctrlKey && !event\.metaKey\) return true;/);
  assert.match(source, /const activeElement = document\.activeElement;/);
  assert.match(source, /function onPromptOptimizeShortcut\(event\) \{\s*if \(runtime\.disposed\) return;/);
});

test("输入框重绘先恢复权限控件时，润色按钮仍可定位", () => {
  const { ComposerElement, document, composerInsertAnchor } = composerAnchorAdapter();
  const composer = document.body.append(new ComposerElement("DIV"));
  const input = composer.append(new ComposerElement("DIV", { attributes: { role: "textbox" } }));
  const permission = composer.append(new ComposerElement("BUTTON", { attributes: { "aria-label": "完全访问" } }));

  const anchorWithoutSend = composerInsertAnchor(input);
  assert.equal(anchorWithoutSend.node, composer);
  assert.equal(anchorWithoutSend.before, null);

  const send = composer.append(new ComposerElement("BUTTON", { attributes: { "aria-label": "Send message" } }));
  const anchorWithSend = composerInsertAnchor(input);
  assert.equal(anchorWithSend.node, composer);
  assert.equal(anchorWithSend.before, send);
  assert.equal(permission.nextSibling, send);
});

test("polish script supports cancellation, restore state and settings without exposing credentials", () => {
  assert.match(source, /optimizeToken|AbortController/);
  assert.match(source, /promptOptimizeState/);
  assert.match(source, /\/v1\/polish\/settings/);
  assert.doesNotMatch(source, /Authorization\s*:/i);
  assert.doesNotMatch(source, /bearer\s+\$?\{/i);
});

test("润色三个接口在直连不可用时都通过页面桥接且原样转发参数", async () => {
  const calls = [];
  const result = { status: "ok", settings: { enabled: true }, text: "修改后的文本" };
  const { call, timers } = adapter((path, payload) => {
    calls.push({ path, payload });
    return Promise.resolve(result);
  });
  for (const [path, route] of routes) {
    const payload = { text: "草稿", enabled: true };
    assert.equal(await call(path, payload), result);
    assert.deepEqual(calls.at(-1), { path: route, payload });
    assert.equal(timers.size, 0);
  }
});

test("润色插件未连接时不回退宿主或 HTTP，也不要求更新启动器", async () => {
  const { call } = adapter(undefined);
  for (const [path] of routes) {
    const result = await call(path, { text: "草稿" });
    assert.equal(result.status, "failed");
    assert.match(result.message, /插件尚未连接/);
    assert.doesNotMatch(result.message, /启动器/);
  }
});

test("润色页面桥接失败不回退重发，并展示中文错误", async () => {
  for (const bridge of [
    () => { throw new Error("Failed to fetch (127.0.0.1:57324)"); },
    () => Promise.reject(new Error("networkerror")),
    () => ({ status: "failed", message: "Unknown bridge path" }),
    () => ({ status: "failed", error: { message: "服务暂不可用" } }),
  ]) {
    const { call, timers } = adapter(bridge);
    const result = await call("/settings/set", { enabled: true });
    assert.equal(result.status, "failed");
    assert.equal(result.error, result.message);
    assert.match(result.message, /\p{Script=Han}/u);
    assert.doesNotMatch(result.message, /Failed to fetch|127\.0\.0\.1|\[object Object\]/);
    assert.equal(timers.size, 0);
  }
});

test("润色业务失败及页面桥接超时均有可读结果并清除计时器", async () => {
  const { call } = adapter(async () => ({ status: "failed", error: { message: "当前配置不可用" } }));
  assert.equal((await call("/prompt-optimize/settings", {})).error, "当前配置不可用");
  const stalled = adapter(() => new Promise(() => {}));
  const pending = stalled.call("/prompt-optimize/generate", { text: "草稿" });
  stalled.timers.get(1)();
  assert.match((await pending).error, /超时/);
  assert.equal(stalled.timers.size, 0);
});
