const assert = require("node:assert/strict");
const { test } = require("node:test");
const fs = require("node:fs");
const vm = require("node:vm");
const source = fs.readFileSync(require.resolve("./renderer-inject.js"), "utf8");
const begin = source.indexOf('  const officialUsageWindowMarker =');
const end = source.indexOf('  let zedRemoteStatusPromise', begin);
assert.ok(begin >= 0 && end > begin);
const profile = source.slice(source.indexOf('  function codexRemoteSessionActiveProfile()'), source.indexOf('  function codexRemoteSessionProviderPatchEnabled()'));
const install = `(function(){let codexPlusBackendSettings={};let codexPlusBackendSettingsLoaded=false;const sendCodexPlusDiagnostic=()=>{};${profile}\n${source.slice(begin, end)}})()`;
const settings = mixed => ({ relayProfilesEnabled: true, activeRelayId: "active", relayProfiles: [{ id: "active", relayMode: "official", officialMixApiKey: mixed }] });
const usage = (allowed = false) => ({ plan_type: "free", account_id: "account", user_id: "user", rate_limit: { allowed, limit_reached: !allowed, primary_window: { used_percent: allowed ? 0 : 100 } }, rate_limit_warning: { title: "low" } });

function fixture({ copyOnPublish = false } = {}) {
  const observers = [];
  class Element {
    constructor(tag = "div", text = "") { this.nodeType = 1; this.tagName = tag.toUpperCase(); this.children = []; this.attrs = {}; this.className = ""; this.text = text; this.parentElement = null; this.display = ""; this.priority = ""; this.style = { setProperty: (k,v,p) => { this.display=v;this.priority=p||""; }, removeProperty: () => { this.display="";this.priority=""; }, getPropertyValue:()=>this.display, getPropertyPriority:()=>this.priority }; }
    get textContent() { return this.text + this.children.map(x=>x.textContent).join(""); }
    set textContent(value) { this.text=value; }
    append(child) { child.parentElement=this;this.children.push(child);return child; }
    getAttribute(key) { return this.attrs[key] ?? null; }
    setAttribute(key,value) { this.attrs[key]=value; }
    removeAttribute(key) { delete this.attrs[key]; }
    hasAttribute(key) { return key in this.attrs; }
    matches(selector) { return selector.split(",").some(s=>{s=s.trim();return s==="aside"?this.tagName==="ASIDE":s==='[role="status"]'?this.attrs.role==="status":s.includes("data-codex-plus-official-usage-window")&&this.hasAttribute("data-codex-plus-official-usage-window");}); }
    querySelectorAll(selector) { return this.children.flatMap(c=>[...(c.matches(selector)?[c]:[]),...c.querySelectorAll(selector)]); }
  }
  const body = new Element("body");
  const domReady = new Set();
  const document = { body, querySelectorAll:s=>body.querySelectorAll(s), getElementById:()=>null, addEventListener:(name,f)=>domReady.add(f), removeEventListener:(name,f)=>domReady.delete(f) };
  class Observer { constructor(callback){this.callback=callback;this.active=false;observers.push(this);} observe(root,options){this.active=true;this.options=options;} disconnect(){this.active=false;} }
  const events = [], listeners = [], queries = [];
  class Query { constructor(key,data){this.queryKey=key;this.state={data};queries.push(this);} setData(data){const stored=copyOnPublish?structuredClone(data):data;this.state.data=stored;events.push({key:this.queryKey,allowed:stored?.rate_limit?.allowed});for(const f of listeners)f({type:"updated",query:this});return stored;} }
  const main = new Query(["rate-limit-status","user","account"], usage());
  const image = new Query(["rate-limit-status","image-generation"], usage());
  const cache = { getAll:()=>queries, findAll:()=>queries, find:({queryKey})=>queries.find(q=>JSON.stringify(q.queryKey)===JSON.stringify(queryKey)), subscribe:f=>{listeners.push(f);return()=>listeners.splice(listeners.indexOf(f),1);} };
  const client = { getQueryCache:()=>cache, setQueryData:(key,value)=>{const q=queries.find(q=>JSON.stringify(q.queryKey)===JSON.stringify(key));return q?.setData(typeof value==="function"?value(q.state.data):value);}, invalidateQueries:()=>new Promise(()=>{}) };
  const window = { __CODEX_PLUS_TEST_RATE_LIMIT_UNLOCK__:true, __REACT_QUERY_CLIENT__:client };
  const context = vm.createContext({window,document,Node:{ELEMENT_NODE:1},MutationObserver:Observer,setTimeout,clearTimeout});
  const load = () => {vm.runInContext(install,context);return window.__codexPlusRateLimitUnlockTest;};
  const inject = mixed => {const api=load();api.setBackendSettings(settings(mixed));api.install();return api;};
  const deliver = records => { for(const o of observers)if(o.active)o.callback(records); };
  const card = text => {const e=body.append(new Element("section",text));e.setAttribute("role","status");e.className="rounded-2xl ring-border";return e;};
  return {inject,load,window,document,body,Element,main,image,client,events,observers,deliver,card,domReady,listenerCount:()=>listeners.length};
}

