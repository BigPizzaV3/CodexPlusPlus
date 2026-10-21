import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
const source = await readFile(new URL("../../../assets/inject/renderer-inject/70-model-catalog.js", import.meta.url), "utf8");
function runtime(storage = new Map<string,string>(), sharedWindow: any = {}) {
  let profile = "aggregate", now = 10000;
  const catalog = { status: "ok", codex_plus_default_group_id: "a" };
  let names = ["gpt-5.5", "sol", "cpx-b/sol", "cpx-a/sol"];
  let mode = "aggregate";
  let supported = ["low", "medium", "high", "xhigh", "max", "ultra"];
  const effortByModel: Record<string,string[]> = {};
  const nodes: any[] = [];
  const context = vm.createContext({
    window: sharedWindow, document: { querySelectorAll: () => nodes },
    localStorage: { getItem: (k: string) => storage.get(k), setItem: (k:string,v:string) => storage.set(k,v) },
    Date: { now: () => now }, Map, WeakMap, Promise, Set,
    codexPlusBackendSettingsLoaded: true,
    codexPlusBackendSettings: { relayProfiles: [{id:"a",relayMode:"pureApi"}] },
    codexRemoteSessionActiveProfile: () => ({ id: profile, relayMode: mode }),
    codexModelCatalog: catalog,
    codexPlusModelNames: () => names,
    modelReasoningEfforts: (model:string) => (effortByModel[model] || supported).map(reasoningEffort=>({reasoningEffort})),
    codexPlusModelMetadata: (model:string) => ({ defaultReasoningEffort:"medium",
      codex_plus_group_id: mode==="pureApi" ? undefined : model.startsWith("cpx-") ? model.slice(4,model.indexOf("/")) : catalog.codex_plus_default_group_id }),
  });
  const start = source.indexOf("  function codexRememberedThreadModels()");
  const end = source.indexOf("  function codexPerModelContextEnabled()", start);
  const api = vm.runInContext(`(()=>{${source.slice(start,end)};return {rememberCodexThreadModel,codexRetainedThreadModel,retainCodexThreadResumeModel,sendCodexRetainedModelRequest,syncCodexRetainedThreadModels,observeCodexThreadModelSelection}})()`,context);
  return { api, storage, catalog, nodes, effortByModel, profile: (v:string) => { profile=v; },
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
  const resume=r.api.retainCodexThreadResumeModel("thread/resume",{threadId:"one",model:"sol",reasoningEffort:"low"},"local");
  assert.equal(resume.model,"cpx-a/sol");assert.equal(resume.reasoningEffort,"high");
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
  const original={threadId:"one",model:"different",reasoningEffort:"low"};
  assert.equal(r.api.retainCodexThreadResumeModel("thread/resume",original,"local"),original);
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
it("resume fills missing model but respects an explicit context/model switch",()=>{
  const r=runtime();r.api.rememberCodexThreadModel("turn/start",{threadId:"one",model:"sol",effort:"low"},{},"local");
  assert.equal(r.api.retainCodexThreadResumeModel("thread/resume",{threadId:"one"},"local").model,"sol");
  assert.equal(r.api.retainCodexThreadResumeModel("thread/resume",{threadId:"one",model:"gpt-5.5"},"local").model,"gpt-5.5");
});
it("same-model resumes restore saved effort while explicit efforts and other models stay intact",()=>{
  const r=runtime();
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  const restarted=runtime(r.storage);
  assert.equal(restarted.api.retainCodexThreadResumeModel("thread/resume",{threadId:"one",model:"sol"},"local").reasoningEffort,"high");
  assert.equal(restarted.api.retainCodexThreadResumeModel("thread/resume",{threadId:"one",model:"sol",reasoningEffort:"low"},"local").reasoningEffort,"low");
  assert.equal(restarted.api.retainCodexThreadResumeModel("thread/resume",{threadId:"one",model:"gpt-5.5"},"local").reasoningEffort,undefined);
});
it("native power slider pointer and keyboard commits survive renderer restart and thread unload",async()=>{
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
    assert.deepEqual(calls,[["sol","high"]]);
    await Promise.resolve();
    fresh.owner.conversationId="two";next.api.syncCodexRetainedThreadModels();
    fresh.owner.conversationId="one";fresh.picker.reasoningEffort="low";
    next.api.syncCodexRetainedThreadModels();next.tick();next.api.syncCodexRetainedThreadModels();
    assert.equal(calls.length,2);
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
it("stable native fallback restores once; pending user selection and locked pickers are respected",async()=>{
  const r=runtime(); const calls:any[]=[];
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol",effort:"high"},{},"local");
  const picker={model:"gpt-5.5",reasoningEffort:"low",models:[{model:"sol"}],onSelectModel:(...v:any[])=>{calls.push(v);picker.model=v[0];picker.reasoningEffort=v[1];}};
  const node={__reactFiber$test:{memoizedProps:picker,return:{memoizedProps:{conversationId:"one",threadId:"wrong",permissionsHostId:"local"}}}};
  r.nodes.push(node);
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();
  assert.deepEqual(calls,[["sol","high"]]);r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,1);
  await Promise.resolve();
  picker.model="sol";picker.reasoningEffort="high";r.api.syncCodexRetainedThreadModels();
  picker.model="gpt-5.5";r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,2);
  let done:any; const p=r.api.sendCodexRetainedModelRequest({hostId:"local"},()=>new Promise(resolve=>{done=resolve}),"thread/settings/update",{threadId:"one",model:"gpt-5.5",effort:"low"});
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,2);done({});await p;
  r.api.syncCodexRetainedThreadModels();r.tick();r.api.syncCodexRetainedThreadModels();assert.equal(calls.length,2);
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
it("failed restores retry when native callback is ready and stop after three attempts",async()=>{
  const r=runtime(),f=pickerFixture(r);let failed=0,ready=0;
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
  assert.equal(failed,4);
});
it("undefined native callback results are verified from picker state and do not spin",async()=>{
  const r=runtime(),f=pickerFixture(r);let calls=0;
  r.api.rememberCodexThreadModel("thread/settings/update",{threadId:"one",model:"sol"},{},"local");
  f.picker.onSelectModel=()=>{calls++};
  r.api.syncCodexRetainedThreadModels();
  for(let i=0;i<8;i++){r.tick();r.api.syncCodexRetainedThreadModels();await Promise.resolve();}
  assert.equal(calls,3);
});
it("successful duplicate settings acknowledgements cannot reset the restore retry budget",async()=>{
  const r=runtime(),f=pickerFixture(r);let calls=0;
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
  assert.equal(actual.params.model,"sol");assert.equal(actual.params.reasoningEffort,"high");
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
