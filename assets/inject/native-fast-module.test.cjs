const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { createRequire } = require("node:module");

const helper = fs.readFileSync(require.resolve("./native-fast-unlock.js"), "utf8");
const enabled = { enhancementsEnabled: true, codexAppNativeFastUnlock: true };

function browser({ existing = false, location = "app://-/index.html", storage = new Map(), supports = true } = {}) {
  const events = [];
  const listeners = new Map();
  const timers = new Map();
  let nextTimer = 0;
  let reloads = 0;
  const document = {
    readyState: "loading",
    documentElement: existing ? { existing: true } : null,
    addEventListener(type, listener, options) {
      const current = listeners.get(type) ?? [];
      current.push({ listener, once: options?.once === true });
      listeners.set(type, current);
    },
    removeEventListener(type, listener) {
      listeners.set(type, (listeners.get(type) ?? []).filter(item => item.listener !== listener));
    },
    createElement(tag) {
      const node = { tag, parentNode: null, children: [], relList: { supports: () => supports } };
      node.appendChild = child => { node.children.push(child); child.parentNode = node; events.push([tag, child]); return child; };
      return node;
    },
    appendChild(node) {
      assert.equal(this.documentElement, null);
      this.documentElement = node;
      node.parentNode = this;
      events.push(["document-append", node]);
      return node;
    },
    removeChild(node) {
      assert.equal(this.documentElement, node);
      this.documentElement = null;
      node.parentNode = null;
      events.push(["document-remove", node]);
      return node;
    },
  };
  const window = {
    document,
    location: { href: location, reload: () => { reloads++; } },
    URL: class extends URL { static createObjectURL() { events.push(["blob"]); return "blob:app://-/fixture"; } },
    Blob,
    HTMLScriptElement: { supports: () => supports },
    setTimeout(callback, delay) { const id = ++nextTimer; timers.set(id, { callback, delay }); return id; },
    clearTimeout(id) { timers.delete(id); },
    sessionStorage: {
      getItem: key => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
      removeItem: key => storage.delete(key),
    },
  };
  const context = vm.createContext({ module: { exports: {} }, window, URL });
  vm.runInContext(helper, context);
  const api = context.module.exports;
  api.runtime.update(enabled);
  const domContentLoaded = () => {
    document.readyState = "interactive";
    for (const item of [...(listeners.get("DOMContentLoaded") ?? [])]) {
      if (item.once) document.removeEventListener("DOMContentLoaded", item.listener);
      item.listener();
    }
  };
  const runTimers = () => {
    for (const [id, timer] of [...timers]) { timers.delete(id); timer.callback(); }
  };
  return { api, window, events, storage, timers, listeners, domContentLoaded, runTimers, reloads: () => reloads };
}

function moduleReferences(parser, source) {
  const ast = parser.parse(source, { sourceType: "module", createImportExpressions: true });
  const references = { static: [], dynamic: [], meta: 0 };
  const pending = [ast];
  while (pending.length) {
    const node = pending.pop();
    if (!node || typeof node !== "object") continue;
    if (node.type === "ImportExpression") references.dynamic.push(node.source.value ?? node.source.quasis?.[0]?.value.cooked);
    if ((node.type === "ImportDeclaration" || node.type?.startsWith("Export")) && node.source) references.static.push(node.source.value);
    if (node.type === "MetaProperty" && node.meta.name === "import") references.meta++;
    for (const [key, value] of Object.entries(node)) {
      if (["loc", "extra", "comments", "leadingComments", "trailingComments", "innerComments"].includes(key)) continue;
      if (Array.isArray(value)) pending.push(...value);
      else if (value && typeof value === "object") pending.push(value);
    }
  }
  return references;
}

