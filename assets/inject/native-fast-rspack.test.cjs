const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { test } = require("node:test");
const rspack = require("./native-fast-rspack.js");

// 独立合成的 Rspack 工厂，保留四处门控结构，不携带官方应用资产。
const fixtures = () => [
  { key: "UI", source: 'UI(e,t,n){function access(arg){let fallback="local",host=arg?.hostId??fallback,identity=(0,n.account)(host),allowed=identity?.authMethod==="chatgpt"||identity?.authMethod==="personalAccessToken"||identity?.authMethod==="amazonBedrock",method=identity?.authMethod??null;let query={authMethod:method,hostId:host};return{serviceTierAccess:allowed?{fast:true}:null}}e.exports=access}' },
  { key: "Request", source: 'Request(e,t,n){async function access(scope,host){let auth=await (0,n.auth)(scope,host);if("chatgpt"!==auth&&"personalAccessToken"!==auth&&"amazonBedrock"!==auth)return null;let requirements=await (0,n.requirements)(scope,host,{priority:"critical"});return scope.query.setData(n.key,{authMethod:auth,hostId:host},requirements),(0,n.parse)(requirements)}e.exports=access}' },
  { key: "Queue", source: 'Queue(e,t,n){const read=n.read;e.exports={defaults:function(manager,{runtime:runtime},before,changed){let pending=null;return{async readServiceTier(model){for(;null!=pending;)await pending.catch(()=>void 0);return before(),read(manager.requestClient,manager.logger,model,async()=>{if("local"===manager.getHostId()&&await runtime.isUsingCopilotInference())return!1;let result=await manager.getAccount({priority:"critical"});return result.account?.type==="chatgpt"||result.account?.type==="amazonBedrock"})},async writeServiceTier(t,a){}}},explicit:function(manager,{settings:settings,storage:storage,filesystem:filesystem,runtime:runtime},readDefault,other,last){return async(conversation,message)=>{let options=message.submissionOptions,work=message.context.workPermissions,hasWork=null!=work,host=options?.executionHostId??manager.getHostId();let readTier=async()=>{if(null==options)return readDefault(conversation?.latestModel||null);if(null==options.serviceTier)return null;let{account:account}=await manager.getAccount({priority:"critical"});return account?.type==="chatgpt"||account?.type==="amazonBedrock"?options.serviceTier:null},[tier,permissions]=await Promise.all([readTier(),options?.usePermissionSelection??settings.readPermissionRefreshEnabled()]);return tier}}}}' },
];

function load(source, window, dependency) {
  const key = source.slice(0, source.indexOf("("));
  const factory = vm.runInNewContext(`({${source}})`, { window })[key];
  const module = { exports: {} };
  factory(module, module.exports, dependency);
  return module.exports;
}

function context(enabled = true, localApi = false) {
  const window = {};
  // 使用生产身份策略，避免测试重复实现后与实际行为脱节。
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "native-fast-unlock.js"), "utf8"), { window, URL, Error });
  window.__codexPlusNativeFastUnlock.runtime.update({ enhancementsEnabled: enabled, codexAppNativeFastUnlock: true, nativeFastLocalApiConfigured: localApi });
  return window;
}

test("Rspack requires all four gates exactly once across factories", () => {
  const result = rspack.planFactories(fixtures());
  assert.equal(result.ok, true);
  assert.deepEqual(Object.values(result.diagnostics.matches), [1, 1, 1, 1]);
  for (const missing of fixtures()) {
    assert.equal(rspack.planFactories(fixtures().filter(f => f.key !== missing.key)).ok, false);
  }
  const duplicate = { key: "Other", source: fixtures()[0].source.replace(/^UI/, "Other") };
  assert.equal(rspack.planFactories([...fixtures(), duplicate]).diagnostics.reason, "ambiguous-gates");
});

test("Rspack factory discovery keeps originals on re-prepare without executing modules", async () => {
  const source = 'const original=({UI(e,t){throw Error("readServiceTier must not execute")}}).UI;const replacement=({UI(e,t){throw Error("patched factory must not execute")}}).UI;replacement.__codexPlusNativeFastOriginal=original;export const __webpack_require__={m:{UI:replacement,ignored:()=>null}};';
  const found = await rspack.readFactories('data:text/javascript,' + encodeURIComponent(source), 1);
  assert.deepEqual(found, [{ key: "UI", source: 'UI(e,t){throw Error("readServiceTier must not execute")}' }]);
});

test("Rspack cold-start discovery waits for lazy factories without invoking them", async () => {
  const source = `export const __webpack_require__={m:{}};setTimeout(()=>Object.assign(__webpack_require__.m,{${fixtures().map(f=>f.source).join(",")}}),20);`;
  const found = await rspack.readFactories('data:text/javascript,' + encodeURIComponent(source), 5);
  assert.equal(rspack.planFactories(found).ok, true);
});

