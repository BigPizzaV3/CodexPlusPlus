const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

// 来自已解包的 app-initial；只缩短队列门控之后与测试无关的执行逻辑。
const ui = function Mxo(e){let t=(0,Nxo.c)(10),n=q(gk),r=e?.hostId??n,i=OT(r),a=i?.authMethod===`chatgpt`||i?.authMethod===`personalAccessToken`,o=i?.authMethod??null,s;t[0]!==r||t[1]!==o?(s={authMethod:o,hostId:r},t[0]=r,t[1]=o,t[2]=s):s=t[2];let{data:c,isPending:l}=Eu(nC,s),u=!!i?.isLoading||a&&l,d;t[3]!==c||t[4]!==u||t[5]!==a?(d=a&&!u&&c!=null?Tr(c):null,t[3]=c,t[4]=u,t[5]=a,t[6]=d):d=t[6];let f=d,p;return t[7]!==u||t[8]!==f?(p={serviceTierAccess:f,isLoading:u},t[7]=u,t[8]=f,t[9]=p):p=t[9],p}.toString();
const request = async function S$s(e,t){let n=await v$s(e,t);if(n!==`chatgpt`&&n!==`personalAccessToken`)return null;let r=await kSn(e,t,{priority:`critical`});return e.query.setData(nC,{authMethod:n,hostId:t},r),Tr(r)}.toString();
const requestTier = async function x$s(e,t,n,r){try{let i=await S$s(e,t);if(r!==void 0)return Xi(null,r,i);let a=e.get(lar,t);if(a.type!==`fromConfig`)return Xi(null,Cir(a,null),i);let{config:o}=await k_o(e,t,{includeLayers:!1,cwd:null},{priority:`critical`}),s=Eb(o);return s.service_tier==null?Xi(await C$s(e,t,n??s.model),s.service_tier,i):Xi(null,s.service_tier,i)}catch(e){return fn.error(`Failed to read service tier for request`,{safe:{},sensitive:{error:e}}),null}}.toString();
const savedTier = async function zin(e,t,n,r){try{if(!await r())return null;let t=await e.sendRequest(`configRequirements/read`,void 0,{priority:`critical`}),i=Tr(t),{config:a}=await e.sendRequest(`config/read`,{includeLayers:!1,cwd:null},{priority:`critical`}),o=Eb(a);if(o.service_tier!=null)return Xi(null,o.service_tier,i);let{data:s}=await e.sendRequest(`model/list`,{includeHidden:!0,cursor:null,limit:100},{priority:`critical`}),c=n??o.model,l=c==null?s.find(e=>e.isDefault):s.find(e=>e.model===c||e.id===c);return Xi(l,null,i)}catch(e){return t.error(`Failed to read saved service tier for request`,{safe:{},sensitive:{error:e}}),null}}.toString();
const queueDefault = function Vin(e,{runtime:t,storage:n},r,i){let a=null;return{async readServiceTier(i){for(;a!=null;)await a.catch(()=>void 0);return r(),zin(e.requestClient,e.logger,i,async()=>e.getHostId()===`local`&&await n.readGlobalState(`use-copilot-auth-if-available`)===!0&&await t.isCopilotApiAvailable()?!1:(await e.getAccount({priority:`critical`})).account?.type===`chatgpt`)},async writeServiceTier(t,n){let o=a,s=(async()=>(await o?.catch(()=>void 0),r(),e.sendRequest(`config/batchWrite`,{edits:[{keyPath:n==null?`service_tier`:`profiles.${n}.service_tier`,value:t??`default`,mergeStrategy:`upsert`}],filePath:null,expectedVersion:null,reloadUserConfig:!0})))();a=s;try{return await s}finally{a===s&&(a=null,i())}},writeModel(t,n,r){return e.sendRequest(`config/batchWrite`,a1t(t,n,r))},readShellEnvironmentPolicy(e,t){return Promise.resolve(null)}}}.toString();
const queueExplicit = function Mfn(e,{settings:t,storage:n,filesystem:r,runtime:i},a,o,s){return async(c,l)=>{let u=l.submissionOptions,d=l.context.workPermissions,f=d!=null,p=u?.executionHostId??e.getHostId();if(Aln(l.context,p,i.accountId(),i.threadMode(p)===`durable`))return{status:`paused`,reason:Nln};let m=e.getConversation(c),h=tx(m?.threadStartKind,m?.threadSource),g=u?.collaborationMode??null;u!=null&&h&&m!=null&&(g={...m.latestCollaborationMode,mode:g?.mode??m.latestCollaborationMode.mode});let _=e.getConversationCwd(c)??l.cwd,v=l.context.workspaceRoots??(e.needsResume(c)?await r.readThreadWorkspaceRoots?.(c):m?.currentPermissions?.runtimeWorkspaceRoots??[_]);if(v==null||!v.every(e=>e.length>0))return{status:`paused`,reason:`workspace-unavailable`};let[y,b]=await Promise.all([(async()=>{if(u==null)return a(m?.latestModel||null);if(u.serviceTier==null)return null;let{account:t}=await e.getAccount({priority:`critical`});return t?.type===`chatgpt`?u.serviceTier:null})(),u?.usePermissionSelection??t.readPermissionRefreshEnabled()]);return{serviceTier:y,executionHostId:p}}}.toString();

