const assert = require("node:assert/strict");
const vm = require("node:vm");
const { permitsExternalApi, locate, condition } = require("./api-quota-gate.js");

// 独立模式的「官方额度解锁」开关是唯一判据：打开就放行，不再按供应商模式挑条件。
const profile = { id: "api", relayMode: "official", officialMixApiKey: true, upstreamBaseUrl: "https://proxy.example/v1" };
const settings = { codexAppStandaloneQuotaUnlock: true, relayProfilesEnabled: true, activeRelayId: "api", relayProfiles: [profile], activeRelaySessionProvider: "openai" };
assert.equal(permitsExternalApi(settings, "local"), true, "开关打开即放行");
for (const host of ["remote", "durable", "", null]) assert.equal(permitsExternalApi(settings, host), false, "只对本机生效");
// 供应商怎么配都与判断无关：开关才是唯一判据。
for (const patch of [{ relayProfilesEnabled: false }, { activeRelayId: "missing" }, { relayProfiles: null }, { relayProfiles: {} }]) {
  assert.equal(permitsExternalApi({ ...settings, ...patch }, "local"), true, `开关打开时 ${JSON.stringify(patch)} 不影响放行`);
}
for (const upstreamBaseUrl of [
  "https://api.openai.com/v1",
  "https://chatgpt.com/backend-api",
  "https://sub.openai.com/v1",
  "https://proxy.example/v1",
  "file:///tmp",
  "",
]) assert.equal(permitsExternalApi({ ...settings, relayProfiles: [{ ...profile, upstreamBaseUrl }] }, "local"), true, "不看上游地址");
for (const patch of [{ officialMixApiKey: false }, { relayMode: "pureApi" }]) {
  assert.equal(permitsExternalApi({ ...settings, relayProfiles: [{ ...profile, ...patch }] }, "local"), true, "不看供应商模式");
}
// 关掉开关就恢复官方判断，缺字段也按关闭处理。
for (const off of [{ ...settings, codexAppStandaloneQuotaUnlock: false }, (() => { const copy = { ...settings }; delete copy.codexAppStandaloneQuotaUnlock; return copy; })()]) {
  assert.equal(permitsExternalApi(off, "local"), false, "开关关闭必须恢复官方额度判断");
}
// 网关与聚合也不再需要单独放行条件（#2704 并入统一判据）。
for (const key of ["codex-plus-gateway", "agg"]) {
  assert.equal(permitsExternalApi({ ...settings, activeRelayId: key, relayProfiles: [{ id: key, relayMode: "pureApi" }] }, "local"), true, `${key} 在开关打开时放行`);
  assert.equal(permitsExternalApi({ ...settings, codexAppStandaloneQuotaUnlock: false, activeRelayId: key, relayProfiles: [{ id: key, relayMode: "pureApi" }] }, "local"), false, `${key} 在开关关闭时锁住`);
}
console.log("standalone quota unlock switch is the only gate");

const fixture = [
  "var Q;function init(){Q=derived(root,({get:g})=>{let a=g(auth),r=g(rate).data;if(a.authMethod!==`chatgpt`||r.rate_limit?.allowed!==!1)return!1;return true})}",
  "function Composer(){let quota=read(Q)&&host===`local`;let disabled=busy||quota;render({submitDisabled:disabled})}",
].join("\n");
const found = locate(fixture, "app://-/assets/app-primary-fixture.js");
assert.equal(found.quotaVariable, "quota");
assert.equal(found.hostVariable, "host");
assert.equal(fixture.split("\n")[found.lineNumber].slice(found.columnNumber,found.columnNumber+4), "busy");
assert.equal(locate("unrelated source", "app://-/a.js"), null);
assert.equal(locate(fixture + fixture, "app://-/a.js"), null, "ambiguous matches must fail closed");
assert.equal(locate(fixture.replace("submitDisabled:disabled", "other:disabled"), "app://-/a.js"), null);
assert.equal(locate(fixture + "\nlet other=read(Q)&&host===`local`;", "app://-/a.js"), null);
assert.equal(locate(fixture.replace("busy||quota", "busy||another"), "app://-/a.js"), null);
const unicodeFixture = fixture.replace("let disabled=", 'let label="\u{1f600}";let disabled=');
const unicodeFound = locate(unicodeFixture, "app://-/a.js");
assert.equal(unicodeFixture.split("\n")[unicodeFound.lineNumber].slice(unicodeFound.columnNumber, unicodeFound.columnNumber + 4), "busy");
assert.equal(condition({quotaVariable:"x;alert(1)",hostVariable:"host"}), null);

for (const allowed of [false, true]) {
  const context = {quota:true, host:"local", busy:false, window:{__codexPlusExternalApiQuotaAllowed:()=>allowed}};
  assert.equal(vm.runInNewContext(condition(found), context), false, "debugger condition never pauses");
  assert.equal(context.quota, !allowed);
  assert.equal(vm.runInNewContext("busy||quota", context), !allowed);
  context.busy = true;
  assert.equal(vm.runInNewContext("busy||quota", context), true, "other composer blocks must survive");
}
console.log("API quota gate policy, source matching, fail-closed behavior and other blocks passed");

const fs = require("node:fs");
let allowed = true;
const calls = [];
const dispatch = value => calls.push(value);
const hook = { memoizedState: new Set(), queue: {dispatch}, next: null };
const type = () => {};
type.toString = () => "function Adapter(){let quota=read(Q)&&host===`local`,disabled=busy||quota,next;render({submitDisabled:disabled})}";
const root = {__reactFiber$test: {type:"div", return:{type, memoizedState:hook, return:null}}};
const context = {
  Set, WeakMap, URL,
  window: {__codexPlusExternalApiQuotaAllowed:()=>allowed},
  document: {querySelectorAll:()=>[root]},
};
vm.runInNewContext(fs.readFileSync(require.resolve("./api-quota-gate.js"), "utf8"), context);
const refresh = context.window.__codexPlusApiQuotaGate.refreshComposers;
assert.equal(refresh(found), 1, "already mounted composer must render after installation");
assert.equal(calls[0].size, 0);
assert.equal(refresh(found), 0, "polls must not keep rerendering");
allowed = false;
assert.equal(refresh(found), 1, "switching back must restore the official gate");
assert.equal(refresh(found), 0);
assert.equal(refresh(found, true), 1, "rearming in official mode must restore original render");
allowed = true;
assert.equal(refresh(found), 1);
assert.equal(refresh(found, true), 1, "reconnected breakpoint must invalidate stale render");
hook.memoizedState = new Set(["active-stop"]);
assert.equal(refresh(found, true), 0, "never change active stop state");
hook.memoizedState = new Set();
hook.next = {memoizedState:new Set(),queue:{dispatch:()=>{}},next:null};
assert.equal(refresh(found, true), 0, "ambiguous hooks fail closed");
hook.next = null;
type.toString = () => "function unrelated(){}";
assert.equal(refresh(found, true), 0);
console.log("Composer startup refresh, reconnect, policy changes and deduplication passed");
