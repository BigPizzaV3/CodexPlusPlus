import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
const source = await readFile(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
const serviceSource = await readFile(new URL("../../../assets/inject/renderer-inject/30-service-tier.js", import.meta.url), "utf8");
function runtime(storage = new Map<string,string>(), sharedWindow: any = {}, enabled = true) {
  let profile = "aggregate", now = 10000, active = true;
  let managed = false, modelWindows = "", reads = 0, writes = 0, parses = 0;
  const listeners = new Map<string, (event: any) => void>();
  const timers: Array<() => void> = [];
  sharedWindow.addEventListener ??= (type: string, callback: (event: any) => void) => listeners.set(type, callback);
  sharedWindow.setTimeout ??= (callback: () => void) => { timers.push(callback); return timers.length; };
  let configuredDefault = "gpt-5.5";
  const catalog = { status: "ok", codex_plus_default_group_id: "a" };
  let names = ["gpt-5.5", "sol", "cpx-b/sol", "cpx-a/sol"];
  let mode = "aggregate";
  let supported = ["low", "medium", "high", "xhigh", "max", "ultra"];
  const effortByModel: Record<string,string[]> = {};
  const groups: Record<string,string> = {};
  const nodes: any[] = [];
  const context = vm.createContext({
    window: sharedWindow, document: { querySelectorAll: () => nodes },
    localStorage: { getItem: (k: string) => { reads++; return storage.get(k); }, setItem: (k:string,v:string) => { writes++; storage.set(k,v); } },
    Date: { now: () => now }, Map, WeakMap, Promise, Set,
    JSON: { parse: (value: string) => { parses++; return JSON.parse(value); }, stringify: JSON.stringify },
    codexPlusBackendSettingsLoaded: true,
    codexPlusBackendSettings: { codexAppThreadModelRetention: enabled, relayProfiles: [{id:"a",relayMode:"pureApi"}] },
    codexRemoteSessionActiveProfile: () => active ? ({ id: profile, relayMode: mode, modelWindows }) : null,
    codexManagedSessionConnection: () => managed ? {} : null,
    codexManagedSessionRecoveryOptions: (options: any) => options,
    assertCodexManagedSessionConnection: () => {},
    applyCodexRemoteSessionProviderOverride: (_method: string, params: any) => params,
    applyCodexManagedSessionConnection: (_method: string, params: any) => params,
    sendCodexPlusDiagnostic: () => {},
    codexModelCatalog: catalog,
    codexPlusConfiguredDefaultModel: () => configuredDefault,
    codexPlusModelNames: () => names,
    modelReasoningEfforts: (model:string) => (effortByModel[model] || supported).map(reasoningEffort=>({reasoningEffort})),
    codexPlusModelMetadata: (model:string) => ({ defaultReasoningEffort:"medium",
      codex_plus_group_id: groups[model] || (mode==="pureApi" ? undefined : model.startsWith("cpx-") ? model.slice(4,model.indexOf("/")) : catalog.codex_plus_default_group_id) }),
  });
  const start = source.indexOf("  function codexThreadModelRetentionSettingsChanged()");
  const end = source.indexOf("  function patchAppServerModelRequestClient(", start);
  const hostStart = source.indexOf("  function modelResponseHostId(");
  const hostEnd = source.indexOf("  function codexPureApiAllowedModelSet(", hostStart);
  const recoveryStart = serviceSource.indexOf("  async function reloadCodexManagedSessionConnection(");
  const recoveryEnd = serviceSource.indexOf("  async function sendCodexManagedSessionTurn(", recoveryStart);
  const api = vm.runInContext(`(()=>{${source.slice(hostStart,hostEnd)}${source.slice(start,end)}${serviceSource.slice(recoveryStart,recoveryEnd)};return {codexThreadModelRetentionSettingsChanged,rememberCodexThreadModel,codexRetainedThreadModel,sendCodexRetainedModelRequest,syncCodexRetainedThreadModels,observeCodexThreadModelSelection,noteCodexThreadModelPickerIntent,refreshCodexThreadModelBeforeTurn,reloadCodexManagedSessionConnection}})()`,context);
  sharedWindow.__codexPlusThreadModelRetentionRefresh = api.codexThreadModelRetentionSettingsChanged;
  api.codexThreadModelRetentionSettingsChanged();
  return { api, storage, catalog, nodes, effortByModel, groups, window: sharedWindow, profile: (v:string) => { profile=v; },
    enabled: (value: boolean) => { context.codexPlusBackendSettings.codexAppThreadModelRetention=value; api.codexThreadModelRetentionSettingsChanged(); },
    master: (value: boolean) => { (context.codexPlusBackendSettings as any).enhancementsEnabled=value; api.codexThreadModelRetentionSettingsChanged(); },
    replaceSettings: () => { context.codexPlusBackendSettings = { ...context.codexPlusBackendSettings }; },
    managed: (value: boolean) => { managed=value; }, contextWindows: (value: string) => { modelWindows=value; },
    fresh: (id="one") => { api.codexThreadModelRetentionSettingsChanged().threads.set(id,"fresh"); },
    direct: () => { profile="a"; mode="pureApi"; },
    counts: () => ({reads,writes}), parseCount: () => parses, storageEvent: (key: string | null) => listeners.get("storage")?.({key}),
    runTimers: () => { const scheduled=timers.splice(0); scheduled.forEach(callback=>callback()); }, timers,
    setDefault: (v:string) => { configuredDefault=v; },
    lockProfile: () => {
      active = false;
      context.codexPlusBackendSettings.activeRelayId = profile;
      context.codexPlusBackendSettings.relayProfiles = [{ id: profile, relayMode: mode }];
      (catalog as any).sources = [{ id: `${mode === "aggregate" ? "aggregate" : "relay-profile"}:${profile}` }];
    },
    configure: (id:string,relayMode:string,models:string[],efforts=supported) => {profile=id;mode=relayMode;names=models;supported=efforts;},
    tick: () => { now+=2000; } };
}
function pickerFixture(r: ReturnType<typeof runtime>, model = "gpt-5.5") {
  const picker: any = { model, reasoningEffort: "low", models: [{model:"sol"},{model:"gpt-5.5"}], onSelectModel: () => {} };
  const owner = {conversationId:"one",permissionsHostId:"local"};
  const fiber = {memoizedProps:picker,return:{memoizedProps:owner}};
  const node = {__reactFiber$test:fiber};
  r.nodes.push(node);
  const row = (selected: string) => ({
    __reactFiber$test: {key:selected,memoizedProps:{},return:fiber},
    getAttribute: () => null,
    closest: (selector: string) => selector === "[data-model-picker-view]" ? {} : null,
  });
  const choose = (selected: string) => {
    const target = row(selected);
    r.api.observeCodexThreadModelSelection({type:"click",target:{closest:(selector:string)=>selector === '[role="menuitemradio"]' ? target : null}});
  };
  return {picker,owner,choose};
}
it("source provider survives direct to aggregate to another aggregate and back",()=>{
  const r=runtime();
  r.configure("a","pureApi",["sol"]);
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  r.configure("aggregate-1","aggregate",["sol","cpx-a/sol"]);
  r.catalog.codex_plus_default_group_id="b";
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"cpx-a/sol");
  r.configure("aggregate-2","aggregate",["sol","cpx-a/sol"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"cpx-a/sol");
  r.configure("a","pureApi",["sol"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
});
it("when source is absent a same-name candidate supporting the saved effort precedes model defaults",()=>{
  const r=runtime();
  r.configure("a","pureApi",["sol"]);
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"ultra"},{},"local");
  r.configure("aggregate-new","aggregate",["sol","cpx-c/sol"]);
  r.catalog.codex_plus_default_group_id="b";
  r.effortByModel.sol=["low","medium"];
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"cpx-c/sol");
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"ultra");
});
it("missing source uses same-name model and supported effort before native defaults",()=>{
  const r=runtime();
  r.configure("a","pureApi",["sol"]);
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"ultra"},{},"local");
  r.configure("b","pureApi",["sol"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"ultra");
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"ultra"},{},"local");
  r.configure("c","pureApi",["sol"],["low","medium","high"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"medium");
  r.configure("d","pureApi",["different"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one"),null);
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"different",effort:"low"},{},"local");
  r.configure("a","pureApi",["sol"],["low","medium","high","ultra"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one").groupId,"a");
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"ultra");
});
it("a profile change waits for its own catalog instead of restoring with stale member identities",()=>{
  const r=runtime();
  r.configure("a","pureApi",["sol"]);
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  r.configure("aggregate-new","aggregate",["sol","cpx-a/sol"]);
  (r.catalog as any).sources=[{id:"aggregate:aggregate-old"}];
  assert.equal(r.api.codexRetainedThreadModel("local","one"),null);
  (r.catalog as any).sources=[{id:"aggregate:aggregate-new"}];
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
});
it("latest explicit cross-profile choice wins and old pureApi records migrate by source id",()=>{
  const r=runtime();
  r.configure("a","pureApi",["sol"]);
  r.storage.set("codexPlus.threadModels.v1",JSON.stringify([{key:JSON.stringify(["local","a","one"]),model:"sol",effort:"high",groupId:"",at:9000}]));
  r.configure("aggregate","aggregate",["sol","cpx-a/sol"]);r.catalog.codex_plus_default_group_id="b";
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"cpx-a/sol");
  r.tick();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"low"},{},"local",undefined,true);
  r.configure("a","pureApi",["sol"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"low");
});
it("successful per-thread settings survive a fresh renderer and retain provider identity", async () => {
  const r=runtime();
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>({}),"thread/settings/update",{threadId:"one",model:"sol",effort:"high"});
  const next=runtime(r.storage);
  next.catalog.codex_plus_default_group_id="b";
  assert.equal(next.api.codexRetainedThreadModel("local","one").model,"cpx-a/sol");
  assert.equal(next.api.codexRetainedThreadModel("local","one").effort,"high");
  assert.equal(next.api.codexRetainedThreadModel("local","two"),null);
  assert.equal(next.api.codexRetainedThreadModel("ssh","one"),null);
  next.profile("other");
  assert.equal(next.api.codexRetainedThreadModel("local","one").model,"cpx-a/sol");
});
it("failed settings, read/resume fallback, and late responses after profile change do not overwrite", async()=>{
  const r=runtime(); const send=r.api.sendCodexRetainedModelRequest;
  await send({hostId:"local"},async()=>({}),"thread/settings/update",{threadId:"one",model:"sol",effort:"low"});
  await assert.rejects(send({hostId:"local"},async()=>{throw Error("cancelled")},"thread/settings/update",{threadId:"one",model:"gpt-5.5"}));
  await send({hostId:"local"},async()=>({applied:false}),"thread/settings/update",{threadId:"one",model:"gpt-5.5"});
  await send({hostId:"local"},async()=>({thread:{model:"gpt-5.5"}}),"thread/resume",{threadId:"one"});
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
  let done:any;
  const p=send({hostId:"local"},()=>new Promise(resolve=>{done=resolve}),"thread/settings/update",{threadId:"one",model:"gpt-5.5"});
  r.profile("other");done({});await p;
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
});
it("gateway pinned routes preserve provider identity across direct, gateway and aggregate catalogs",()=>{
  const r=runtime();
  r.configure("a","pureApi",["sol"]);
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"ultra"},{},"local");
  r.configure("codex-plus-gateway","pureApi",["sol","a/sol","b/sol","group/coding"]);
  Object.assign(r.groups,{"sol":"codex-plus-gateway-auto","a/sol":"a","b/sol":"b","group/coding":"codex-plus-gateway-groups"});
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"a/sol");
  r.tick();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"b/sol",effort:"high"},{},"local",undefined,true);
  r.configure("aggregate","aggregate",["sol","cpx-b/sol"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"cpx-b/sol");
  r.configure("b","pureApi",["sol"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
});
it("gateway automatic routes and named groups remain distinct from pinned supplier routes",()=>{
  const r=runtime();
  r.configure("codex-plus-gateway","pureApi",["sol","a/sol","group/coding","group/review"]);
  Object.assign(r.groups,{"sol":"codex-plus-gateway-auto","a/sol":"a","group/coding":"codex-plus-gateway-groups","group/review":"codex-plus-gateway-groups"});
  for(const model of ["sol","a/sol","group/coding","group/review"]) {
    r.tick();
    r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model,effort:"high"},{},"local",undefined,true);
    assert.equal(r.api.codexRetainedThreadModel("local","one").model,model);
  }
  r.configure("codex-plus-gateway","pureApi",["sol","a/sol","group/coding"]);
  assert.equal(r.api.codexRetainedThreadModel("local","one"),null,"a removed group must not resolve to another group");
});
it("automatic settings acknowledgements cannot replace an explicit conversation effort",async()=>{
  const r=runtime(),f=pickerFixture(r,"sol");
  f.picker.reasoningEffort="high";f.choose("high");
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>({}),
    "thread/settings/update",{threadId:"one",model:"sol",effort:"low"});
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
});
it("rapid A to B to A navigation saves a confirmed picker choice before the settle timer",()=>{
  const r=runtime(),f=pickerFixture(r,"sol");
  f.choose("high");f.picker.reasoningEffort="high";
  r.api.syncCodexRetainedThreadModels();
  f.owner.conversationId="two";f.picker.reasoningEffort="low";
  r.api.syncCodexRetainedThreadModels();
  r.tick();
  f.owner.conversationId="one";
  assert.equal(r.api.codexRetainedThreadModel("local","one")?.effort,"high");
});
it("a new explicit effort and cancelled intent do not alter resume parameters",async()=>{
  const r=runtime(),f=pickerFixture(r,"sol");
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  f.choose("low");
  const params={threadId:"one",model:"sol",reasoningEffort:"low"};
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async(_method:any,value:any)=>assert.equal(value,params),"thread/resume",params);
  r.tick();
  r.window.__codexPlusThreadModelSelections?.clear();
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async(_method:any,value:any)=>assert.equal(value,params),"thread/resume",params);
});
it("a fixed multi-instance profile retains conversations only when the catalog confirms its identity",()=>{
  const r=runtime(),f=pickerFixture(r,"sol");
  r.lockProfile();
  f.choose("high");f.picker.reasoningEffort="high";
  r.api.syncCodexRetainedThreadModels();
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
  (r.catalog as any).sources=[{id:"aggregate:another"}];
  assert.equal(r.api.codexRetainedThreadModel("local","one"),null);
});
it("choosing the configured default is an explicit choice and replaces the old selection",()=>{
  const r=runtime(),f=pickerFixture(r,"sol");
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local",undefined,true);
  r.api.noteCodexThreadModelPickerIntent(r.nodes[0],{model:"gpt-5.5",effort:"low",groupId:"a"});
  f.picker.model="gpt-5.5";
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"gpt-5.5");
});
it("confirming the same effort upgrades old records so later automatic writes cannot replace them",async()=>{
  const r=runtime(),f=pickerFixture(r,"sol");
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  f.picker.reasoningEffort="high";f.choose("high");r.api.syncCodexRetainedThreadModels();
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>({}),
    "thread/settings/update",{threadId:"one",model:"sol",effort:"low"});
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
});
it("native power slider choices survive restart without automatically overriding an existing thread",async()=>{
  for (const type of ["pointerup","keydown"]) {
    const r=runtime(),f=pickerFixture(r,"sol");
    const slider={__reactFiber$test:{memoizedProps:{},return:r.nodes[0].__reactFiber$test},
      getAttribute:()=>null,closest:()=>null};
    r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"low"},{},"local");
    if(type==="pointerup")f.picker.reasoningEffort="high";
    r.api.observeCodexThreadModelSelection({type,key:"ArrowRight",
      target:{closest:(selector:string)=>selector==="[data-reasoning-slider]"?slider:null}});
    f.picker.reasoningEffort="high";
    r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
    const next=runtime(r.storage),fresh=pickerFixture(next,"sol");
    assert.equal(next.api.codexRetainedThreadModel("local","one").effort,"high");
    const calls:any[]=[];
    fresh.picker.onSelectModel=(model:string,effort:string)=>{calls.push([model,effort]);fresh.picker.reasoningEffort=effort};
    next.api.syncCodexRetainedThreadModels();next.tick();next.api.syncCodexRetainedThreadModels();
    assert.deepEqual(calls,[]);
    await Promise.resolve();
    fresh.owner.conversationId="two";next.api.syncCodexRetainedThreadModels();
    fresh.owner.conversationId="one";fresh.picker.reasoningEffort="low";
    next.api.syncCodexRetainedThreadModels();next.tick();next.api.syncCodexRetainedThreadModels();
    assert.equal(calls.length,0);
  }
});
it("power-slider settings acknowledgements save the submitted effort, not the previous effort",async()=>{
  const r=runtime(),f=pickerFixture(r,"sol");
  const slider={__reactFiber$test:r.nodes[0].__reactFiber$test,getAttribute:()=>null,closest:()=>null};
  r.api.observeCodexThreadModelSelection({type:"keydown",key:"ArrowRight",
    target:{closest:(selector:string)=>selector==="[data-reasoning-slider]"?slider:null}});
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>({}),
    "thread/settings/update",{threadId:"one",model:"sol",effort:"high"});
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
});
it("slider previews, cancellation, disabled controls and unrelated arrow keys do not save choices",()=>{
  for(const type of ["pointermove","pointercancel","keydown","pointerup"]) {
    const r=runtime(),f=pickerFixture(r,"sol");
    const slider={__reactFiber$test:r.nodes[0].__reactFiber$test,getAttribute:()=>null,closest:()=>null};
    f.picker.reasoningEffortDisabled=type==="pointerup";
    r.api.observeCodexThreadModelSelection({type,key:"Escape",
      target:{closest:(selector:string)=>selector==="[data-reasoning-slider]"?slider:null}});
    r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
    assert.equal(r.api.codexRetainedThreadModel("local","one"),null);
  }
});
it("an arrow at the slider boundary does not confirm a native fallback as a new choice",()=>{
  const r=runtime(),f=pickerFixture(r,"sol");
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  const slider={__reactFiber$test:r.nodes[0].__reactFiber$test,getAttribute:()=>null,closest:()=>null};
  r.api.observeCodexThreadModelSelection({type:"keydown",key:"ArrowLeft",
    target:{closest:(selector:string)=>selector==="[data-reasoning-slider]"?slider:null}});
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
});
it("stable native selection restores once and does not override subsequent changes",async()=>{
  const r=runtime(); const calls:any[]=[];
  r.direct();r.fresh();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  const picker={model:"gpt-5.5",reasoningEffort:"low",models:[{model:"sol"}],onSelectModel:(...v:any[])=>{calls.push(v);picker.model=v[0];picker.reasoningEffort=v[1];}};
  const node={__reactFiber$test:{memoizedProps:picker,return:{memoizedProps:{conversationId:"one",threadId:"wrong",permissionsHostId:"local"}}}};
  r.nodes.push(node);
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.deepEqual(calls,[["sol","high"]]);r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,1);
  await Promise.resolve();
  picker.model="sol";picker.reasoningEffort="high";r.api.syncCodexRetainedThreadModels();
  picker.model="gpt-5.5";r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,1);
  let done:any; const p=r.api.sendCodexRetainedModelRequest({hostId:"local"},()=>new Promise(resolve=>{done=resolve}),"thread/settings/update",{threadId:"one",model:"gpt-5.5",effort:"low"});
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,1);done({});await p;
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,1);
});
it("an unavailable historical model leaves the native selection and saved history unchanged",()=>{
  const r=runtime(),f=pickerFixture(r,"sol"),calls:any[]=[];
  r.direct();r.fresh();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  r.configure("new","pureApi",["gpt-5.5"]);
  f.picker.models=[{model:"gpt-5.5"}];
  f.picker.onSelectModel=(model:string,effort:string)=>{calls.push([model,effort]);f.picker.model=model;f.picker.reasoningEffort=effort};
  const before=r.storage.get("codexPlus.threadModels.v1");
  r.api.syncCodexRetainedThreadModels();r.api.syncCodexRetainedThreadModels();
  assert.deepEqual(calls,[]);assert.equal(f.picker.model,"sol");
  assert.equal(r.storage.get("codexPlus.threadModels.v1"),before);
});
it("a retained model missing from the refreshed picker does not force a default",()=>{
  const r=runtime(),f=pickerFixture(r,"sol"),calls:any[]=[];
  r.direct();r.fresh();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  f.picker.models=[{model:"gpt-5.5"}];
  f.picker.onSelectModel=(model:string,effort:string)=>{calls.push([model,effort]);f.picker.model=model;f.picker.reasoningEffort=effort};
  r.api.syncCodexRetainedThreadModels();r.api.syncCodexRetainedThreadModels();
  assert.deepEqual(calls,[]);assert.equal(f.picker.model,"sol");
});
it("three failed native restores stop without forcing a default or replacing history",async()=>{
  const r=runtime(),f=pickerFixture(r,"gpt-5.5"),calls:any[]=[];
  r.direct();r.fresh();
  r.setDefault("cpx-b/sol");
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  f.picker.models.push({model:"cpx-b/sol"});
  f.picker.onSelectModel=(model:string,effort:string)=>{
    calls.push([model,effort]);
    if(model==="gpt-5.5"){f.picker.model=model;f.picker.reasoningEffort=effort}
  };
  const before=r.storage.get("codexPlus.threadModels.v1");
  r.api.syncCodexRetainedThreadModels();
  for(let i=0;i<5;i++){
    r.tick();r.api.syncCodexRetainedThreadModels();
    await Promise.resolve();await Promise.resolve();
  }
  assert.deepEqual(calls.slice(0,3),[["sol","high"],["sol","high"],["sol","high"]]);
  assert.equal(calls.length,3);assert.equal(f.picker.model,"gpt-5.5");
  assert.equal(r.storage.get("codexPlus.threadModels.v1"),before);
});
it("explicit native local selections update memory even when settings RPC is unsupported or skipped", async()=>{
  const r=runtime(), f=pickerFixture(r);
  r.api.rememberCodexThreadModel("turn/start",{threadId:"one",model:"gpt-5.5"},{},"local");
  f.choose("sol");
  await assert.rejects(r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>{throw Error("method not found")},"thread/settings/update",{threadId:"one",model:"sol"}));
  f.picker.model="sol";f.picker.reasoningEffort="high";
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
  f.choose("gpt-5.5");f.picker.model="gpt-5.5";
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>({}),"turn/start",{threadId:"one",model:"gpt-5.5"});
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"gpt-5.5");
});
it("cancelled, rejected, remote and unrelated menu activations do not become model choices",async()=>{
  const r=runtime(),f=pickerFixture(r);
  r.api.rememberCodexThreadModel("turn/start",{threadId:"one",model:"gpt-5.5"},{},"local");
  f.choose("sol");r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"gpt-5.5");
  f.picker.model="sol";
  await assert.rejects(r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>{throw Error("denied")},"thread/settings/update",{threadId:"one",model:"sol"}));
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"gpt-5.5");
  f.owner.permissionsHostId="ssh";f.choose("sol");
  f.owner.permissionsHostId="local";f.choose("unknown");
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"gpt-5.5");
});
it("pending requests keep their original provider identity across default changes",async()=>{
  const r=runtime();let done:any;
  const pending=r.api.sendCodexRetainedModelRequest({hostId:"local"},()=>new Promise(resolve=>{done=resolve}),"thread/settings/update",{threadId:"one",model:"sol",effort:"high"});
  r.catalog.codex_plus_default_group_id="b";done({});await pending;
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"cpx-a/sol");
  assert.equal(r.api.codexRetainedThreadModel("local","one").groupId,"a");
});
it("local user selection also retains the provider captured before catalog changes",()=>{
  const r=runtime(),f=pickerFixture(r);f.choose("sol");
  r.catalog.codex_plus_default_group_id="b";f.picker.model="cpx-a/sol";
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.equal(r.api.codexRetainedThreadModel("local","one").groupId,"a");
});
it("failed restores retry while pending and stop once the native callback succeeds",async()=>{
  const r=runtime(),f=pickerFixture(r);let failed=0,ready=0;
  r.direct();r.fresh();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  f.picker.onSelectModel=async()=>{failed++;return false};
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();await Promise.resolve();
  f.picker.onSelectModel=(model:string,effort:string)=>{ready++;f.picker.model=model;f.picker.reasoningEffort=effort};
  r.tick();r.api.syncCodexRetainedThreadModels();await Promise.resolve();
  assert.equal(failed,1);assert.equal(ready,1);
  r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(ready,1);
  f.picker.model="gpt-5.5";f.picker.onSelectModel=()=>{failed++;throw Error("not ready")};
  r.api.syncCodexRetainedThreadModels();
  for(let i=0;i<8;i++){r.tick();r.api.syncCodexRetainedThreadModels();await Promise.resolve();}
  assert.equal(failed,1);
});
it("undefined native callback results are verified from picker state and do not spin",async()=>{
  const r=runtime(),f=pickerFixture(r);let calls=0;
  r.direct();r.fresh();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol"},{},"local");
  f.picker.onSelectModel=()=>{calls++};
  r.api.syncCodexRetainedThreadModels();
  for(let i=0;i<8;i++){r.tick();r.api.syncCodexRetainedThreadModels();await Promise.resolve();}
  assert.equal(calls,3);
});
it("successful duplicate settings acknowledgements cannot reset the restore retry budget",async()=>{
  const r=runtime(),f=pickerFixture(r);let calls=0;
  r.direct();r.fresh();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  f.picker.onSelectModel=async(model:string,effort:string)=>{
    calls++;
    await r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>({}),"thread/settings/update",{threadId:"one",model,effort});
  };
  r.api.syncCodexRetainedThreadModels();
  for(let i=0;i<12;i++){
    r.tick();r.api.syncCodexRetainedThreadModels();
    for(let n=0;n<4;n++)await Promise.resolve();
  }
  assert.equal(calls,3);
});
it("effort-only native selections survive unsupported local settings updates",async()=>{
  const r=runtime(),f=pickerFixture(r,"sol");
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  f.picker.reasoningEffort="high";f.choose("low");
  await assert.rejects(r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>{throw Error("method not found")},"thread/settings/update",{threadId:"one",model:"sol",effort:"low"}));
  f.picker.reasoningEffort="low";r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"low");
});
it("old scheduled callbacks cannot replace a newer reinjected observer",()=>{
  const sharedWindow:any={},old=runtime(new Map(),sharedWindow);
  old.api.syncCodexRetainedThreadModels();const oldObserver=sharedWindow.__codexPlusThreadModelSelectionObserver;
  const next=runtime(new Map(),sharedWindow);next.api.syncCodexRetainedThreadModels();
  const newObserver=sharedWindow.__codexPlusThreadModelSelectionObserver;assert.notEqual(oldObserver,newObserver);
  old.api.syncCodexRetainedThreadModels();assert.equal(sharedWindow.__codexPlusThreadModelSelectionObserver,newObserver);
  old.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol"},{},"local");
  assert.equal(old.api.codexRetainedThreadModel("local","one"),null);
});
it("corrupt storage and removed models fail closed",()=>{
  const r=runtime(new Map([["codexPlus.threadModels.v1","invalid"]]));assert.equal(r.api.codexRetainedThreadModel("local","one"),null);
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"removed"},{},"local");assert.equal(r.api.codexRetainedThreadModel("local","one"),null);
});
it("successful selection is recorded before the catalog finishes loading and restored after it arrives",async()=>{
  const r=runtime();
  (r.catalog as any).status = "loading";
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>({}),
    "turn/start",{threadId:"one",model:"sol",reasoningEffort:"high"});
  assert.equal(r.storage.has("codexPlus.threadModels.v1"),true);
  assert.equal(r.api.codexRetainedThreadModel("local","one"),null);
  (r.catalog as any).status = "ok";
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
});
it("an automatic fallback turn cannot replace the last explicit model and host envelopes stay isolated",async()=>{
  const r=runtime();const send=r.api.sendCodexRetainedModelRequest;
  const envelope={hostId:"local",method:"thread/settings/update",params:{threadId:"one",model:"sol",effort:"high"}};
  await send({hostId:"local"},async()=>({}),"send-cli-request-for-host",envelope);
  await send({hostId:"local"},async()=>({}),"turn/start",{threadId:"one",model:"gpt-5.5"});
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
  await send({hostId:"local"},async()=>({}),"send-cli-request-for-host",{...envelope,hostId:"ssh",params:{...envelope.params,model:"gpt-5.5"}});
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
  let actual:any;
  await send({hostId:"local"},async(_m:any,p:any)=>{actual=p;return{}},"send-cli-request-for-host",{hostId:"local",method:"thread/resume",params:{threadId:"one"}});
  assert.deepEqual(actual,{hostId:"local",method:"thread/resume",params:{threadId:"one"}});
});
it("locked and remote picker contexts are never restored",()=>{
  const r=runtime();let calls=0;
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  const picker:any={model:"gpt-5.5",models:[{model:"sol"}],lockedModelSlug:"gpt-5.5",onSelectModel:()=>{calls++}};
  const props={conversationId:"one",permissionsHostId:"local"};
  r.nodes.push({__reactFiber$x:{memoizedProps:picker,return:{memoizedProps:props}}});
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls,0);
  delete picker.lockedModelSlug;props.permissionsHostId="ssh";
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls,0);
});