// 26.10 起的结构：官方门控追加 amazonBedrock，默认队列改成 if/return 语句体。
// 函数名沿用旧夹具，便于复用同一套运行环境。
const uiBedrock = function Mxo(e){let t=(0,Nxo.c)(10),n=q(gk),r=e?.hostId??n,i=OT(r),a=i?.authMethod===`chatgpt`||i?.authMethod===`personalAccessToken`||i?.authMethod===`amazonBedrock`,o=i?.authMethod??null,s;t[0]!==r||t[1]!==o?(s={authMethod:o,hostId:r},t[0]=r,t[1]=o,t[2]=s):s=t[2];let{data:c,isPending:l}=Eu(nC,s),u=!!i?.isLoading||a&&l,d;t[3]!==c||t[4]!==u||t[5]!==a?(d=a&&!u&&c!=null?Tr(c):null,t[3]=c,t[4]=u,t[5]=a,t[6]=d):d=t[6];let f=d,p;return t[7]!==u||t[8]!==f?(p={serviceTierAccess:f,isLoading:u},t[7]=u,t[8]=f,t[9]=p):p=t[9],p}.toString();
const requestBedrock = async function S$s(e,t){let n=await v$s(e,t);if(n!==`chatgpt`&&n!==`personalAccessToken`&&n!==`amazonBedrock`)return null;let r=await kSn(e,t,{priority:`critical`});return e.query.setData(nC,{authMethod:n,hostId:t},r),Tr(r)}.toString();
const queueDefaultBedrock = function Vin(e,{runtime:t,storage:n},r,i){let a=null;return{async readServiceTier(i){for(;a!=null;)await a.catch(()=>void 0);return r(),zin(e.requestClient,e.logger,i,async()=>{if(e.getHostId()===`local`&&await n.readGlobalState(`use-copilot-auth-if-available`)===!0&&await t.isCopilotApiAvailable())return!1;let r=await e.getAccount({priority:`critical`});return r.account?.type===`chatgpt`||r.account?.type===`amazonBedrock`})},async writeServiceTier(t,n){let o=a,s=(async()=>(await o?.catch(()=>void 0),r(),e.sendRequest(`config/batchWrite`,{edits:[{keyPath:n==null?`service_tier`:`profiles.${n}.service_tier`,value:t??`default`,mergeStrategy:`upsert`}],filePath:null,expectedVersion:null,reloadUserConfig:!0})))();a=s;try{return await s}finally{a===s&&(a=null,i())}},writeModel(t,n,r){return e.sendRequest(`config/batchWrite`,a1t(t,n,r))},readShellEnvironmentPolicy(e,t){return Promise.resolve(null)}}}.toString();
const queueExplicitBedrock = queueExplicit.replace("return t?.type===`chatgpt`?", "return t?.type===`chatgpt`||t?.type===`amazonBedrock`?");

