// Claude 页面拓展在补丁副本里的注入：主进程 hook（assets/claude-localization/hook.js）。
//
// 这条链路横跨三处，任何一处改名都会让拓展「静默失效」：
//   hook.js ↔ claude_localization.rs（注入计划的文件名）
//   hook.js ↔ claude_page_host.rs（helper 凭据文件名、请求头、路由前缀、能发的操作）
//   注入计划 ↔ claude-local（页面运行时和包装由它渲染，hook 只读文件、拼接、执行）
// 前半用字面量钉住契约；后半把 hook.js 放进 vm，配上假的 electron 和一个真的本机 HTTP 服务，
// 走一遍「注入 → 页面调后台 → 答复投回页面」。
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";

const root = path.resolve(import.meta.dirname, "../../..");
const read = (relative: string) => fs.readFileSync(path.join(root, relative), "utf8");
const hookSource = read("assets/claude-localization/hook.js");
const host = read("crates/codex-plus-core/src/claude_page_host.rs");
const localization = read("crates/codex-plus-core/src/claude_localization.rs");

function literal(source: string, pattern: RegExp, label: string): string {
  const match = source.match(pattern);
  assert.ok(match, `找不到${label}`);
  return match[1];
}

test("注入计划的文件名、helper 的凭据和路由在 hook 与 Rust 两边一致", () => {
  const planFile = literal(localization, /const PAGE_INJECT_FILE: &str = "([^"]+)";/, "claude_localization.rs 的 PAGE_INJECT_FILE");
  assert.ok(hookSource.includes(`path.join(DIR, '${planFile}')`), `hook.js 必须读 ${planFile}`);
  const receipt = literal(host, /pub const RECEIPT_FILE: &str = "([^"]+)";/, "claude_page_host.rs 的 RECEIPT_FILE");
  assert.equal(literal(hookSource, /const EXT_RECEIPT_NAME = '([^']+)'/, "hook.js 的 EXT_RECEIPT_NAME"), receipt);
  const header = literal(host, /pub const TOKEN_HEADER: &str = "([^"]+)";/, "TOKEN_HEADER");
  assert.ok(hookSource.toLowerCase().includes(`'${header}': receipt.token`), `hook.js 必须用请求头 ${header} 带令牌`);
  const prefix = literal(host, /pub const ROUTE_PREFIX: &str = "([^"]+)";/, "ROUTE_PREFIX");
  assert.ok(hookSource.includes(`path: '${prefix}' + op`), `hook.js 必须请求 ${prefix}<op>`);
  // 页面能发的操作都得是 helper 认识的；只给 Mods 用的 mods-call 不能从页面发。
  const ops = literal(hookSource, /const EXT_OPS = \[([^\]]+)\]/, "hook.js 的 EXT_OPS").split(",").map((item) => item.trim().replace(/'/g, ""));
  assert.deepEqual(ops, ["call", "events", "stop", "route", "status", "config"]);
  for (const op of ops) assert.ok(host.includes(`"${op}" =>`), `claude_page_host.rs 没有处理 ${op}`);
  assert.ok(host.includes('"mods-call" =>') && !ops.includes("mods-call"));
  // 通道名、运行时、包装都来自计划：hook 里不写死。
  for (const name of ["__codexPlusClaudeCall", "__codexPlusClaudeDeliver", "beginPackage", "codexPlus.extension"]) {
    assert.ok(!hookSource.includes(name), `hook.js 不该写死 ${name}`);
  }
});

type Sent = { method: string; params: Record<string, unknown> };