it("disabled retention does not read, write, install a picker action, or alter requests",async()=>{
  const saved=JSON.stringify([{key:JSON.stringify(["local","a","one"]),model:"sol",effort:"high",at:1}]);
  const r=runtime(new Map([["codexPlus.threadModels.v1",saved]]),{},false), f=pickerFixture(r);
  r.direct();r.fresh();let calls=0;f.picker.onSelectModel=()=>calls++;
  const params={threadId:"one",model:"gpt-5.5",config:{model_reasoning_effort:"low"}};
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async(_method:any,value:any)=>assert.equal(value,params),"thread/resume",params);
  r.api.rememberCodexThreadModel("thread/settings/update",params,{},"local");
  assert.equal(r.api.codexRetainedThreadModel("local","one"),null);
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();r.runTimers();
  assert.deepEqual(r.counts(),{reads:0,writes:0});assert.equal(calls,0);
  assert.equal(r.storage.get("codexPlus.threadModels.v1"),saved);
});

it("only observed new unmanaged local threads can automatically synchronize",async()=>{
  const r=runtime(),f=pickerFixture(r),calls:any[]=[];r.direct();
  f.picker.onSelectModel=(...args:any[])=>{calls.push(args);};
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.equal(calls.length,0);
  await r.api.sendCodexRetainedModelRequest({hostId:"local"},async()=>({thread:{id:"one"}}),"thread/start",{});
  r.managed(true);r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,0);
  r.managed(false);r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.deepEqual(calls,[["sol","high"]]);
  await Promise.resolve();
  let done:any;
  const resume=r.api.sendCodexRetainedModelRequest({hostId:"local"},()=>new Promise(resolve=>{done=resolve}),"thread/resume",{threadId:"one"});
  r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,1);
  done({});await resume;r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,1);
});