const shapes = [
  { name: "legacy", fixtures: [ui, request, queueDefault, queueExplicit], bedrockNative: false },
  { name: "bedrock", fixtures: [uiBedrock, requestBedrock, queueDefaultBedrock, queueExplicitBedrock], bedrockNative: true },
];

// 原生 requirements 和 tier 解析函数也直接保留，验证补丁没有绕开它们。
const access = function Uat(e){let t=e.requirements?.featureRequirements,n=t?.fast_mode!==!1;return{fast:n,ultrafast:(`supportsIndependentSpeedModes`in e&&e.supportsIndependentSpeedModes===!0||n)&&t?.ultrafast_mode!==!1}};
const permitted = function Wat(e,t){if(e==null||e===eot)return!0;if(t==null)return!1;let n=Gat(e);return e===`flex`||t[n??`fast`]};
const speedKind = function Gat(e,t){let n=t?.trim().toLowerCase();return e===`priority`||e===`fast`||n===`fast`?`fast`:e===$at||n===`ultrafast`?`ultrafast`:null};
const modelTier = function Kat(e,t){return t==null?null:t===`fast`?qat(e):e?.serviceTiers?.find(e=>e.id===t)??null};
const fastTier = function qat(e){return e?.serviceTiers?.find(e=>Gat(e.id,e.name)===`fast`||e.name.trim().toLowerCase()===`priority`)??null};
const tierId = function Yat(e,t){return Kat(e,t)?.id??null};
const selectedTier = function Xat(e,t,n={fast:!0,ultrafast:!0}){let r=t??Yat(e,e?.defaultServiceTier??null);return r===eot||!Wat(r,n)?null:r};

const sourceOf = fixtures => [...fixtures, savedTier, requestTier].join("\n");
const source = sourceOf(shapes[0].fixtures);
const url = "app://-/assets/app-initial-fixture.js";
const helper = fs.readFileSync(require.resolve("./native-fast-unlock.js"), "utf8");
const enabled = { enhancementsEnabled: true, codexAppNativeFastUnlock: true };

function environment(patch = true, source = sourceOf(shapes[0].fixtures)) {
  const state = {
    host: "local", auth: "apikey", accountType: "apiKey", authLoading: false,
    requirements: { requirements: null }, requirementsPending: false,
    config: { model: "model", service_tier: "priority" }, override: { type: "fromConfig" },
    model: { model: "model", isDefault: true, defaultServiceTier: "fast", serviceTiers: [{ id: "priority", name: "Fast" }] },
    copilotPreferred: false, copilotAvailable: false, accountMissing: false, accountReads: 0, requirementsReads: 0, publications: 0,
  };
  const sendRequest = async method => {
    if (method === "configRequirements/read") { state.requirementsReads++; return state.requirements; }
    if (method === "config/read") return { config: state.config };
    if (method === "model/list") return { data: [state.model] };
    throw new Error(`Unexpected fixture request: ${method}`);
  };
  const manager = {
    getHostId: () => state.host,
    getAccount: async () => { state.accountReads++; return { account: state.accountMissing ? null : { type: state.accountType } }; },
    getConversation: () => null, getConversationCwd: () => "/fixture", needsResume: () => false,
    requestClient: { sendRequest }, logger: { error() {} }, sendRequest,
  };
  const scope = {
    get: () => state.override,
    query: { setData: () => { state.publications++; } },
  };
  const context = vm.createContext({
    module: { exports: {} }, window: {},
    Nxo: { c: () => [] }, gk: {}, nC: {}, lar: {}, q: () => state.host,
    OT: () => ({ authMethod: state.auth, isLoading: state.authLoading }),
    Eu: () => ({ data: state.requirements, isPending: state.requirementsPending }),
    v$s: async () => state.auth,
    kSn: async () => { state.requirementsReads++; return state.requirements; },
    k_o: async () => ({ config: state.config }), C$s: async () => state.model, Eb: value => value,
    Cir: (override, config) => override.type === "custom" ? override.serviceTier : override.type === "standard" ? "default" : config,
    Aln: () => false, tx: () => false, Nln: "fixture", fn: { error() {} },
  });
  vm.runInContext(helper, context);
  const api = context.module.exports;
  api.runtime.update(enabled);
  const result = api.transform(source, url);
  assert.equal(result.ok, true, JSON.stringify(result.diagnostics));
  const shared = [access, permitted, speedKind, modelTier, fastTier, tierId, selectedTier].map(fn => fn.toString()).join("\n");
  vm.runInContext(`const eot=\`default\`,$at=\`ultrafast\`;${shared};const Tr=Uat,Xi=Xat;\n${patch ? result.patchedSource : source};globalThis.fixtures={Mxo,S$s,x$s,Vin,Mfn};`, context);
  const fns = context.fixtures;
  const queueRuntime = {
    isCopilotApiAvailable: async () => state.copilotAvailable,
    accountId: () => "fixture", threadMode: () => "default",
  };
  const queue = () => fns.Vin(manager, { runtime: queueRuntime, storage: { readGlobalState: async () => state.copilotPreferred } }, () => {}, () => {});
  const explicit = async (serviceTier, executionHostId = state.host, hasOptions = true) => {
    const prepare = fns.Mfn(manager, {
      settings: { readPermissionRefreshEnabled: () => false },
      storage: { readGlobalState: async () => state.copilotPreferred }, filesystem: {}, runtime: queueRuntime,
    }, async () => "saved-default", () => "id", () => {});
    return prepare("thread", {
      submissionOptions: hasOptions ? { serviceTier, executionHostId, usePermissionSelection: false } : undefined,
      context: { workspaceRoots: ["/fixture"] }, cwd: "/fixture",
    });
  };
  return { api, state, fns, manager, scope, queue, explicit, result, context };
}