/** 把 hook.js 装进一个假的 Electron 主进程：一个顶层页面，状态目录在临时文件夹里。 */
function loadHook(state: string, plan: unknown, { loading = false } = {}) {
  const dir = path.join(state, "claude-localization");
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "page-shim.js"), "var LOCALE = 'zh-Hans';");
  fs.writeFileSync(path.join(dir, "page-dictation.js"), "");
  if (plan !== undefined) fs.writeFileSync(path.join(dir, "page-inject.json"), JSON.stringify(plan));
  const sent: Sent[] = [];
  const executed: string[] = [];
  const debuggerListeners: Array<(event: unknown, method: string, params: unknown) => void> = [];
  const pageListeners = new Map<string, Array<(...args: unknown[]) => void>>();
  const defaultSession = { setPermissionRequestHandler() {}, setPermissionCheckHandler() {} };
  const wc = {
    session: defaultSession,
    getURL: () => "https://claude.ai/new",
    isDestroyed: () => false,
    isLoading: () => loading,
    reload() {},
    on(event: string, handler: (...args: unknown[]) => void) {
      pageListeners.set(event, [...(pageListeners.get(event) ?? []), handler]);
    },
    executeJavaScript(code: string) {
      executed.push(code);
      // 页面里语法错误的脚本：executeJavaScript 以那个错误拒绝。
      return code.includes("SYNTAX ERROR HERE") ? Promise.reject(new Error("Unexpected identifier 'ERROR'")) : Promise.resolve(undefined);
    },
    debugger: {
      isAttached: () => false,
      attach() {},
      on(_event: string, handler: (event: unknown, method: string, params: unknown) => void) { debuggerListeners.push(handler); },
      sendCommand(method: string, params: Record<string, unknown> = {}) {
        sent.push({ method, params });
        if (method === "Page.getFrameTree") return Promise.resolve({ frameTree: { frame: { id: "main-frame" } } });
        if (method === "Runtime.evaluate") return Promise.resolve({ result: { value: "{}" } });
        return Promise.resolve({});
      },
    },
  };
  const electron = {
    app: { on() {}, whenReady: () => new Promise(() => {}), getVersion: () => "0.0.0-test", getName: () => "Claude" },
    session: { defaultSession },
    webContents: { getAllWebContents: () => [wc] },
  };
  const nodeRequire = createRequire(import.meta.url);
  const context = vm.createContext({
    require: (name: string) => {
      if (name === "electron") return electron;
      if (name === "./bash-timeout.js") return { beginTimeoutOverrides: () => Promise.resolve({}) };
      // hook.js falls back to the home directory when a local helper receipt is missing.
      if (name === "node:os") return { ...os, homedir: () => state };
      return nodeRequire(name);
    },
    __dirname: dir,
    process, Buffer, console, setTimeout, clearTimeout, URL, Promise, JSON, WeakSet, Map,
    // bash 时限那段和这里无关：给一个已就绪的结果，不让它去动主进程。
    __claudeCnBashTimeoutReady: Promise.resolve({}),
  });
  context.globalThis = context;
  vm.runInContext(hookSource, context, { filename: "hook.js" });
  const emit = (method: string, params: unknown) => debuggerListeners.forEach((handler) => handler({}, method, params));
  const settle = async () => { for (let index = 0; index < 20; index += 1) await new Promise((resolve) => setImmediate(resolve)); };
  return { dir, sent, executed, emit, settle, fire: (event: string) => (pageListeners.get(event) ?? []).forEach((handler) => handler()) };
}

function plan(packageRoot: string, extra: Record<string, unknown> = {}) {
  return {
    version: 1,
    transport: "binding",
    binding: { name: "__testCall", deliver: "__testDeliver" },
    runtime: { key: "runtime", pieces: [{ text: "window.runtimeLoaded = true;" }] },
    legacy: [{ key: "theme/a.js", root: packageRoot, pieces: [{ text: "/*legacy*/(function(){" }, { file: "a.js" }, { text: "})();" }] }],
    packages: [{
      key: "user:demo", slug: "h-demo", id: "demo", root: packageRoot,
      pieces: [{ text: "/*wrap*/style(" }, { file: "a.css", json: true }, { text: ");" }, { file: "main.js" }, { text: "/*end*/" }],
    }],
    ...extra,
  };
}