it("a resumed conversation keeps a later manual model and effort across scans",async()=>{
  for(const mode of ["pureApi", "aggregate"]) for(const enabled of [false,true]) {
    const r=runtime(),f=pickerFixture(r,"sol"),calls:any[]=[];
    if(mode === "pureApi") r.direct();
    f.picker.reasoningEffort="high";
    r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
    r.enabled(enabled);
    f.picker.onSelectModel=(...args:any[])=>{calls.push(args);};
    let finish:any;
    const params={threadId:"one",model:"sol",reasoningEffort:"high"};
    const resume=r.api.sendCodexRetainedModelRequest({hostId:"local"},(_method:any,value:any)=>{
      assert.equal(value,params);
      return new Promise(resolve=>{finish=resolve;});
    },"thread/resume",params);
    f.choose("gpt-5.5");f.picker.model="gpt-5.5";f.picker.reasoningEffort="low";
    finish({});await resume;
    for(let i=0;i<5;i++){r.api.syncCodexRetainedThreadModels();r.tick();r.runTimers();await Promise.resolve();}
    assert.deepEqual(calls,[],`${mode} retention=${enabled}`);
    assert.equal(f.picker.model,"gpt-5.5");assert.equal(f.picker.reasoningEffort,"low");
    if(enabled) {
      assert.equal(r.api.codexRetainedThreadModel("local","one").model,"gpt-5.5");
      assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"low");
    }
  }
});