async function runDiscoveryTests(source, url) {
  const index = new URL("./index-fixture.js", url).href;
  const main = new URL("./app-main-fixture.js", url).href;
  const initialImport = 'import "./app-initial-fixture.js";';
  const mainImport = 'import(`./app-main-fixture.js`);';
  const document = { scripts: [{ src: index }] };
  const location = new URL("app://-/index.html");

  // 旧入口直接引用 app-initial；26.1027 经 app-main 引用同一目标块。
  for (const entry of [initialImport, mainImport, `${mainImport}${initialImport}`]) {
    const env = browser();
    const sources = new Map([
      [index, entry],
      [main, `${initialImport}import "./app-runtime-unrelated.js";import "./unrelated.js";`],
      [url, source],
    ]);
    const fetched = [];
    const result = await env.api.prepareDocumentModules({
      document, location,
      fetchText: async target => { fetched.push(target); return sources.get(target) ?? null; },
    });
    assert.equal(result.ok, true, JSON.stringify(result.diagnostics));
    assert.equal(result.diagnostics.rewriters.fast, "patched");
    assert.equal(result.modules.length, 1, "中间模块只用于发现，不改写或额外预加载");
    assert.equal(result.modules[0].url, url);
    assert.ok(result.modules[0].patchedSource.includes('__codexPlusNativeFastCompiled="v1"'));
    assert.deepEqual(fetched, entry === initialImport ? [index, url] : [index, main, url], "去重且不遍历无关模块");
    assert.equal(env.events.length, 0, "准备模块不改变当前文档");
  }

  const duplicate = new URL("./app-initial-duplicate.js", url).href;
  for (const failure of ["missing-main", "fetch-failed", "missing-initial", "incomplete-gates", "duplicate-initial"]) {
    const env = browser();
    const result = await env.api.prepareDocumentModules({
      document, location,
      fetchText: async target => {
        if (target === index) return mainImport;
        if (target === main) {
          if (failure === "missing-main") return null;
          if (failure === "fetch-failed") throw new Error("fixture fetch failed");
          return `${initialImport}${failure === "duplicate-initial" ? 'import "./app-initial-duplicate.js";' : ""}`;
        }
        if (target === url) {
          if (failure === "missing-initial") return null;
          return failure === "incomplete-gates" ? "export const changed = true;" : source;
        }
        return target === duplicate ? source : null;
      },
    });
    assert.equal(result.ok, false, failure);
    assert.equal(result.modules.length, 0, "发现或匹配失败时不留下半套补丁");
    assert.equal(env.events.length, 0);
  }

  for (const settings of [{ ...enabled, enhancementsEnabled: false }, { ...enabled, codexAppNativeFastUnlock: false }]) {
    const env = browser();
    env.api.runtime.update(settings);
    let fetched = false;
    const result = await env.api.prepareDocumentModules({
      document, location, fetchText: async () => { fetched = true; return source; },
    });
    assert.equal(result.diagnostics.reason, "disabled");
    assert.equal(fetched, false, "关闭任一开关后不发现或改写模块");
  }
}