function fixture() {
  const state = fs.mkdtempSync(path.join(os.tmpdir(), "claude-hook-"));
  const packageRoot = path.join(state, "package");
  fs.mkdirSync(packageRoot);
  fs.writeFileSync(path.join(packageRoot, "a.js"), "window.legacy = 1;");
  fs.writeFileSync(path.join(packageRoot, "a.css"), 'body { content: "x"; }');
  fs.writeFileSync(path.join(packageRoot, "main.js"), "window.demo = 1;");
  return { state, packageRoot, cleanup: () => fs.rmSync(state, { recursive: true, force: true }) };
}

test("hook 照注入计划注入：旧格式扩展在页面脚本之前，运行时和拓展包在页面解析完之后", async () => {
  const { state, packageRoot, cleanup } = fixture();
  try {
    const hook = loadHook(state, plan(packageRoot));
    await hook.settle();
    // 旧格式页面扩展：登记成新文档脚本，并在已加载的页面上补跑。
    const early = hook.sent.filter((item) => item.method === "Page.addScriptToEvaluateOnNewDocument").map((item) => String(item.params.source));
    assert.ok(early.includes("/*legacy*/(function(){window.legacy = 1;})();"), early.join("\n---\n"));
    // 通道名来自计划。
    assert.ok(hook.sent.some((item) => item.method === "Runtime.addBinding" && item.params.name === "__testCall"));
    // 页面已经解析完：运行时在前，然后每个包一段；样式文件成了字符串字面量，脚本原样拼上。
    assert.deepEqual(hook.executed, [
      "window.runtimeLoaded = true;",
      '/*wrap*/style("body { content: \\"x\\"; }");window.demo = 1;/*end*/',
    ]);
    // 刷新页面：再注入一遍，改过的脚本和计划都是现读的。
    fs.writeFileSync(path.join(packageRoot, "main.js"), "window.demo = 2;");
    hook.fire("dom-ready");
    await hook.settle();
    assert.equal(hook.executed.at(-1), '/*wrap*/style("body { content: \\"x\\"; }");window.demo = 2;/*end*/');
    // 拓展都停用了（计划里没有运行时）：页面上什么都不加。
    fs.writeFileSync(path.join(hook.dir, "page-inject.json"), JSON.stringify(plan(packageRoot, { runtime: null, packages: [] })));
    const before = hook.executed.length;
    hook.fire("dom-ready");
    await hook.settle();
    assert.equal(hook.executed.length, before);
  } finally {
    cleanup();
  }
});

test("一个包出错只记到它自己头上，别的包照常注入", async () => {
  const { state, packageRoot, cleanup } = fixture();
  try {
    fs.symlinkSync("/etc/hosts", path.join(packageRoot, "link.js"));
    const packages = [
      { key: "user:syntax", root: packageRoot, pieces: [{ text: "SYNTAX ERROR HERE" }] },
      { key: "user:missing", root: packageRoot, pieces: [{ file: "gone.js" }] },
      { key: "user:escape", root: packageRoot, pieces: [{ file: "../outside.js" }] },
      { key: "user:link", root: packageRoot, pieces: [{ file: "link.js" }] },
      { key: "user:fine", root: packageRoot, pieces: [{ file: "main.js" }] },
    ];
    const hook = loadHook(state, plan(packageRoot, { legacy: [], packages }));
    await hook.settle();
    const failed = hook.executed.filter((code) => code.includes("markFailed("));
    assert.equal(failed.length, 4);
    assert.match(failed[0], /markFailed\("user:syntax","Unexpected identifier 'ERROR'"\)/);
    for (const [index, key] of ["user:missing", "user:escape", "user:link"].entries()) {
      assert.match(failed[index + 1], new RegExp(`markFailed\\("${key}","读不到拓展包里的文件"\\)`));
    }
    assert.equal(hook.executed.at(-1), "window.demo = 1;");
    assert.ok(!hook.executed.some((code) => code.includes("localhost")), "包外的文件（符号链接）不会被读进来");
  } finally {
    cleanup();
  }
});