async function runShape({ name, fixtures, bedrockNative }) {
  const source = sourceOf(fixtures);
  const env = environment(true, source);
  const { api, state, fns, result } = env;
  assert.deepEqual(Object.values(result.diagnostics.matches), [1, 1, 1, 1]);
  assert.equal(result.replacements.length, 4);
  assert.equal(result.patchedSource.split("\n").length, source.split("\n").length);
  for (const replacement of result.replacements) {
    assert.ok(!/[\r\n]/.test(source.slice(replacement.start, replacement.end)));
    assert.ok(!/[\r\n]/.test(replacement.text));
  }
  const renamed = source.replace(/\b(Mxo|Vin|Mfn)\b|S\$s/g, name => `renamed_${name.replaceAll("$", "_")}`);
  assert.equal(api.plan(renamed, url).ok, true, "压缩函数改名后仍按结构定位");
  const renamedLocals = source.replace(/(?<![.\w$])\b(e|t|n|r|i|a|o|s|c|l|u|d|f|p|m|h|g|v|y|b)\b/g, name => `_${name}`);
  assert.equal(api.plan(renamedLocals, url).ok, true, "局部绑定改名后仍按引用关系定位");
  const unicode = api.plan(`// 🥷 中文\n${source}`, url);
  assert.equal(unicode.ok, true);
  assert.equal(unicode.replacements[0].start, result.replacements[0].start + "// 🥷 中文\n".length);

  for (const fixture of fixtures) {
    const missingSource = source.replace(fixture, "");
    const missing = api.transform(missingSource, url);
    assert.equal(missing.ok, false, "缺少任一门控时不允许部分应用");
    assert.equal(missing.patchedSource, missingSource);
    assert.equal(missing.replacements.length, 0);
    const duplicateSource = `${source}\n${fixture}`;
    const duplicate = api.transform(duplicateSource, url);
    assert.equal(duplicate.ok, false, "任一门控多匹配时不允许部分应用");
    assert.equal(duplicate.diagnostics.reason, "ambiguous-gates");
    assert.equal(duplicate.patchedSource, duplicateSource);
  }
  assert.equal(api.plan(source.replace("serviceTierAccess:f", "otherAccess:f"), url).ok, false);
  assert.equal(api.plan(source, "").ok, false);
  assert.equal(api.plan(null, url).ok, false);
  assert.equal(api.transform(result.patchedSource, url).patchedSource, result.patchedSource, "重复拦截不能重复插入补丁");

  for (const host of ["remote", "durable", "LOCAL", null]) assert.equal(api.runtime.allow(host, "apikey"), false);
  for (const auth of ["chatgpt", "personalAccessToken", "copilot", "amazonBedrock", "ApiKey", null]) assert.equal(api.runtime.allow("local", auth), false);
  assert.equal(api.runtime.allow("local", "apikey"), true);
  assert.equal(api.runtime.allow("local", "apiKey"), true);
  for (const settings of [null, {}, { ...enabled, enhancementsEnabled: false }, { ...enabled, codexAppNativeFastUnlock: false }, { ...enabled, codexAppNativeFastUnlock: "true" }]) {
    api.runtime.update(settings);
    assert.equal(api.runtime.allow("local", "apikey"), false, "两个开关都必须明确开启");
    assert.equal(fns.Mxo().serviceTierAccess, null);
  }
  api.runtime.update(enabled);
  env.context.window.__codexPlusSettings = { ...enabled, codexAppNativeFastUnlock: false };
  assert.equal(api.runtime.allow("local", "apikey"), false, "每次读取最新设置，关闭后立即停止放行");
  api.runtime.update(enabled);

  assert.equal(environment(false, source).fns.Mxo().serviceTierAccess, null, `${name}: 原生 API key UI 确实被门控`);
  assert.equal(fns.Mxo().serviceTierAccess.fast, true);
  state.authLoading = true;
  assert.equal(fns.Mxo().serviceTierAccess, null);
  assert.equal(fns.Mxo().isLoading, true);
  state.authLoading = false;
  state.requirementsPending = true;
  assert.equal(fns.Mxo().serviceTierAccess, null);
  assert.equal(fns.Mxo().isLoading, true);
  state.requirementsPending = false;
  state.requirements = { requirements: { featureRequirements: { fast_mode: false, ultrafast_mode: false } } };
  assert.equal(fns.Mxo().serviceTierAccess.fast, false, "保留 fast_mode 管理限制");
  assert.equal(await fns.x$s(env.scope, "local", "model", "priority"), null);
  assert.equal(await env.queue().readServiceTier("model"), null);
  state.requirements = { requirements: null };

  assert.equal(await fns.x$s(env.scope, "local", "model", "priority"), "priority");
  assert.equal(await fns.x$s(env.scope, "local", "model", "default"), null, "Standard 请求保持默认值");
  state.config.service_tier = null;
  assert.equal(await fns.x$s(env.scope, "local", "model"), "priority", "使用模型声明的 Fast tier id");
  state.model = { model: "model", isDefault: true };
  assert.equal(await fns.x$s(env.scope, "local", "model"), null, "未声明的模型 tier 不会凭空补齐");
  state.config.service_tier = "priority";
  state.override = { type: "standard" };
  assert.equal(await fns.x$s(env.scope, "local", "model"), null, "保留会话 Standard 覆盖");
  state.override = { type: "custom", serviceTier: "priority" };
  assert.equal(await fns.x$s(env.scope, "local", "model"), "priority");
  state.override = { type: "fromConfig" };

  // 补丁只追加 API key 放行；官方认可的登录方式随 Codex 版本原样保留。
  const officialAuths = ["chatgpt", "personalAccessToken", ...(bedrockNative ? ["amazonBedrock"] : [])];
  for (const auth of ["copilot", "amazonBedrock", null].filter(auth => !officialAuths.includes(auth))) {
    state.auth = auth;
    assert.equal(fns.Mxo().serviceTierAccess, null);
    assert.equal(await fns.x$s(env.scope, "local", "model", "priority"), null);
  }
  for (const auth of officialAuths) {
    state.auth = auth;
    api.runtime.update(null);
    assert.equal(fns.Mxo().serviceTierAccess.fast, true, "保留已有官方认证路径");
    assert.equal(await fns.x$s(env.scope, "remote", "model", "priority"), "priority");
  }
  api.runtime.update(enabled);
  state.auth = "apikey";
  state.host = "remote";
  assert.equal(fns.Mxo().serviceTierAccess, null);
  assert.equal(await fns.x$s(env.scope, "remote", "model", "priority"), null);
  assert.equal(await env.queue().readServiceTier("model"), null);
  assert.equal((await env.explicit("priority", "local")).serviceTier, null, "远端 manager 不借本机执行参数放行");
  state.host = "local";

  state.accountReads = 0;
  assert.equal(await env.queue().readServiceTier("model"), "priority");
  assert.equal(state.accountReads, 1, "默认队列只读取一次账号类型");
  state.copilotPreferred = true;
  state.copilotAvailable = true;
  state.accountReads = 0;
  assert.equal(await env.queue().readServiceTier("model"), null);
  assert.equal(state.accountReads, 0, "保留 Copilot 优先分支的短路");
  assert.equal((await env.explicit("priority")).serviceTier, null, "显式队列不能借底层 API 身份放行所选 Copilot");
  state.copilotAvailable = false;
  assert.equal((await env.explicit("priority")).serviceTier, "priority", "Copilot 不可用时仍使用 API 身份");
  state.copilotPreferred = false;
  state.copilotAvailable = true;
  assert.equal((await env.explicit("priority")).serviceTier, "priority", "未选择 Copilot 时不受其可用性影响");
  state.copilotAvailable = false;
  state.accountReads = 0;
  assert.equal((await env.explicit("priority")).serviceTier, "priority");
  assert.equal(state.accountReads, 1, "显式队列只读取一次账号类型");
  assert.equal((await env.explicit("priority", "remote")).serviceTier, null, "本机 manager 不能放行远端执行队列");
  assert.equal((await env.explicit(null)).serviceTier, null);
  assert.equal((await env.explicit(null, "local", false)).serviceTier, "saved-default", "无选项时沿用原有默认读取器");
  state.accountType = "amazonBedrock";
  api.runtime.update(null);
  assert.equal(await env.queue().readServiceTier("model"), bedrockNative ? "priority" : null);
  assert.equal((await env.explicit("priority")).serviceTier, bedrockNative ? "priority" : null);
  api.runtime.update(enabled);
  state.accountType = "chatgpt";
  api.runtime.update(null);
  assert.equal(await env.queue().readServiceTier("model"), "priority");
  assert.equal((await env.explicit("priority", "remote")).serviceTier, "priority", "已有官方账号的远端行为保持不变");
  state.accountType = "apiKey";
  assert.equal(await env.queue().readServiceTier("model"), null);
  assert.equal((await env.explicit("priority")).serviceTier, null);

  await runLocalCustomApiCases(environment(true, source), name);
  await require("./native-fast-module.test.cjs")({ source, url });
}