async function runNativeFastModuleTests({ source, url }) {
  await runDiscoveryTests(source, url);
  const fixture = [
    'import dependency from "./static.js"; import "./side.js";',
    'export { dependency } from "../export.js"; export * from "./star.js";',
    'const dynamic = import("./dynamic.js"); const escaped = import("./\\u0061.js");',
    'const tick = import(`./tick.js`, { with: { type: "json" } });',
    'const object = { import: () => import("./property.js"), import() { return import("./method.js") }, export() {} }; object.import; object?.import;',
    'const template = `raw import("./untouched.js") import.meta.url ${import("./nested.js")} ${`${import.meta.url}`}`;',
    'const text = "import(\'./text.js\') import.meta.url";',
    'const pattern = /import\\("\\.\\/regex.js"\\)|import.meta.url/v;',
    'if (true) /import\\("regex"\\)|import.meta.url/.test(text);',
    'const division = 1 / 2 / 3; const objectDivision = ({ value: 1 }) / 2;',
    '// import("./comment.js"); import.meta.url',
    '/* import("./block-comment.js"); import.meta.url */',
    source,
  ].join("\n");
  const env = browser();
  const prepared = env.api.prepareModule(fixture, url);
  assert.equal(prepared.ok, true, JSON.stringify(prepared.diagnostics));
  assert.equal(prepared.diagnostics.staticImports, 4);
  assert.equal(prepared.diagnostics.dynamicImports, 6);
  assert.equal(prepared.diagnostics.importMetaUrls, 1);
  assert.ok(prepared.patchedSource.includes('const text = "import(\'./text.js\') import.meta.url";'));
  assert.ok(prepared.patchedSource.includes('/import\\("\\.\\/regex.js"\\)|import.meta.url/v'));
  assert.ok(prepared.patchedSource.includes('raw import("./untouched.js") import.meta.url'));
  assert.ok(prepared.patchedSource.includes('// import("./comment.js"); import.meta.url'));
  assert.ok(prepared.patchedSource.includes(`//# sourceURL=${url}\n`));
  assert.ok(prepared.patchedSource.includes('window.__codexPlusNativeFastCompiled="v1";'));

  const menu = fs.readFileSync(path.join(__dirname, "renderer-inject/20-menu.js"), "utf8");
  const candidateFunctions = ["uniqueCodexAppAssetUrls", "codexAppAssetCandidateUrls"].map(name => {
    const found = menu.match(new RegExp(`  function ${name}\\([^]*?\\n  }`));
    assert.ok(found, `${name} must remain available`);
    return found[0];
  }).join("\n");
  const assetWindow = { __codexPlusNativeFastCompiled: "v1", __codexPlusNativeFastModuleDiagnostics: { url } };
  const candidates = () => vm.runInNewContext(`${candidateFunctions};codexAppAssetCandidateUrls()`, {
    window: assetWindow, document: { scripts: [], querySelectorAll: () => [] },
    performance: { getEntriesByType: () => [{ name: "blob:app://-/fixture" }] },
  });
  assert.equal(candidates().join(), url, "Blob 不在普通资源候选中时仍能定位原生 client 资产");
  assetWindow.__codexPlusNativeFastCompiled = undefined;
  assert.equal(candidates().length, 0, "未执行补丁时不发布模块候选");

  // 纯 cargo test 不要求先安装 manager 的前端依赖；本机完整验证可额外核对 AST。
  let parser;
  try { parser = createRequire(path.resolve(__dirname, "../../apps/codex-plus-manager/package.json"))("@babel/parser"); }
  catch (error) { if (process.env.CODEX_NATIVE_FAST_MODULE_SOURCE) throw error; }
  if (parser) {
    const before = moduleReferences(parser, fixture);
    const after = moduleReferences(parser, prepared.patchedSource);
    assert.equal(after.meta, 0);
    assert.deepEqual(after.static, before.static.map(value => new URL(value, url).href));
    assert.deepEqual(after.dynamic, before.dynamic.map(value => new URL(value, url).href));
  }

  for (const unsupported of ['import("./" + name)', 'import(name)', 'import.meta.resolve("./x")']) {
    const rejected = env.api.prepareModule(`${source}\n${unsupported};`, url);
    assert.equal(rejected.ok, false);
    assert.equal(rejected.patchedSource, `${source}\n${unsupported};`);
  }
  assert.equal(env.api.prepareModule(source, "https://example.com/assets/app.js").ok, false);
  env.api.runtime.update({ ...enabled, enhancementsEnabled: false });
  assert.equal(env.api.prepareModule(fixture, url).diagnostics.reason, "disabled");
  assert.equal(env.api.installModule(prepared.patchedSource, url).diagnostics.reason, "disabled");
  assert.equal(env.events.length, 0);
  env.api.runtime.update(enabled);

  const installed = env.api.installModule(prepared.patchedSource, url);
  assert.equal(installed.ok, true, JSON.stringify(installed.diagnostics));
  assert.equal(env.window.document.documentElement, null, "原 parser 继续接管文档，临时 html 必须同步移除");
  const nodes = env.events.filter(([kind]) => kind === "html").map(([, node]) => node);
  assert.deepEqual(nodes.map(node => node.tag), ["script", "link"]);
  assert.equal(nodes[0].type, "importmap");
  assert.equal(JSON.parse(nodes[0].textContent).imports[url], "blob:app://-/fixture");
  assert.equal(nodes[1].rel, "modulepreload");
  assert.equal(nodes[1].href, "blob:app://-/fixture");
  assert.equal(env.window.__codexPlusNativeFastCompiled, undefined, "预加载不能伪造原生模块已执行标记");
  JSON.stringify(installed);

  nodes[1].onerror();
  assert.equal(env.reloads(), 1);
  assert.equal(env.window.__codexPlusNativeFastPatchState, "failed");
  const fallback = browser({ storage: env.storage });
  assert.equal(fallback.api.installModule(prepared.patchedSource, url).diagnostics.reason, "module-preload-failed-open");
  assert.equal(fallback.events.length, 0, "失败后的下一文档直接加载原模块");
  assert.equal(fallback.reloads(), 0, "不形成重载循环");

  const executionFailure = browser();
  assert.equal(executionFailure.api.installModule(prepared.patchedSource, url).ok, true);
  const preloaded = executionFailure.events.find(([kind, node]) => kind === "html" && node.tag === "link")[1];
  preloaded.onload?.();
  assert.equal(executionFailure.timers.size, 0, "DOMContentLoaded 之前不判断目标模块执行失败");
  executionFailure.domContentLoaded();
  assert.equal(executionFailure.timers.size, 1, "预加载成功但模块执行 throw 时仍有完成标记守护");
  assert.equal([...executionFailure.timers.values()][0].delay, 30000);
  assert.equal(executionFailure.listeners.has("error"), false, "不把其他脚本的异常当成目标模块失败");
  executionFailure.runTimers();
  assert.equal(executionFailure.window.__codexPlusNativeFastModuleDiagnostics.reason, "module-execution-timeout");
  assert.equal(executionFailure.reloads(), 1);
  preloaded.onerror();
  assert.equal(executionFailure.reloads(), 1, "同一文档的重复失败只恢复一次");
  const executionFallback = browser({ storage: executionFailure.storage });
  assert.equal(executionFallback.api.installModule(prepared.patchedSource, url).ok, false);
  assert.equal(executionFallback.events.length, 0, "执行失败的下一文档同样加载原模块");

  for (const beforeDomReady of [true, false]) {
    const success = browser();
    assert.equal(success.api.installModule(prepared.patchedSource, url).ok, true);
    if (!beforeDomReady) { success.domContentLoaded(); assert.equal(success.timers.size, 1); }
    success.window.__codexPlusNativeFastCompiled = "v1";
    success.window.__codexPlusNativeFastModuleComplete();
    if (beforeDomReady) success.domContentLoaded();
    assert.equal(success.timers.size, 0, "目标模块执行成功立即撤销守护");
    assert.equal(success.window.__codexPlusNativeFastModuleComplete, undefined);
    success.runTimers();
    assert.equal(success.reloads(), 0);
  }

  const existing = browser({ existing: true });
  assert.equal(existing.api.installModule(prepared.patchedSource, url).diagnostics.reason, "document-already-started");
  assert.equal(existing.events.length, 0);
  assert.equal(existing.window.document.documentElement.existing, true);
  const foreign = browser({ location: "https://example.com/" });
  assert.equal(foreign.api.installModule(prepared.patchedSource, url).ok, false);
  assert.equal(foreign.events.length, 0);
  const unavailable = browser({ supports: false });
  assert.equal(unavailable.api.installModule(prepared.patchedSource, url).ok, false);
  assert.equal(unavailable.events.length, 0);
  const file = browser({ location: "file:///tmp/native-fast-test.html" });
  const filePrepared = file.api.prepareModule(source, "file:///tmp/assets/app.js");
  assert.equal(filePrepared.ok, true);
  assert.equal(file.api.installModule(filePrepared.patchedSource, "file:///tmp/assets/app.js").ok, true);

  // 真实解包源码是本机可选证据，不把安装文件或大体积 fixture 收入仓库。
  const nativeSource = process.env.CODEX_NATIVE_FAST_MODULE_SOURCE;
  if (nativeSource) {
    const original = fs.readFileSync(nativeSource, "utf8");
    const nativePrepared = env.api.prepareModule(original, url);
    assert.equal(nativePrepared.ok, true, JSON.stringify(nativePrepared.diagnostics));
    const originalReferences = moduleReferences(parser, original);
    const patchedReferences = moduleReferences(parser, nativePrepared.patchedSource);
    assert.equal(nativePrepared.diagnostics.staticImports, originalReferences.static.length);
    assert.equal(nativePrepared.diagnostics.dynamicImports, originalReferences.dynamic.length);
    assert.equal(nativePrepared.diagnostics.importMetaUrls, originalReferences.meta);
    assert.equal(patchedReferences.meta, 0);
    assert.deepEqual(patchedReferences.static, originalReferences.static.map(value => new URL(value, url).href));
    assert.deepEqual(patchedReferences.dynamic, originalReferences.dynamic.map(value => new URL(value, url).href));
  }
  return "Native Fast module base rewrite, lexical boundaries, preload, document isolation and one-shot fallback passed";
}

module.exports = runNativeFastModuleTests;