it("a fresh conversation stops automatic restoration after the initial choice has settled",async()=>{
  for(const initiallyMatched of [true,false]) {
    const r=runtime(),f=pickerFixture(r,initiallyMatched ? "sol" : "gpt-5.5"),calls:any[]=[];
    r.direct();r.fresh();
    r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
    if(initiallyMatched) f.picker.reasoningEffort="high";
    f.picker.onSelectModel=(model:string,effort:string)=>{
      calls.push([model,effort]);f.picker.model=model;f.picker.reasoningEffort=effort;
    };
    r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();await Promise.resolve();
    r.api.syncCodexRetainedThreadModels();
    assert.equal(calls.length,initiallyMatched ? 0 : 1);
    // Native selection can change through paths the injected event observer does not see.
    f.picker.model="gpt-5.5";f.picker.reasoningEffort="low";
    for(let i=0;i<5;i++){r.tick();r.api.syncCodexRetainedThreadModels();await Promise.resolve();}
    assert.equal(calls.length,initiallyMatched ? 0 : 1);
    assert.equal(f.picker.model,"gpt-5.5");assert.equal(f.picker.reasoningEffort,"low");
  }
});

it("late acknowledgements for an older manual choice cannot replace the newer one",async()=>{
  const r=runtime(),f=pickerFixture(r,"sol");r.direct();r.fresh();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local",undefined,true);
  let finishOld:any,finishNew:any;
  f.choose("gpt-5.5");
  const old=r.api.sendCodexRetainedModelRequest({hostId:"local"},()=>new Promise(resolve=>{finishOld=resolve;}),
    "thread/settings/update",{threadId:"one",model:"gpt-5.5"});
  r.tick();f.choose("high");
  const current=r.api.sendCodexRetainedModelRequest({hostId:"local"},()=>new Promise(resolve=>{finishNew=resolve;}),
    "thread/settings/update",{threadId:"one",model:"sol",effort:"high"});
  f.picker.model="sol";f.picker.reasoningEffort="high";
  finishNew({});await current;finishOld({});await old;
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
  assert.equal(r.api.codexRetainedThreadModel("local","one").effort,"high");
});