test("Rspack rejects invalid, shadowed and already patched factories", () => {
  assert.equal(rspack.planFactories([...fixtures(), fixtures()[0]]).ok, false);
  assert.equal(rspack.planFactories([{ key: "UI", source: "Other(e,t){}" }]).ok, false);
  assert.equal(rspack.planFactories([{ key: "UI", source: "UI(e,t){return import.meta.url}" }]).ok, false);
  const factories = fixtures();
  factories[2].source = factories[2].source.replaceAll("account:account", "account:runtime").replaceAll("account?.type", "runtime?.type");
  assert.equal(rspack.planFactories(factories).diagnostics.reason, "shadowed-bindings");
  assert.equal(rspack.planFactories(rspack.planFactories(fixtures()).factories).ok, false);
});

test("Rspack UI and request access preserve auth, host and opt-in boundaries", async () => {
  const factories = rspack.planFactories(fixtures()).factories;
  for (const enabled of [true, false]) for (const localApi of [true, false]) {
    const window = context(enabled, localApi);
    for (const host of ["local", "remote"]) for (const auth of ["apikey", "apiKey", "chatgpt", "personalAccessToken", "amazonBedrock", "copilot", null]) {
      const official = ["chatgpt", "personalAccessToken", "amazonBedrock"].includes(auth);
      const expected = official || enabled && host === "local" && (["apikey", "apiKey"].includes(auth) || auth == null && localApi);
      const ui = load(factories.find(f => f.key === "UI").source, window, { account: () => auth == null ? null : { authMethod: auth } });
      assert.equal(!!ui({ hostId: host }).serviceTierAccess, expected);
      let queries = 0;
      const request = load(factories.find(f => f.key === "Request").source, window, {
        auth: async () => auth, requirements: async () => { queries++; return { fast: true }; }, parse: v => v,
      });
      assert.equal(!!await request({ query: { setData() {} } }, host), expected);
      assert.equal(queries, expected ? 1 : 0);
    }
  }
});

test("Rspack queue gates preserve Standard, priority, ultrafast, Copilot and remote behavior", async () => {
  const source = rspack.planFactories(fixtures()).factories.find(f => f.key === "Queue").source;
  for (const enabled of [true, false]) for (const copilot of [true, false]) for (const host of ["local", "remote"]) {
    const queue = load(source, context(enabled), { read: async (client, logger, model, eligible) => await eligible() ? "priority" : null });
    let reads = 0;
    const manager = { getHostId: () => host, getAccount: async () => { reads++; return { account: { type: "apiKey" } }; } };
    const deps = { runtime: { isUsingCopilotInference: async () => copilot }, settings: { readPermissionRefreshEnabled: () => true } };
    const expected = enabled && host === "local" && !copilot;
    assert.equal(await queue.defaults(manager, deps, () => {}).readServiceTier("model"), expected ? "priority" : null);
    assert.equal(reads, host === "local" && copilot ? 0 : 1);
    const submit = queue.explicit(manager, deps, async () => "inherited");
    for (const tier of [null, "default", "priority", "ultrafast"]) {
      reads = 0;
      assert.equal(await submit({}, { submissionOptions: { serviceTier: tier }, context: {} }), expected ? tier : null);
      assert.equal(reads, tier == null ? 0 : 1);
      assert.equal(await submit({}, { submissionOptions: { serviceTier: tier, executionHostId: "remote" }, context: {} }), null);
    }
    assert.equal(await submit({}, { context: {} }), "inherited");
  }
  const queue = load(source, context(true, true), { read: async (client, logger, model, eligible) => await eligible() ? "priority" : null });
  for (const auth of ["chatgpt", "amazonBedrock", null]) {
    const manager = { getHostId: () => "local", getAccount: async () => ({ account: auth == null ? null : { type: auth } }) };
    const deps = { runtime: { isUsingCopilotInference: async () => false }, settings: { readPermissionRefreshEnabled: () => true } };
    assert.equal(await queue.defaults(manager, deps, () => {}).readServiceTier("model"), "priority");
    assert.equal(await queue.explicit(manager, deps, () => null)({}, { submissionOptions: { serviceTier: "priority" }, context: {} }), "priority");
    if (auth == null) {
      deps.runtime = {};
      assert.equal(await queue.explicit(manager, deps, () => null)({}, { submissionOptions: { serviceTier: "priority" }, context: {} }), null);
    }
  }
});

const runtimeSource = 'const require={m:{},o:(o,k)=>Object.prototype.hasOwnProperty.call(o,k),O(){}};const register=chunk=>{var key,unused,ids=chunk.__rspack_esm_ids,modules=chunk.__webpack_modules__,run=chunk.__rspack_esm_runtime,i=0;for(key in modules)require.o(modules,key)&&(require.m[key]=modules[key]);for(run&&run(require);i<ids.length;i++){}require.O()};require.C=register;import*as support from"./782259.abcdef.js";require.C(support);export{require as __webpack_require__,register};';
const chunkSource = 'export const __rspack_esm_id=782259;export const __rspack_esm_ids=[782259];export const __webpack_modules__={};';

