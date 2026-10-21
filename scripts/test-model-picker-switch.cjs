const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { chromium } = require("playwright");

const read = name => fs.readFileSync(path.join(__dirname, "../assets/inject/renderer-inject", name), "utf8");
const catalogSource = read("70-model-catalog.js");
const names = [
  "uniqueValues", "codexPlusModelNames", "codexPlusModelMetadata", "modelReasoningEfforts",
  "applyCodexPlusModelMetadata", "codexPlusModelDescriptor", "modelArrayLooksPatchable",
  "modelFilterPolicy", "sameModelCollection", "prepareModelCollection", "finishModelCollection",
  "codexPureApiAllowedModelSet", "codexAggregateAllowedModelSet", "codexAggregateSelectedModelTarget",
  "codexAggregateModelPickers", "codexPlusConfiguredDefaultModel", "syncCodexConfiguredDefaultModel",
  "normalizeCodexConfiguredModelRequest", "modelResponseHostId", "patchModelArray",
  "cloneCodexPlusModelCollection", "patchCachedLocalModelQueryData",
  "transferCodexPlusModelCollectionSnapshot",
];
const functions = names.map(name => {
  const start = catalogSource.indexOf(`  function ${name}(`);
  const end = catalogSource.indexOf("\n  }\n", start) + 5;
  assert.ok(start >= 0 && end > start, name);
  return catalogSource.slice(start, end);
}).join("\n");
const fixture = `<!doctype html><html lang="zh-CN"><meta charset="utf-8">
<title>Model picker switch regression</title><style>
:root{--color-token-text-secondary:#666;--color-token-text-primary:#252525;--color-token-bg-secondary:#f4f4f4}
body{margin:16px;font:14px Arial;background:white;color:#252525}
button{font:inherit} [data-model-picker-view]{width:100%;max-width:390px}
[role=menuitemradio]{display:flex;width:100%;box-sizing:border-box;padding:8px;border:0;background:none;text-align:left}
[data-menu-row-content]{display:flex;width:100%}.flex-1{flex:1;min-width:0}
.native-check{width:12px;height:12px}
.truncate{display:flex;flex-direction:column}.subtitle{font-size:13px}
#trigger{margin-bottom:12px} output{display:block;margin-top:12px}
</style><button id="trigger" data-codex-intelligence-trigger>Custom</button>
<div role="menu" aria-labelledby="trigger"><div data-model-picker-view="simple"><div class="_ModelList_fixture"></div></div></div>
<output id="result"></output>
<script>
let codexModelCatalogAppliedEpoch = 1;
const codexPlusBackendSettingsLoaded = true;
const codexPlusBackendSettings = {};
const codexRemoteSessionActiveProfile = () => ({id:"aggregate",relayMode:"aggregate"});
const codexPlusModelUnlockEnabled = () => false;
const codexServiceTierModelFromValue = value => String(value || "");
const codexPlusModelCollectionSnapshots = new WeakMap();
const codexPlusModelDefaultSnapshots = new WeakMap();
const registerCodexPlusExtensionSelector = () => {};
const loadCodexModelCatalog = async () => codexModelCatalog;
const alpha = "gpt-6.1-sol", beta = "cpx-b/gpt-6.1-sol";
let codexModelCatalog = {status:"ok",model:alpha,default_model:alpha,
  models:[beta,alpha],codex_plus_default_group_id:"a",
  sources:[{id:"aggregate:aggregate",type:"aggregate_model_catalog"}],
  modelMetadata:{
    [beta]:{displayName:"GPT-6.1-Sol（Beta）",codex_plus_group:"Beta",codex_plus_group_id:"b"},
    [alpha]:{displayName:"GPT-6.1-Sol",codex_plus_group:"Alpha",codex_plus_group_id:"a",codex_plus_default:true}
  }};
${functions}
let nativeDefaultCalls = 0;
const picker = {model:"unconfigured-compat",reasoningEffort:"high",models:[],
  onSelectModel(model,effort) {this.model=model;this.reasoningEffort=effort;document.querySelector("#trigger").textContent=
    this.models.find(entry=>entry.model===model)?.displayName || "Custom";
    document.querySelector("#result").textContent=model;}};
const owner = {permissionsHostId:"local",conversationId:""};
document.querySelector("#trigger").__reactFiber$fixture = {memoizedProps:picker,return:{memoizedProps:owner}};
function renderNative(current = null) {
  const native = current || [{model:"unconfigured-compat"},{model:beta},{model:alpha}];
  patchModelArray(native,true,"local");picker.models=native;
  const list=document.querySelector("._ModelList_fixture"); list.replaceChildren();
  const heading=document.createElement("div");heading.setAttribute("data-menu-section-label","");heading.textContent="选择模型";list.append(heading);
  const add = (label,subtitle,onClick) => {
    const row=document.createElement("button");row.setAttribute("role","menuitemradio");
    row.innerHTML='<div data-menu-row-content><span class="flex-1"><span class="truncate"><span></span></span></span></div>';
    row.querySelector(".truncate > span").textContent=label;
    if(subtitle){
      row.innerHTML='<div data-menu-row-content><div class="flex min-w-0 flex-1 flex-col"><span class="min-w-0 truncate"></span><span class="subtitle"><span></span></span></div></div>';
      row.querySelector(".truncate").textContent=label;
      row.querySelector(".subtitle > span").textContent=subtitle;
    }
    row.addEventListener("click",onClick);list.append(row);return row;
  };
  const defaultRow=add("默认","推荐模型集",()=>{
    nativeDefaultCalls++;
    picker.onSelectModel("gpt-5.6-sol","low");
  });
  defaultRow.setAttribute("data-model-selected","");
  defaultRow.setAttribute("aria-checked","true");
  defaultRow.querySelector("[data-menu-row-content]").insertAdjacentHTML("beforeend",'<svg class="native-check" viewBox="0 0 12 12"><path d="m2 6 3 3 5-6"/></svg>');
  defaultRow.addEventListener("keydown",event=>{
    if(["Enter"," "].includes(event.key))defaultRow.click();
  });
  for(const model of native) add(model.displayName || model.model,"",()=>picker.onSelectModel(model.model,"high"));
}
renderNative();
${read("71-model-picker-groups.js")}
syncCodexConfiguredDefaultModel();
window.fixture = {
  state:()=>({model:picker.model,effort:picker.reasoningEffort,nativeDefaultCalls,models:picker.models.map(m=>m.model)}),
  selectOther:()=>picker.onSelectModel(beta,"high"),
  unpublished:()=>{
    codexModelCatalog.status="not_applied";
    window.__codexPlusModelPickerGroupsRuntime.scan();
    return {automaticDefault:codexPlusConfiguredDefaultModel(),allowed:codexAggregateAllowedModelSet("local")};
  },
  published:()=>{codexModelCatalog.status="ok";window.__codexPlusModelPickerGroupsRuntime.scan();},
  failBeforeClick:(key)=>{
    codexModelCatalog.status="failed";
    const row=document.querySelector(".codex-plus-model-default-row");
    if(key)row.dispatchEvent(new KeyboardEvent("keydown",{key,bubbles:true,cancelable:true}));
    else row.click();
  },
  menuOwner:(label,scan=true)=>{
    const menu=document.querySelector('[role="menu"]');
    if(label===null)menu.removeAttribute("aria-labelledby");
    else menu.setAttribute("aria-labelledby",label);
    if(scan)window.__codexPlusModelPickerGroupsRuntime.scan();
  },
  available:(enabled)=>{picker.models=enabled?[{model:alpha},{model:beta}]:[{model:beta}];},
  changeDefault:(other)=>{
    codexModelCatalog.model=codexModelCatalog.default_model=other?beta:alpha;
    window.__codexPlusModelPickerGroupsRuntime.scan();
  },
  addTemporaryModel:()=>{
    const model="cpx-b/test";
    codexModelCatalog.models.splice(1,0,model);
    codexModelCatalog.modelMetadata[model]={displayName:"Test（Beta）",codex_plus_group:"Beta",codex_plus_group_id:"b"};
    codexModelCatalogAppliedEpoch++;
    renderNative();window.__codexPlusModelPickerGroupsRuntime.scan();
  },
  deleteTemporaryModel:()=>{
    codexModelCatalog.models=codexModelCatalog.models.filter(model=>model!=="cpx-b/test");
    delete codexModelCatalog.modelMetadata["cpx-b/test"];
    codexModelCatalog.status="not_applied";
    codexModelCatalogAppliedEpoch++;codexPlusModelPickerCatalogChanged();
    renderNative(patchCachedLocalModelQueryData(picker.models));
    window.__codexPlusModelPickerGroupsRuntime.scan();
  },
  switchPool:()=>{
    codexModelCatalog={...codexModelCatalog,models:[alpha],sources:[{id:"aggregate:aggregate",type:"aggregate_pool_catalog"}],
      codex_plus_default_group_id:"",modelMetadata:{[alpha]:{displayName:"GPT-6.1-Sol"}}};
    codexModelCatalogAppliedEpoch++;codexPlusModelPickerCatalogChanged();renderNative();
    window.__codexPlusModelPickerGroupsRuntime.scan();
  },
  send:async()=>{
    const params=normalizeCodexConfiguredModelRequest("turn/start",{threadId:"isolated",model:picker.model,effort:picker.reasoningEffort},"local");
    return (await fetch("/responses",{method:"POST",body:JSON.stringify(params)})).json();
  }
};
window.__codexPlusModelPickerGroupsRuntime.scan();
</script></html>`;