it("aggregate, managed history, and recovery-pending sessions never auto-select",()=>{
  const r=runtime(),f=pickerFixture(r);let calls=0;f.picker.onSelectModel=()=>calls++;
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  r.fresh();r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  r.direct();r.window.__codexPlusManagedSessionStarts=new Map([["one",{}]]);
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  r.window.__codexPlusManagedSessionStarts.clear();r.window.__codexPlusConnectionRecoveryPending=new Set(["one"]);
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls,0);
});

it("uncertain, managed, remote or stale start responses do not authorize automatic selection",async()=>{
  for (const scenario of ["managed-result","remote-result","changed-settings","changed-profile"]) {
    const r=runtime(),f=pickerFixture(r);r.direct();let calls=0,finish:any;f.picker.onSelectModel=()=>calls++;
    r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
    const start=r.api.sendCodexRetainedModelRequest({hostId:"local"},()=>new Promise(resolve=>{finish=resolve}),"thread/start",{});
    if(scenario==="changed-settings") r.replaceSettings();
    if(scenario==="changed-profile") r.configure("other","pureApi",["sol","gpt-5.5"]);
    finish({thread:{id:"one",...(scenario==="managed-result"?{modelProvider:"codex_plus_managed"}:{})},
      ...(scenario==="remote-result"?{host_id:"ssh"}:{})});
    await start;r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
    assert.equal(calls,0,scenario);
  }
});