async function runLocalCustomApiCases(env, name) {
  const { api, state, fns } = env;
  const configured = { ...enabled, nativeFastLocalApiConfigured: true };
  state.auth = null;
  state.accountMissing = true;
  assert.equal(fns.Mxo().serviceTierAccess, null, `${name}: 不把未知空身份自动当作 API`);
  assert.equal(await env.queue().readServiceTier("model"), null);
  api.runtime.update(configured);
  assert.equal(api.runtime.status().localApiConfigured, true);
  for (const auth of [null, undefined]) {
    state.auth = auth;
    assert.equal(api.runtime.allow("local", auth), true);
    assert.equal(fns.Mxo().serviceTierAccess.fast, true, `${name}: 本机已确认的自定义 API 恢复入口`);
    assert.equal(await fns.x$s(env.scope, "local", "model", "priority"), "priority");
    state.accountReads = 0;
    assert.equal(await env.queue().readServiceTier("model"), "priority", "默认队列支持原生 account=null");
    assert.equal(state.accountReads, 1, "空账号默认队列只读取一次身份");
    state.accountReads = 0;
    assert.equal((await env.explicit("priority")).serviceTier, "priority", "显式队列与 UI 使用同一策略");
    assert.equal(state.accountReads, 1);
    assert.equal(await fns.x$s(env.scope, "local", "model", "default"), null, "Standard 不被强制成 priority");
  }
  state.authLoading = true;
  assert.equal(fns.Mxo().serviceTierAccess, null, "保留原生身份加载状态");
  state.authLoading = false;
  state.requirementsPending = true;
  assert.equal(fns.Mxo().serviceTierAccess, null, "保留原生 requirements 加载状态");
  state.requirementsPending = false;
  state.requirements = { requirements: { featureRequirements: { fast_mode: false } } };
  assert.equal(fns.Mxo().serviceTierAccess.fast, false);
  assert.equal(await fns.x$s(env.scope, "local", "model", "priority"), null);
  assert.equal(await env.queue().readServiceTier("model"), null, "自定义 API 也遵循管理员限制");
  state.requirements = { requirements: null };
  state.config.service_tier = null;
  state.model = { model: "model", isDefault: true };
  assert.equal(await fns.x$s(env.scope, "local", "model"), null, "不制造模型服务等级");
  assert.equal(await env.queue().readServiceTier("model"), null);
  state.config.service_tier = "priority";
  state.copilotPreferred = true;
  state.copilotAvailable = true;
  assert.equal(await env.queue().readServiceTier("model"), null, "不覆盖所选 Copilot");
  assert.equal((await env.explicit("priority")).serviceTier, null);
  state.copilotPreferred = false;
  state.copilotAvailable = false;
  for (const auth of ["", "unknown", "copilot", "ApiKey", "amazonBedrock"]) {
    assert.equal(api.runtime.allow("local", auth), false, "配置投影不改变其它认证方式");
  }
  state.host = "remote";
  assert.equal(fns.Mxo().serviceTierAccess, null);
  assert.equal(await fns.x$s(env.scope, "remote", "model", "priority"), null);
  assert.equal(await env.queue().readServiceTier("model"), null);
  assert.equal((await env.explicit("priority", "local")).serviceTier, null, "远端 manager 不能借用本机投影");
  state.host = "local";
  assert.equal((await env.explicit("priority", "remote")).serviceTier, null);
  for (const proof of [false, "true", 1, {}, undefined]) {
    api.runtime.update({ ...enabled, nativeFastLocalApiConfigured: proof });
    assert.equal(fns.Mxo().serviceTierAccess, null, "缺失、撤销或无效投影必须停止放行");
    assert.equal(await fns.x$s(env.scope, "local", "model", "priority"), null);
    assert.equal(await env.queue().readServiceTier("model"), null);
    assert.equal((await env.explicit("priority")).serviceTier, null);
  }
  for (const settings of [{ ...configured, enhancementsEnabled: false }, { ...configured, codexAppNativeFastUnlock: false }]) {
    api.runtime.update(settings);
    assert.equal(fns.Mxo().serviceTierAccess, null, "确认 API 连接也不能绕开用户开关");
  }
}