test("页面调后台：只认顶层页面发来的，转给 Codex++ 的 helper，答复按 id 投回页面", async () => {
  const { state, packageRoot, cleanup } = fixture();
  const seen: Array<{ url?: string; token?: string | string[]; body: unknown }> = [];
  const token = "0f8fad5b-d9cb-469f-a165-70867728950e";
  const helper = http.createServer((request, response) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => {
      seen.push({ url: request.url, token: request.headers["x-codex-plus-claude-ext"], body: JSON.parse(Buffer.concat(chunks).toString("utf8")) });
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify({ ok: true, result: { count: 1 } }));
    });
  });
  helper.listen(0, "127.0.0.1");
  await new Promise((resolve) => helper.once("listening", resolve));
  try {
    const hook = loadHook(state, plan(packageRoot), { loading: true });
    await hook.settle();
    hook.emit("Runtime.executionContextCreated", { context: { id: 1, auxData: { frameId: "main-frame", isDefault: true } } });
    hook.emit("Runtime.executionContextCreated", { context: { id: 2, auxData: { frameId: "child-frame", isDefault: true } } });
    hook.emit("Runtime.executionContextCreated", { context: { id: 3, auxData: { frameId: "main-frame", isDefault: false } } });
    const call = (contextId: number, id: string, op = "call") =>
      hook.emit("Runtime.bindingCalled", { name: "__testCall", executionContextId: contextId, payload: JSON.stringify({ id, op, body: { key: "user:demo", method: "status" } }) });
    const delivered = () => hook.executed.filter((code) => code.includes("__testDeliver"));
    const wait = async (count: number) => { for (let index = 0; index < 200 && delivered().length < count; index += 1) await new Promise((resolve) => setTimeout(resolve, 5)); };

    // helper 没在运行：给出明确的原因。
    call(1, "a");
    await wait(1);
    assert.match(delivered()[0], /__testDeliver"\]\("a",\{"ok":false,"error":"拓展后台由 Codex\+\+ 托管，但它没有在运行[^"]*","code":"host_offline"\}\)/);

    fs.writeFileSync(path.join(state, "claude-ext-host.json"), JSON.stringify({ port: (helper.address() as { port: number }).port, token }));
    call(1, "b");
    await wait(2);
    assert.deepEqual(seen, [{ url: "/claude-ext/v1/call", token, body: { key: "user:demo", method: "status" } }]);
    assert.match(delivered()[1], /__testDeliver"\]\("b",\{"ok":true,"result":\{"count":1\}\}\)/);

    // 子框架、隔离世界、别的 binding、不认识的操作：不转发。
    call(2, "c");
    call(3, "d");
    hook.emit("Runtime.bindingCalled", { name: "__other", executionContextId: 1, payload: JSON.stringify({ id: "e", op: "call", body: {} }) });
    call(1, "f", "mods-call");
    await wait(3);
    assert.equal(seen.length, 1);
    assert.match(delivered()[2], /"f",\{"ok":false,"error":"没有这个接口"\}/);
    assert.equal(delivered().length, 3);
    // 页面换了文档：旧的执行上下文作废。
    hook.emit("Runtime.executionContextsCleared", {});
    call(1, "g");
    await hook.settle();
    assert.equal(seen.length, 1);
  } finally {
    helper.close();
    cleanup();
  }
});

test("没有注入计划时 hook 照常工作，不注册拓展通道", async () => {
  const { state, cleanup } = fixture();
  try {
    const hook = loadHook(state, undefined);
    await hook.settle();
    assert.ok(hook.sent.some((item) => item.method === "Page.addScriptToEvaluateOnNewDocument"), "汉化照常注入");
    assert.ok(!hook.sent.some((item) => item.method === "Page.getFrameTree"));
    assert.deepEqual(hook.executed, []);
  } finally {
    cleanup();
  }
});