it("disabling invalidates pending requests, timers and picker completions across re-enabling",async()=>{
  const r=runtime(),f=pickerFixture(r);r.direct();r.fresh();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  let finishRequest:any,finishPicker:any,calls=0;
  f.picker.onSelectModel=()=>{calls++;return new Promise(resolve=>{finishPicker=resolve});};
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls,1);
  const request=r.api.sendCodexRetainedModelRequest({hostId:"local"},()=>new Promise(resolve=>{finishRequest=resolve}),
    "thread/settings/update",{threadId:"one",model:"gpt-5.5",effort:"low"});
  const saved=r.storage.get("codexPlus.threadModels.v1"), counts=r.counts();
  r.enabled(false);r.enabled(true);finishRequest({});finishPicker({});await request;await Promise.resolve();
  r.runTimers();r.tick();r.runTimers();
  assert.deepEqual(r.counts(),counts);assert.equal(r.storage.get("codexPlus.threadModels.v1"),saved);assert.equal(calls,1);
});

it("master switch disables retention and invalidates pending work",async()=>{
  const r=runtime();r.direct();let finish:any;
  const request=r.api.sendCodexRetainedModelRequest({hostId:"local"},()=>new Promise(resolve=>{finish=resolve}),
    "thread/settings/update",{threadId:"one",model:"sol"});
  r.master(false);r.master(true);finish({});await request;
  assert.deepEqual(r.counts(),{reads:0,writes:0});
});