async function runNativeFastTests() {
  for (const shape of shapes) await runShape(shape);
  await require("./native-fast-settings.test.cjs")();

  // 补丁文本引用外层绑定；被内层账号解构遮蔽时必须整体放弃。
  const api = environment().api;
  const shadowed = sourceOf([uiBedrock, requestBedrock, queueDefaultBedrock,
    queueExplicitBedrock.replace("let{account:t}=", "let{account:n}=").replace("return t?.type===`chatgpt`||t?.type===`amazonBedrock`?", "return n?.type===`chatgpt`||n?.type===`amazonBedrock`?")]);
  assert.notEqual(shadowed, sourceOf(shapes[1].fixtures));
  assert.equal(api.plan(shadowed, url).diagnostics.reason, "shadowed-bindings");
  const shadowedDefault = sourceOf([uiBedrock, requestBedrock,
    queueDefaultBedrock.replace("let r=await e.getAccount", "let e=await e.getAccount").replace("return r.account?.type===`chatgpt`||r.account?.type===`amazonBedrock`", "return e.account?.type===`chatgpt`||e.account?.type===`amazonBedrock`"),
    queueExplicitBedrock]);
  assert.notEqual(shadowedDefault, sourceOf(shapes[1].fixtures));
  assert.equal(api.plan(shadowedDefault, url).diagnostics.reason, "shadowed-bindings");
  return "Native Fast structure matching (legacy and Bedrock shapes), four auth paths, requirements, local scope, opt-in, queue and module transport behavior passed";
}

module.exports = runNativeFastTests;
if (require.main === module) {
  runNativeFastTests().then(message => console.log(message)).catch(error => {
    console.error(error);
    process.exitCode = 1;
  });
}