test("reinjecting replaces quota policy in both directions before publication",()=>{
  for(const [first,second] of [[true,false],[false,true]]) {
    const f=fixture();f.inject(first);f.inject(second);f.main.setData(usage());
    assert.equal(f.events.at(-1).allowed,second);
    assert.equal(f.main.state.data.rate_limit.allowed,second);
    assert.equal(f.image.state.data.rate_limit.allowed,false);
  }
});
test("turning mixed mode off restores raw usage immediately even while refresh hangs",()=>{
  const f=fixture(),raw=f.main.state.data,api=f.inject(true);
  api.setBackendSettings(settings(false));api.install();
  assert.equal(f.main.state.data,raw);
  assert.equal(f.main.state.data.rate_limit.allowed,false);
  assert.equal(f.main.state.data.rate_limit.limit_reached,true);
  assert.equal(f.main.state.data.rate_limit_warning.title,"low");
});
test("reinjection retains the last known policy while its settings request is pending",()=>{
  const f=fixture();f.inject(true);const api=f.load();
  f.main.setData(usage());assert.equal(f.main.state.data.rate_limit.allowed,true);
  api.setBackendSettings(settings(false));api.install();assert.equal(f.main.state.data.rate_limit.allowed,false);
});
test("raw snapshots survive reinjection and follow the latest account response",()=>{
  const f=fixture();f.inject(true);
  const latest={...usage(),account_id:"new-account",rate_limit:{...usage().rate_limit,primary_window:{used_percent:87}}};
  f.main.setData(latest);f.inject(false);
  assert.equal(f.main.state.data,latest);
});
test("leaving relay mode restores raw data and repeated heartbeats do not republish",()=>{
  for(const patch of [{relayProfilesEnabled:false},{relayProfiles:[{id:"active",relayMode:"pureApi",officialMixApiKey:false}]}]) {
    const f=fixture(),raw=f.main.state.data,api=f.inject(true),published=f.events.length;
    for(let i=0;i<5;i++)api.install();
    assert.equal(f.events.length,published,"stable heartbeats should not rewrite sanitized cache data");
    api.setBackendSettings({...settings(true),...patch});api.install();
    assert.equal(f.main.state.data,raw);
  }
  const f=fixture();for(let i=0;i<5;i++)f.inject(i%2===0);
  assert.equal(f.listenerCount(),1,"reinjection should reuse the existing cache subscription");
});
test("functional cache updates see the real usage and preserve unrelated updates",()=>{
  const f=fixture(),api=f.inject(true);
  f.client.setQueryData(f.main.queryKey,old=>({...old,promo:{title:"new promo"}}));
  api.setBackendSettings(settings(false));api.install();
  assert.equal(f.main.state.data.rate_limit.allowed,false);
  assert.equal(f.main.state.data.promo.title,"new promo");
});
test("fresh real allowance replaces an older exhausted snapshot",()=>{
  const f=fixture(),api=f.inject(true),refilled=usage(true);
  f.main.setData(refilled);api.setBackendSettings(settings(false));api.install();
  assert.equal(f.main.state.data,refilled);
});
test("synchronous subscribers updating a structurally shared result preserve raw quota",()=>{
  const f=fixture({copyOnPublish:true}),api=f.inject(true);let updated=false;
  f.client.getQueryCache().subscribe(({query})=>{
    if(query!==f.main||updated)return;updated=true;
    f.client.setQueryData(query.queryKey,old=>({...old,promo:{title:"subscriber update"}}));
    f.client.setQueryData(query.queryKey,old=>({...old,extra:{title:"second update"}}));
  });
  f.main.setData(usage());api.setBackendSettings(settings(false));api.install();
  assert.equal(f.main.state.data.rate_limit.allowed,false);
  assert.equal(f.main.state.data.promo.title,"subscriber update");
  assert.equal(f.main.state.data.extra.title,"second update");
});
test("reinjection disposes the previous observer before leaving mixed mode",()=>{
  const f=fixture();f.inject(true);f.inject(true);f.inject(false);
  assert.equal(f.observers.filter(o=>o.active).length,0);
  const card=f.card("10% usage remaining");f.deliver([{type:"childList",target:f.body,addedNodes:[card]}]);
  assert.equal(card.display,"");
});
test("existing cards become hidden when text or nested children change",()=>{
  const f=fixture(),card=f.card("Loading");f.inject(true);
  assert.equal(f.observers.at(-1).options.characterData,true);
  card.textContent="10% usage remaining";
  f.deliver([{type:"characterData",target:{nodeType:3,parentElement:card},addedNodes:[]}]);
  assert.equal(card.display,"none");
  const aside=f.body.append(new f.Element("aside"));aside.className="rounded-3xl";
  const text=aside.append(new f.Element("span","Codex 和工作使用额度已用完"));
  f.deliver([{type:"childList",target:aside,addedNodes:[text]}]);
  assert.equal(aside.display,"none");
});
test("reused cards with ordinary text become visible and retain their original display",()=>{
  const f=fixture(),card=f.card("10% usage remaining");card.style.setProperty("display","flex","important");f.inject(true);
  card.textContent="Upload finished";f.deliver([{type:"characterData",target:{nodeType:3,parentElement:card},addedNodes:[]}]);
  assert.equal(card.display,"flex");assert.equal(card.priority,"important");
});
test("reinjecting before DOM readiness leaves only the newest observer",()=>{
  const f=fixture();f.document.body=null;f.inject(true);f.inject(true);f.document.body=f.body;
  for(const callback of [...f.domReady])callback();
  assert.equal(f.observers.filter(o=>o.active).length,1);
});