it("storage records are parsed once and invalidated after writes and other-window events",()=>{
  const key="codexPlus.threadModels.v1", record=(model:string,at:number)=>JSON.stringify([{key:JSON.stringify(["local","a","one"]),model,effort:"high",at}]);
  const r=runtime(new Map([[key,record("sol",1)]]));r.direct();
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
  const first=r.counts().reads, firstParses=r.parseCount();
  for(let index=0;index<5;index++) assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");
  assert.equal(r.counts().reads,first);assert.equal(r.parseCount(),firstParses);
  r.storage.set(key,record("gpt-5.5",2));r.storageEvent(key);
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"gpt-5.5");assert.equal(r.counts().reads,first+1);
  r.tick();r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"low"},{},"local",undefined,true);
  assert.equal(r.api.codexRetainedThreadModel("local","one").model,"sol");assert.equal(r.counts().reads,first+2);
  r.storage.clear();r.storageEvent(null);assert.equal(r.api.codexRetainedThreadModel("local","one"),null);
});

it("conflicting host aliases and nested envelopes are treated as remote",async()=>{
  const r=runtime(),f=pickerFixture(r);r.direct();r.fresh();let calls=0;f.picker.onSelectModel=()=>calls++;
  const send=r.api.sendCodexRetainedModelRequest;
  for (const [client,method,params] of [
    [{hostId:"local",host_id:"ssh"},"thread/settings/update",{threadId:"one",model:"sol"}],
    [{hostId:"local"},"thread/settings/update",{hostId:"local",host_id:"ssh",threadId:"one",model:"sol"}],
    [{hostId:"local"},"send-cli-request-for-host",{hostId:"local",host_id:"ssh",method:"thread/settings/update",params:{threadId:"one",model:"sol"}}],
    [{hostId:"local"},"send-cli-request-for-host",{hostId:"local",method:"thread/settings/update",params:{host_id:"ssh",threadId:"one",model:"sol"}}],
  ] as const) await send(client,async(_method:any,value:any)=>{assert.equal(value,params);return{};},method,params);
  (f.owner as any).host_id="ssh";
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.deepEqual(r.counts(),{reads:0,writes:0});assert.equal(calls,0);
});