test("Rspack registration replaces duplicate chunk factories without touching other modules", () => {
  const prepared = rspack.prepare(chunkSource, "app://-/assets/782259.abcdef.js", fixtures(), source => ({ source }), source => source);
  assert.equal(prepared.ok, true, JSON.stringify(prepared.diagnostics));
  const source = prepared.modules[0].patchedSource.replaceAll("export const ", "const ") + ";globalThis.install=__rspack_esm_runtime";
  const sandbox = { window: context() };
  vm.runInNewContext(source, sandbox);
  const require = { m: {} };
  const originalMap = require.m;
  sandbox.install(require);
  const register = chunk => { for (const key of Object.keys(chunk.__webpack_modules__)) require.m[key] = chunk.__webpack_modules__[key]; };
  const original = function original() {};
  register({ __rspack_esm_ids: [1], __webpack_modules__: { UI: original, untouched: original } });
  assert.notEqual(require.m.UI, original);
  assert.equal(require.m.UI.__codexPlusNativeFastOriginal, original);
  assert.equal(require.m.untouched, original);
  assert.equal(originalMap.UI, require.m.UI);
  const replacement = require.m.UI;
  register({ __rspack_esm_ids: [2], __webpack_modules__: { UI: original } });
  assert.equal(require.m.UI, replacement);
});

test("Rspack runtime preparation fails closed on missing or ambiguous registration", () => {
  const select = source => rspack.registrationChunk(source, "app://-/assets/123.abc.js");
  assert.equal(select(runtimeSource), "app://-/assets/782259.abcdef.js");
  assert.equal(select(runtimeSource.replace("require.m[key]=modules[key]", "require.m[key]=null")), null);
  assert.equal(select(runtimeSource + runtimeSource), null);
  assert.equal(select(runtimeSource.replace("./782259.abcdef.js", "https://example.com/782259.abcdef.js")), null);
  const prepare = source => rspack.prepare(source, "app://-/assets/782259.abcdef.js", fixtures(), source => ({ source }), source => source);
  assert.equal(prepare(chunkSource + "export const __rspack_esm_runtime=()=>{};").ok, false);
  assert.equal(prepare(chunkSource + "const __codexPlusFastFactories={};").ok, false);
});

test("Rspack document preparation discovers numbered entries and preserves the import base", async () => {
  const window = context();
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "native-fast-rspack.js"), "utf8"), { window, Error, URL });
  const api = window.__codexPlusNativeFastUnlock;
  const url = "app://-/assets/348410.abcdef.js";
  const support = "app://-/assets/782259.abcdef.js";
  const source = runtimeSource + 'function load(id){return import("../"+require.u(id))}const base=import.meta.url;';
  const options = {
    location: new URL("app://-/index.html"), document: { scripts: [{ src: url }] },
    fetchText: async value => value === url ? source : chunkSource + 'const base=import.meta.url;', readRspackFactories: async () => fixtures(),
  };
  const result = await api.prepareDocumentModules(options);
  assert.equal(result.ok, true, JSON.stringify(result.diagnostics));
  assert.equal(result.modules.length, 1);
  assert.equal(result.modules[0].url, support);
  assert.ok(result.modules[0].patchedSource.includes(`const base="${support}"`));
  assert.ok(result.modules[0].patchedSource.includes('window.__codexPlusNativeFastCompiled="v1"'));

  for (const expression of ['import("../"+other.u(id))', 'import("../"+require.u(id+1))', 'import(name)', 'import("../../"+require.u(id))']) {
    const result = await api.prepareDocumentModules({ ...options, fetchText: async value => value === url ? runtimeSource : chunkSource + `function load(id){return ${expression}}` });
    assert.equal(result.ok, false, expression);
    assert.equal(result.diagnostics.reason, "nonliteral-dynamic-import");
    assert.equal(result.modules.length, 0);
  }
  const partial = await api.prepareDocumentModules({ ...options, readRspackFactories: async () => fixtures().slice(0, 2) });
  assert.equal(partial.ok, false);
  assert.equal(partial.modules.length, 0);
  const duplicated = await api.prepareDocumentModules({ ...options, fetchText: async () => runtimeSource, document: { scripts: [{ src: url }, { src: "app://-/assets/123.abc.js" }] } });
  assert.equal(duplicated.diagnostics.reason, "ambiguous-rspack-runtime");
  const unavailable = await api.prepareDocumentModules({ ...options, readRspackFactories: async () => { throw Error("unavailable"); } });
  assert.equal(unavailable.diagnostics.reason, "rspack-modules-unavailable");

  for (const bad of ["https://example.com/assets/123.abc.js", "app://other/assets/123.abc.js", "app://-/assets/async/123.abc.js"]) {
    let fetched = false;
    const result = await api.prepareDocumentModules({ ...options, document: { scripts: [{ src: bad }] }, fetchText: async () => { fetched = true; return source; } });
    assert.equal(result.ok, false);
    assert.equal(fetched, false);
  }
  api.runtime.update({ enhancementsEnabled: false, codexAppNativeFastUnlock: true });
  assert.equal((await api.prepareDocumentModules(options)).diagnostics.reason, "disabled");
});