(async () => {
  const requests = [];
  const server = http.createServer((req, res) => {
    if (req.method === "POST") {
      let body = ""; req.on("data", part => { body += part; });
      req.on("end", () => { requests.push(JSON.parse(body)); res.setHeader("Content-Type", "application/json"); res.end('{"ok":true}'); });
    } else { res.setHeader("Content-Type", "text/html; charset=utf-8"); res.end(fixture); }
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let browser, attachedPage, originalUrl;
  try {
    const endpoint = process.env.CODEX_TEST_CDP_ENDPOINT;
    browser = endpoint ? await chromium.connectOverCDP(endpoint) : await chromium.launch({ channel: "msedge", headless: true });
    const context = endpoint ? browser.contexts()[0] : await browser.newContext();
    const page = endpoint ? context.pages()[0] : await context.newPage();
    if (endpoint) {
      attachedPage = page; originalUrl = page.url();
      assert.ok(originalUrl.startsWith("http://tauri.localhost/"), "only attach to an isolated manager");
    }
    const errors = []; page.on("pageerror", error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.waitForFunction(() => window.fixture?.state().model === "gpt-6.1-sol");
    assert.equal(await page.locator("#trigger").textContent(), "GPT-6.1-Sol");
    assert.deepEqual(await page.locator(".codex-plus-model-group-title").allTextContents(), ["Beta", "Alpha"]);
    const defaultRow = page.locator(".codex-plus-model-default-row");
    assert.equal((await defaultRow.innerText()).replace(/\s+/g, " "), "默认模型 GPT-6.1-Sol");
    assert.equal(await defaultRow.locator(".subtitle").isVisible(), false);
    assert.equal(await defaultRow.locator(".native-check").isVisible(), false,
      "configured default action must not inherit the native default selection mark");
    await page.waitForFunction(() => document.querySelectorAll(".codex-plus-model-current-row").length === 1);
    assert.match(await page.locator(".codex-plus-model-current-row").innerText(), /GPT-6\.1-Sol/);
    const metrics = await page.evaluate(() => {
      const a=document.querySelector(".codex-plus-model-default-row");
      const b=document.querySelector(".codex-plus-model-group-header");
      return [getComputedStyle(a).fontSize,getComputedStyle(b).fontSize,
        a.getBoundingClientRect().height,b.getBoundingClientRect().height];
    });
    assert.equal(metrics[0], metrics[1]); assert.ok(Math.abs(metrics[2]-metrics[3]) <= 2, JSON.stringify(metrics));
    await defaultRow.click();
    assert.equal(await page.locator("#result").textContent(), "gpt-6.1-sol");
    for (const key of ["Enter", "Space"]) {
      await page.evaluate(() => window.fixture.selectOther());
      await defaultRow.press(key);
      assert.equal(await page.locator("#result").textContent(), "gpt-6.1-sol");
    }
    assert.equal(await page.evaluate(() => window.fixture.state().nativeDefaultCalls), 0);
    assert.equal(await page.evaluate(() => window.fixture.state().effort), "high");
    await page.evaluate(() => window.fixture.changeDefault(true));
    await defaultRow.click();
    assert.equal(await page.locator("#result").textContent(), "cpx-b/gpt-6.1-sol");
    await page.evaluate(() => window.fixture.changeDefault(false));
    await defaultRow.click();
    await page.evaluate(() => window.fixture.send());
    assert.equal(requests[0].model, "gpt-6.1-sol", "actual HTTP body follows the configured default action");
    assert.deepEqual(await page.evaluate(() => window.fixture.unpublished()), { automaticDefault: "", allowed: null });
    assert.equal((await defaultRow.innerText()).replace(/\s+/g, " "), "默认模型 GPT-6.1-Sol");
    await page.evaluate(() => window.fixture.selectOther());
    await page.waitForFunction(() => document.querySelector(".codex-plus-model-current-row")?.textContent?.includes("Beta"));
    assert.equal(await defaultRow.locator(".native-check").isVisible(), false);
    await defaultRow.click();
    assert.equal(await page.locator("#result").textContent(), "gpt-6.1-sol");
    assert.equal(await page.evaluate(() => window.fixture.state().nativeDefaultCalls), 0);
    await page.evaluate(() => { window.fixture.selectOther(); window.fixture.available(false); });
    await defaultRow.click();
    assert.equal(await page.locator("#result").textContent(), "cpx-b/gpt-6.1-sol");
    assert.equal(await page.evaluate(() => window.fixture.state().nativeDefaultCalls), 0,
      "unavailable configured model must not fall through to stale native default");
    await page.evaluate(() => window.fixture.available(true));
    await defaultRow.click();
    await page.evaluate(() => window.fixture.published());
    for (const key of [null, "Enter", " "]) {
      await page.evaluate(key => {
        window.fixture.selectOther();
        window.fixture.failBeforeClick(key);
      }, key);
      assert.equal(await page.locator("#result").textContent(), "cpx-b/gpt-6.1-sol");
      assert.equal(await page.evaluate(() => window.fixture.state().nativeDefaultCalls), 0,
        "catalog failure before rescan must not fall through to the native default");
      await page.evaluate(() => window.fixture.published());
    }
    await page.evaluate(() => {
      window.fixture.menuOwner("remote-trigger", false);
      document.querySelector(".codex-plus-model-default-row").click();
    });
    assert.equal(await page.evaluate(() => window.fixture.state().nativeDefaultCalls), 0,
      "stale local row cannot invoke a remote menu's native action before rescan");
    for (const owner of ["remote-trigger", null]) {
      await page.evaluate(owner => window.fixture.menuOwner(owner), owner);
      assert.equal(await page.locator(".codex-plus-model-default-row").count(), 0);
      const nativeRow = page.locator('[role="menuitemradio"]').filter({ hasText: "推荐模型集" });
      assert.equal(await nativeRow.locator(".subtitle").isVisible(), true);
      const before = await page.evaluate(() => window.fixture.state().nativeDefaultCalls);
      await nativeRow.click();
      assert.equal(await page.evaluate(() => window.fixture.state().nativeDefaultCalls), before + 1,
        "unpatched remote or unidentified menu keeps its native default action");
      assert.equal(await page.locator("#result").textContent(), "gpt-5.6-sol");
    }
    await page.evaluate(() => { window.fixture.menuOwner("trigger"); window.fixture.selectOther(); });
    assert.equal((await defaultRow.innerText()).replace(/\s+/g, " "), "默认模型 GPT-6.1-Sol");
    await defaultRow.click();
    fs.mkdirSync(path.join(__dirname, "../output/playwright"), { recursive: true });
    for (const width of [480, 320]) {
      await page.setViewportSize({ width, height: 500 });
      await page.screenshot({ path: path.join(__dirname, "../output/playwright/model-picker-" + width + ".png") });
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      const layout = await page.evaluate(() => {
        const row = document.querySelector(".codex-plus-model-default-row").getBoundingClientRect();
        const caption = document.querySelector(".codex-plus-model-default-caption").getBoundingClientRect();
        const value = document.querySelector(".codex-plus-model-default-value").getBoundingClientRect();
        const model = [...document.querySelectorAll("[data-codex-plus-model-group] .codex-plus-model-group-own-label")]
          .find(node => node.getClientRects().length)?.getBoundingClientRect();
        return { rightGap: row.right - value.right, sameLine: Math.abs(caption.y - value.y) < 1,
          separated: caption.right < value.left, indent: model?.left - row.left };
      });
      assert.ok(layout.rightGap <= 10 && layout.sameLine && layout.separated, JSON.stringify(layout));
      assert.ok(layout.indent >= 28, JSON.stringify(layout));
    }
    await page.evaluate(() => { window.fixture.switchPool(); });
    assert.deepEqual(await page.evaluate(() => window.fixture.state().models), ["gpt-6.1-sol"]);
    await page.evaluate(() => window.fixture.send());
    assert.equal(requests.length, 2); assert.equal(requests[1].model, "gpt-6.1-sol");
    assert.equal(requests[1].effort, "high");
    await page.reload();
    await page.evaluate(() => window.fixture.addTemporaryModel());
    assert.equal(await page.locator(".codex-plus-model-group-own-label").filter({ hasText: /^Test$/ }).count(), 1);
    await page.evaluate(() => window.fixture.deleteTemporaryModel());
    assert.equal(await page.locator('[role="menuitemradio"]').filter({ hasText: /Test/ }).count(), 0);
    assert.equal(await page.locator(".codex-plus-model-group-header").filter({ hasText: "Beta" }).count(), 1,
      "deleted model must not create an extra legacy provider group");
    assert.deepEqual(await page.evaluate(() => window.fixture.state().models), ["cpx-b/gpt-6.1-sol", "gpt-6.1-sol"]);
    assert.equal(await defaultRow.locator(".native-check").isVisible(), false);
    assert.equal(await page.locator(".codex-plus-model-current-row").count(), 1,
      "catalog deletion must keep exactly one concrete current-model marker");
    await page.screenshot({ path: path.join(__dirname, "../output/playwright/model-picker-deleted-model.png") });
    assert.deepEqual(errors, []);
    console.log("PASS: model/default/group layout, member order, pool withdrawal, unpublished deletion and HTTP request");
  } finally {
    if (attachedPage && !attachedPage.isClosed()) await attachedPage.goto(originalUrl).catch(() => {});
    await browser?.close();
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