it("all direct and enveloped resume payloads retain explicit and missing models and efforts",async()=>{
  const r=runtime();r.direct();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  for(const params of [
    {threadId:"one"},
    {threadId:"one",model:"gpt-5.5",reasoningEffort:"low",config:{model_reasoning_effort:"medium"}},
    {threadId:"one",config:{model_reasoning_effort:"ultra"}},
  ]) {
    for(const wrapped of [false,true]) {
      const value=wrapped?{hostId:"local",method:"thread/resume",params}:params;
      const method=wrapped?"send-cli-request-for-host":"thread/resume";
      const snapshot=JSON.stringify(value);
      await r.api.sendCodexRetainedModelRequest({hostId:"local"},async(actualMethod:any,actual:any)=>{
        assert.equal(actualMethod,method);assert.equal(actual,value);assert.equal(JSON.stringify(actual),snapshot);return{};
      },method,value);
    }
  }
});

it("per-model context refresh and managed connection recovery keep their own resume selections",async()=>{
  const r=runtime();r.direct();r.contextWindows('{"sol":100000}');
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  const client={hostId:"local",__codexPlusThreadModels:new Map([["one","sol"]])},requests:any[]=[];
  const native=async(method:string,params:any)=>{requests.push({method,params});return {modelProvider:"codex_plus_managed"};};
  const send=(method:string,params:any,options:any)=>r.api.sendCodexRetainedModelRequest(client,native,method,params,options);
  assert.equal(await r.api.refreshCodexThreadModelBeforeTurn(client,send,"turn/start",{threadId:"one",model:"gpt-5.5",effort:"low"},{}),true);
  assert.equal(requests[0].method,"thread/resume");assert.equal(requests[0].params.model,"gpt-5.5");
  r.managed(true);
  await r.api.reloadCodexManagedSessionConnection("one",{thread:{status:{type:"notLoaded"}}},
    {model:"gpt-5.5",effort:"low"},send,{},"managed");
  assert.equal(requests[1].method,"thread/unsubscribe");
  assert.equal(requests[2].method,"thread/resume");assert.equal(requests[2].params.model,"gpt-5.5");
  assert.equal(requests[2].params.config.model_reasoning_effort,"low");
});
