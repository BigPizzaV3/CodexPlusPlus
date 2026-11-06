import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { after, before, describe, it } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

export const VIEW_MODE_KEY = "codexPlus.modelPicker.viewMode.v1";
export const VIEW_LABELS = {
  list: "切换为列表视图",
  icons: "切换为图标视图",
  expand: "全部展开",
  collapse: "全部折叠",
};
const repository = fileURLToPath(new URL("../../../", import.meta.url));
const shardPath = join(repository, "assets/inject/renderer-inject/71-model-picker-groups.js");
const require = createRequire(import.meta.url);

export async function launchPickerBrowser() {
  const candidates = process.env.CODEX_PLUS_PLAYWRIGHT_MODULE ? [process.env.CODEX_PLUS_PLAYWRIGHT_MODULE] : [
    "playwright",
    join(homedir(), ".cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright"),
  ];
  let playwright: any;
  for (const candidate of candidates) {
    try { playwright = require(candidate); break; } catch {}
  }
  assert.ok(playwright, "Playwright is required; use an existing package via CODEX_PLUS_PLAYWRIGHT_MODULE.");
  return await playwright.chromium.launch({
    headless: true,
    ...(process.platform === "win32" ? { channel: "msedge" } : {}),
  });
}

export function pickerFixtureHtml() {
  return `<!doctype html>
<html lang="zh-CN">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<link rel="icon" href="data:,">
<title>Model picker DOM fixture</title>
<style>
  :root{color-scheme:dark;font:13px/1.4 "Segoe UI",sans-serif;--color-token-bg-primary:#292929;--color-token-bg-secondary:#343434;--color-token-text-primary:#ededed;--color-token-text-secondary:#b5b5b5}
  *{box-sizing:border-box}body{margin:0;padding:12px;background:#202020;color:#ededed}
  button{font:inherit;color:inherit;letter-spacing:0}button:focus-visible{outline:2px solid #82bba1}
  #fixture-surface{width:360px;max-width:100%;margin:0 auto}
  [data-codex-intelligence-trigger]{height:30px;background:#292929;border:1px solid #555;border-radius:5px;margin-bottom:8px;padding:0 9px}
  [role="menu"]{width:100%;padding:6px;background:#292929;border:1px solid #505050;border-radius:8px}
  .sectionLabel-fixture{padding:8px;display:flex;align-items:center;min-height:36px;font-size:12px;color:#b5b5b5}
  [role="menuitemradio"]{display:block;width:100%;min-width:0;background:transparent;border:0;text-align:left;padding:8px;border-radius:4px;cursor:pointer}
  [role="menuitemradio"]:hover{background:#363636}
  [data-menu-row-content]{display:flex;align-items:center;gap:6px;min-width:0}
  .flex-1{flex:1;min-width:0}.truncate{display:flex;align-items:center;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .native-default-label{flex-direction:column;align-items:flex-start}.native-recommendation{font-size:11px;color:#a1a1a1}
  .native-check{height:14px;width:14px;flex:none}[aria-checked="false"] .native-check{visibility:hidden}
  .fixture-footer{padding:10px 8px 4px;font-size:11px;color:#a1a1a1}
</style>
<div id="fixture-surface">
  <button type="button" data-codex-intelligence-trigger>GPT-6.1-Sol · xhigh</button>
  <div id="fixture-menu-mount"></div>
</div>
<script>
  var fixtureModels = [
    {slug:"fusheng/gpt-6.1-sol",label:"6.1 Sol（浮生）",full:"GPT-6.1-Sol（浮生）",provider:"浮生",group:"provider-fusheng"},
    {slug:"hanhe/gpt-6.1-terra",label:"6.1 Terra（寒鹤）",full:"GPT-6.1-Terra（寒鹤）",provider:"寒鹤",group:"provider-hanhe"},
    {slug:"fusheng/gpt-6.1-terra",label:"6.1 Terra（浮生）",full:"GPT-6.1-Terra（浮生）",provider:"浮生",group:"provider-fusheng"},
    {slug:"group/1",label:"模型组 1",full:"模型组 1",provider:"模型组",group:"codex-plus-gateway-groups"},
    {slug:"hanhe/gpt-6-sol",label:"6 Sol（寒鹤）",full:"GPT-6-Sol（寒鹤）",provider:"寒鹤",group:"provider-hanhe"},
    {slug:"fusheng/gpt-6-sol",label:"6 Sol（浮生）",full:"GPT-6-Sol（浮生）",provider:"浮生",group:"provider-fusheng"},
    {slug:"auto/balanced",label:"自动均衡",full:"自动均衡",provider:"自动路由",group:"codex-plus-gateway-auto"},
    {slug:"fusheng/gpt-5.6-sol",label:"5.6 Sol（浮生）",full:"GPT-5.6-Sol（浮生）",provider:"浮生",group:"provider-fusheng"},
    {slug:"hanhe/deepseek-v4",label:"DeepSeek V4（寒鹤）",full:"DeepSeek V4（寒鹤）",provider:"寒鹤",group:"provider-hanhe"},
    {slug:"fusheng/gpt-6-astra",label:"6 Astra（浮生）",full:"GPT-6-Astra（浮生）",provider:"浮生",group:"provider-fusheng"}
  ];
  var codexModelCatalog = {
    status:"ok",effective_config_model:"hanhe/gpt-6.1-terra",
    model:"hanhe/gpt-6.1-terra",default_model:"hanhe/gpt-6.1-terra",
    codex_plus_default_group_id:"provider-hanhe",models:fixtureModels.map(item=>item.slug),
    modelMetadata:Object.fromEntries(fixtureModels.map(item=>[item.slug,{
      displayName:item.full,codex_plus_group:item.provider,codex_plus_group_id:item.group,
      codex_plus_default:item.group==="provider-hanhe",codex_plus_favorite:item.slug==="fusheng/gpt-6.1-sol",
      supportedReasoningEfforts:[{reasoningEffort:"low"},{reasoningEffort:"xhigh"},{reasoningEffort:"ultra"}]
    }]))
  };
  var codexPlusBackendSettings = {
    enhancementsEnabled:true,relayProfilesEnabled:true,activeRelayId:"fixture-gateway",
    relayProfiles:[{id:"fixture-gateway",name:"Fixture gateway",relayMode:"aggregate"}]
  };
  var codexPlusBackendSettingsLoaded = true;
  var __codexPlusBuiltinInactive = new Set();
  var fixtureState = {model:"fusheng/gpt-6.1-sol",effort:"xhigh",nativeClicks:[],reparented:[],catalogLoads:0,selectors:[],diagnostics:[]};
  var fixturePickerProps = {
    model:fixtureState.model,reasoningEffort:fixtureState.effort,
    models:fixtureModels.map(item=>({model:item.slug,displayName:item.full})),
    hostId:"local",conversationId:"fixture-thread-a",
    onSelectModel(model,effort){
      fixtureState.model=model;fixtureState.effort=effort;
      fixturePickerProps.model=model;fixturePickerProps.reasoningEffort=effort;
      fixtureRememberedRows.forEach(row=>row.setAttribute("aria-checked",String(row.dataset.fixtureModel===model)));
    }
  };
  var fixtureRememberedRows = [];
  var fixtureParents = [];
  var fixtureOriginalAttributes = [];
  var fixtureMovementObserver;
  function fixtureCreateRow(item){
    const row=document.createElement("button");
    row.type="button";row.setAttribute("role","menuitemradio");
    row.dataset.fixtureModel=item.slug;row.setAttribute("aria-checked",String(fixtureState.model===item.slug));
    row.innerHTML='<span data-menu-row-content><span class="flex-1"><span class="truncate"><span></span></span></span><svg class="native-check" viewBox="0 0 16 16" aria-hidden="true"><path d="m3 8 3 3 7-7" fill="none" stroke="currentColor" stroke-width="1.5"/></svg></span>';
    row.querySelector(".truncate>span").textContent=item.label;
    row.__reactFiber$fixture={key:item.slug,memoizedProps:{model:item.slug},return:{memoizedProps:fixturePickerProps,return:null}};
    row.addEventListener("click",()=>{
      fixtureState.nativeClicks.push(item.slug);
      fixturePickerProps.onSelectModel(item.slug,fixtureState.effort);
    });
    return row;
  }
  function fixtureMountMenu(){
    fixtureMovementObserver?.disconnect();
    const menu=document.createElement("div");
    menu.setAttribute("role","menu");menu.setAttribute("data-model-picker-view","simple");
    menu.innerHTML='<div class="ModelList-fixture"><div class="sectionLabel-fixture" data-menu-section-label><span id="native-model-title">选择模型</span></div><button id="native-default-row" type="button" role="menuitemradio" aria-checked="false"><span data-menu-row-content><span class="flex-1"><span class="truncate native-default-label"><span>默认</span><span class="native-recommendation">推荐模型集</span></span></span><svg class="native-check" viewBox="0 0 16 16" aria-hidden="true"><path d="m3 8 3 3 7-7" fill="none" stroke="currentColor" stroke-width="1.5"/></svg></span></button></div><div class="fixture-footer">推理强度 · xhigh</div>';
    const list=menu.querySelector(".ModelList-fixture");
    fixtureRememberedRows=fixtureModels.map(fixtureCreateRow);
    fixtureRememberedRows.forEach(row=>list.appendChild(row));
    const defaultRow=menu.querySelector("#native-default-row");
    defaultRow.__reactFiber$fixture={key:"default",memoizedProps:{},return:{memoizedProps:fixturePickerProps,return:null}};
    defaultRow.addEventListener("click",()=>{fixtureState.nativeClicks.push("default");fixturePickerProps.onSelectModel(codexModelCatalog.default_model,"ultra")});
    document.querySelector("#fixture-menu-mount").replaceChildren(menu);
    document.querySelector("[data-codex-intelligence-trigger]").__reactFiber$fixture={memoizedProps:fixturePickerProps,return:null};
    fixtureParents=fixtureRememberedRows.map(row=>row.parentElement);
    fixtureOriginalAttributes=fixtureRememberedRows.map(row=>({className:row.className,style:row.getAttribute("style")}));
    fixtureObserveMovement();
    return menu;
  }
  function fixtureObserveMovement(){
    fixtureMovementObserver=new MutationObserver(records=>{
      for(const record of records) for(const node of record.removedNodes){
        for(const row of fixtureRememberedRows) if(node===row||node.contains?.(row)) fixtureState.reparented.push(row.dataset.fixtureModel);
      }
    });
    fixtureMovementObserver.observe(document.querySelector("#fixture-menu-mount"),{childList:true,subtree:true});
  }
  function fixtureAudit(){
    const list=document.querySelector(".ModelList-fixture");
    const actual=[...list.querySelectorAll("[data-fixture-model]")];
    return {
      ids:actual.map(row=>row.dataset.fixtureModel),
      sameNodes:fixtureRememberedRows.every((row,index)=>row===actual[index]),
      sameParents:fixtureRememberedRows.every((row,index)=>row.parentElement===fixtureParents[index]),
      moved:[...fixtureState.reparented],model:fixtureState.model,effort:fixtureState.effort,
      clicks:[...fixtureState.nativeClicks]
    };
  }
  function registerCodexPlusExtensionSelector(value){fixtureState.selectors.push(value)}
  function sendCodexPlusDiagnostic(event,detail){fixtureState.diagnostics.push({event,detail})}
  function codexPlusSettings(){return {modelWhitelistUnlock:true}}
  function visibleElement(node){return !!node?.isConnected&&node.getClientRects().length>0&&getComputedStyle(node).display!=="none"}
  function codexServiceTierModelFromValue(value){return typeof value==="string"?value:value?.model||""}
  function codexRemoteSessionActiveProfile(){return codexPlusBackendSettings.relayProfiles[0]}
  function codexPlusModelMetadata(slug){return codexModelCatalog.modelMetadata?.[slug]||null}
  function modelResponseHostId(){return "local"}
  function locationThreadId(){return fixturePickerProps.conversationId}
  async function loadCodexModelCatalog(){fixtureState.catalogLoads++;return codexModelCatalog}
  fixtureMountMenu();
</script>
</html>`;
}

export async function createPickerFixture(browser: any, options: {
  width?: number; height?: number; source?: string; storage?: Record<string, string>;
} = {}) {
  const html = pickerFixtureHtml();
  const server = createServer((_request, response) => {
    response.writeHead(200, { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" });
    response.end(html);
  });
  await new Promise<void>((done, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", done); });
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  const context = await browser.newContext({ viewport: { width: options.width || 455, height: options.height || 960 },
    locale: "zh-CN", reducedMotion: "reduce" });
  const page = await context.newPage();
  page.setDefaultTimeout(1500);
  const errors: string[] = [];
  page.on("pageerror", (error: Error) => errors.push(error.message));
  const content = options.source || await readFile(shardPath, "utf8");
  try {
    await page.goto(`http://127.0.0.1:${address.port}`, { waitUntil: "domcontentloaded" });
    if (options.storage) await page.evaluate((storage: Record<string, string>) => {
      for (const [key, value] of Object.entries(storage)) localStorage.setItem(key, value);
    }, options.storage);
    const inject = async (scan = true) => {
      await page.addScriptTag({ content: `(() => {\n${content}\n})();` });
      if (scan) await page.evaluate(() => (window as any).__codexPlusModelPickerGroupsRuntime.scan());
    };
    await inject();
    return {
      page, context, errors, inject,
      async close() {
        await context.close();
        await new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done()));
      },
    };
  } catch (error) {
    await context.close();
    await new Promise<void>(done => server.close(() => done()));
    throw error;
  }
}

export async function assertNativeRowsUntouched(page: any) {
  const audit = await page.evaluate(() => (window as any).fixtureAudit());
  assert.equal(audit.sameNodes, true, "native model row identities must survive");
  assert.equal(audit.sameParents, true, "native model rows must keep their original parents");
  assert.deepEqual(audit.moved, [], "CSS ordering must never detach or reparent a native row");
  return audit;
}

async function switchToList(page: any) {
  const toggle = page.getByRole("button", { name: VIEW_LABELS.list, exact: true });
  assert.equal(await toggle.count(), 1, "one icon button must switch to the list view");
  await toggle.click();
  await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).waitFor({ state: "visible" });
}

async function assertListIndicatorLayout(page: any, selector: string, expectDefault: boolean) {
  const header = page.locator(selector);
  const boxes = await header.evaluate((node: HTMLElement) => {
    const rect = (element: Element) => {
      const box = element.getBoundingClientRect();
      return { left: box.left, right: box.right, top: box.top, bottom: box.bottom };
    };
    const badge = node.querySelector(".codex-plus-model-group-default");
    return {
      header: rect(node),
      chevron: rect(node.querySelector(".codex-plus-model-group-chevron")!),
      title: rect(node.querySelector(".codex-plus-model-group-title")!),
      badge: badge ? rect(badge) : null,
      count: rect(node.querySelector(".codex-plus-model-group-count")!),
    };
  });
  assert.equal(!!boxes.badge, expectDefault, "default badge must match the selected group");
  const session = await page.context().newCDPSession(page);
  let dot: { left: number; right: number; top: number; bottom: number };
  try {
    const { root } = await session.send("DOM.getDocument", { depth: 0 });
    const { nodeId } = await session.send("DOM.querySelector", { nodeId: root.nodeId, selector });
    const { node } = await session.send("DOM.describeNode", { nodeId, depth: 1 });
    const after = node.pseudoElements?.find((item: any) => item.pseudoType === "after");
    assert.ok(after, "current group must have a rendered ::after indicator");
    const { model } = await session.send("DOM.getBoxModel", { backendNodeId: after.backendNodeId });
    const quad = model.border as number[];
    dot = { left: Math.min(quad[0], quad[2], quad[4], quad[6]),
      right: Math.max(quad[0], quad[2], quad[4], quad[6]),
      top: Math.min(quad[1], quad[3], quad[5], quad[7]),
      bottom: Math.max(quad[1], quad[3], quad[5], quad[7]) };
  } finally {
    await session.detach();
  }
  const items = [boxes.chevron, boxes.title, dot!, ...(boxes.badge ? [boxes.badge] : []), boxes.count];
  const gap = dot!.left - boxes.title.right;
  assert.ok(dot!.right > dot!.left && dot!.bottom > dot!.top, "indicator must have a visible box");
  assert.ok(gap >= 0 && gap <= 8, `indicator must sit immediately after the group title, gap=${gap}`);
  for (const [index, item] of items.entries()) {
    assert.ok(item.left >= boxes.header.left - 0.5 && item.right <= boxes.header.right + 0.5,
      `group item ${index} must fit within the header`);
    assert.ok(item.top >= boxes.header.top - 0.5 && item.bottom <= boxes.header.bottom + 0.5,
      `group item ${index} must fit vertically within the header`);
    if (index) assert.ok(item.left >= items[index - 1].right - 0.5, `group items ${index - 1} and ${index} must not overlap`);
  }
}

export async function pickerIconSnapshot(page: any) {
  return await page.evaluate(() => {
    const list = document.querySelector(".ModelList-fixture")!;
    const grid = list.querySelector(".codex-plus-model-provider-grid")!;
    const gridRect = grid.getBoundingClientRect();
    const visible = (node: Element) => node.getClientRects().length > 0 && getComputedStyle(node).visibility !== "hidden";
    const geometry = (node: Element, anchor: Element) => {
      const rect = node.getBoundingClientRect(), base = anchor.getBoundingClientRect();
      return [rect.x - base.x, rect.y - base.y, rect.width, rect.height].map(value => Math.round(value * 10) / 10);
    };
    return {
      grid: { visible: visible(grid), display: getComputedStyle(grid).display,
        columns: getComputedStyle(grid).gridTemplateColumns, width: Math.round(gridRect.width) },
      providers: [...grid.children].map(header => ({
        name: header.getAttribute("aria-label"), text: (header as HTMLElement).innerText,
        title: header.getAttribute("title"), geometry: geometry(header, grid),
        icon: header.querySelector(".codex-plus-model-provider-icon")?.getAttribute("style"),
      })),
      toolbar: [...list.querySelectorAll(".codex-plus-model-group-toolbar")].filter(visible)
        .map(node => ({ text: (node as HTMLElement).innerText, label: node.querySelector("button")?.getAttribute("aria-label") })),
      native: [...list.querySelectorAll('[role="menuitemradio"]')].map(row => ({
        slug: (row as HTMLElement).dataset.fixtureModel || "default", visible: visible(row),
        text: visible(row) ? (row as HTMLElement).innerText : "",
        checked: row.getAttribute("aria-checked"), padding: getComputedStyle(row).padding,
      })),
    };
  });
}

export const pickerViewScenarios: Array<{
  name: string; baseline?: boolean; run: (fixture: any, browser: any) => Promise<void>;
}> = [
  {
    name: "interleaved model groups and suppliers remain single groups in both views",
    async run({ page, inject }) {
      await page.evaluate(() => {
        const w = window as any;
        w.__codexPlusModelPickerGroupsRuntime.releaseDom();
        const additions = [
          { slug: "group/2", label: "模型组 2", full: "模型组 2", provider: "模型组", group: "codex-plus-gateway-groups" },
          { slug: "hanhe/extra-1", label: "Extra 1（寒鹤）", full: "Extra 1（寒鹤）", provider: "寒鹤", group: "provider-hanhe" },
          { slug: "group/3", label: "模型组 3", full: "模型组 3", provider: "模型组", group: "codex-plus-gateway-groups" },
          { slug: "hanhe/extra-2", label: "Extra 2（寒鹤）", full: "Extra 2（寒鹤）", provider: "寒鹤", group: "provider-hanhe" },
          { slug: "group/4", label: "模型组 4", full: "模型组 4", provider: "模型组", group: "codex-plus-gateway-groups" },
          { slug: "group/5", label: "模型组 5", full: "模型组 5", provider: "模型组", group: "codex-plus-gateway-groups" },
          { slug: "auto/2", label: "自动 2", full: "自动 2", provider: "自动路由", group: "codex-plus-gateway-auto" },
          { slug: "auto/3", label: "自动 3", full: "自动 3", provider: "自动路由", group: "codex-plus-gateway-auto" },
          { slug: "auto/4", label: "自动 4", full: "自动 4", provider: "自动路由", group: "codex-plus-gateway-auto" },
          { slug: "auto/5", label: "自动 5", full: "自动 5", provider: "自动路由", group: "codex-plus-gateway-auto" },
        ];
        for (const item of additions) {
          w.fixtureModels.push(item);
          w.codexModelCatalog.models.push(item.slug);
          w.codexModelCatalog.modelMetadata[item.slug] = {
            displayName: item.full, codex_plus_group: item.provider, codex_plus_group_id: item.group,
          };
        }
        const ids = ["codex-plus-gateway-groups", "codex-plus-gateway-auto", "provider-hanhe", "provider-fusheng"];
        const byGroup = ids.map(id => w.fixtureModels.filter((item: any) => item.group === id));
        w.fixtureModels = Array.from({ length: 5 }, (_, index) => byGroup.map(items => items[index])).flat();
        w.fixtureMountMenu();
      });
      await inject();
      for (const [id, count] of [["codex-plus-gateway-groups", 5], ["codex-plus-gateway-auto", 5], ["provider-hanhe", 5], ["provider-fusheng", 5]] as const) {
        const group = page.locator(`[data-codex-plus-model-group-id="${id}"]`);
        assert.equal(await group.count(), 1);
        assert.match(await group.innerText(), new RegExp(`${count} 个模型`));
      }
      await switchToList(page);
      await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).click();
      for (const [id, count] of [["codex-plus-gateway-groups", 5], ["codex-plus-gateway-auto", 5], ["provider-hanhe", 5], ["provider-fusheng", 5]] as const) {
        assert.equal(await page.locator(`[data-codex-plus-model-group-id="${id}"]`).count(), 1);
        assert.equal(await page.locator(`[data-codex-plus-model-group="${id}"]:visible`).count(), count);
      }
      const layout = await page.evaluate(() => {
        const headers = [...document.querySelectorAll<HTMLElement>("[data-codex-plus-model-group-id]")]
          .map(header => ({
            id: header.getAttribute("data-codex-plus-model-group-id"),
            top: header.getBoundingClientRect().top,
            bottom: header.getBoundingClientRect().bottom,
          })).sort((a, b) => a.top - b.top);
        const rows = [...document.querySelectorAll<HTMLElement>("[data-codex-plus-model-group]")]
          .map(row => ({
            id: row.getAttribute("data-codex-plus-model-group"),
            top: row.getBoundingClientRect().top,
            bottom: row.getBoundingClientRect().bottom,
          }));
        return { headers, rows };
      });
      assert.equal(layout.headers.length, 4);
      for (const [index, header] of layout.headers.entries()) {
        const rows = layout.rows.filter((row: any) => row.id === header.id);
        assert.equal(rows.length, 5, `row count for ${header.id}`);
        const nextTop = layout.headers[index + 1]?.top ?? Infinity;
        for (const row of rows) {
          assert.ok(row.top >= header.bottom && row.bottom <= nextTop,
            `${header.id} row must appear between its own header and the next group`);
        }
      }
      await page.locator('[data-codex-plus-model-group-id="provider-hanhe"]').click();
      assert.equal(await page.locator('[data-codex-plus-model-group="provider-hanhe"]:visible').count(), 0);
      assert.equal(await page.locator('[data-codex-plus-model-group="provider-fusheng"]:visible').count(), 5);
      const collapsedLayout = await page.evaluate(() => ({
        hanheBottom: document.querySelector<HTMLElement>('[data-codex-plus-model-group-id="provider-hanhe"]')!
          .getBoundingClientRect().bottom,
        fushengTop: document.querySelector<HTMLElement>('[data-codex-plus-model-group-id="provider-fusheng"]')!
          .getBoundingClientRect().top,
        fushengBottom: document.querySelector<HTMLElement>('[data-codex-plus-model-group-id="provider-fusheng"]')!
          .getBoundingClientRect().bottom,
        rowTops: [...document.querySelectorAll<HTMLElement>('[data-codex-plus-model-group="provider-fusheng"]')]
          .map(row => row.getBoundingClientRect().top),
      }));
      assert.ok(collapsedLayout.fushengTop >= collapsedLayout.hanheBottom);
      assert.ok(collapsedLayout.rowTops.every((top: number) => top >= collapsedLayout.fushengBottom),
        "Fusheng rows must remain below their own header when Hanhe is collapsed");
      await assertNativeRowsUntouched(page);
    },
  },
  {
    name: "main icon navigation retains native row identities, callbacks and Escape behavior",
    baseline: true,
    async run({ page }) {
      assert.equal(await page.locator(".codex-plus-model-provider-grid").isVisible(), true);
      await assertNativeRowsUntouched(page);
      const provider = page.locator('[data-codex-plus-model-group-id="provider-fusheng"]');
      await provider.click();
      assert.equal(await page.locator('[data-fixture-model^="fusheng/"]:visible').count(), 5);
      assert.equal(await page.locator('[data-fixture-model^="hanhe/"]:visible').count(), 0);
      await page.locator('[data-fixture-model="fusheng/gpt-6.1-terra"]').click();
      const audit = await assertNativeRowsUntouched(page);
      assert.deepEqual(audit.clicks, ["fusheng/gpt-6.1-terra"]);
      assert.equal(audit.model, "fusheng/gpt-6.1-terra");
      assert.equal(audit.effort, "xhigh");
      await page.locator('[data-fixture-model="fusheng/gpt-6.1-terra"]').focus();
      await page.keyboard.press("Escape");
      assert.equal(await page.locator(".codex-plus-model-provider-grid").isVisible(), true);
      assert.equal(await provider.evaluate((node: Element) => node === document.activeElement), true);
    },
  },
  {
    name: "the DOM fixture never fetches app credentials or sends a model request",
    baseline: true,
    async run({ page }) {
      const resources = await page.evaluate(() => performance.getEntriesByType("resource").map((entry: any) => entry.name));
      assert.deepEqual(resources, []);
      const audit = await assertNativeRowsUntouched(page);
      assert.deepEqual(audit.clicks, []);
      assert.equal(audit.model, "fusheng/gpt-6.1-sol");
    },
  },
  {
    name: "list mode reuses the native title with three compact icon controls on the same line",
    async run({ page }) {
      await switchToList(page);
      assert.equal(await page.getByText("选择模型", { exact: true }).count(), 1);
      const title = page.locator("#native-model-title");
      assert.equal(await title.isVisible(), true, "the original title must be reused");
      const titleBox = await title.boundingBox();
      let previousRight = titleBox.x + titleBox.width;
      for (const name of [VIEW_LABELS.expand, VIEW_LABELS.collapse, VIEW_LABELS.icons]) {
        const button = page.getByRole("button", { name, exact: true });
        assert.equal(await button.count(), 1);
        const box = await button.boundingBox();
        assert.ok(box && box.width >= 24 && box.width <= 36 && box.height >= 24 && box.height <= 36);
        assert.ok(Math.abs(box.y + box.height / 2 - titleBox.y - titleBox.height / 2) <= 3, name);
        assert.ok(box.x >= previousRight - 1, `${name} must fit to the right without overlap`);
        assert.equal((await button.innerText()).trim(), "", "commands must be icons with accessible names");
        assert.ok(await button.locator("svg, [aria-hidden='true']").count(), "the icon must be present");
        assert.ok(await button.getAttribute("title"), "the icon must have a hover tooltip");
        previousRight = box.x + box.width;
      }
      await assertNativeRowsUntouched(page);
    },
  },
  {
    name: "list mode shows the real default name and preserves the original default action",
    async run({ page }) {
      await switchToList(page);
      const row = page.locator("#native-default-row");
      const fullName = await page.evaluate(() => {
        const catalog = (window as any).codexModelCatalog;
        return catalog.modelMetadata[catalog.effective_config_model].displayName;
      });
      assert.equal(await row.getByText("默认", { exact: true }).isVisible(), true);
      assert.equal(await row.getByText(fullName, { exact: true }).isVisible(), true);
      assert.equal(await row.getByText("推荐模型集", { exact: true }).isVisible(), false);
      await row.click();
      const audit = await assertNativeRowsUntouched(page);
      assert.deepEqual(audit.clicks, ["default"]);
      assert.equal(audit.model, "hanhe/gpt-6.1-terra");
      assert.equal(audit.effort, "ultra");
    },
  },
  {
    name: "native div-based subtitle rows hide the recommendation and restore their original structure",
    async run({ page, inject }) {
      await page.locator("#native-default-row").evaluate((row: HTMLElement) => {
        row.innerHTML = '<div data-menu-row-content><div class="flex min-w-0 flex-1 flex-col" style="display:flex;flex-direction:column"><span class="min-w-0 truncate">默认</span><span class="min-w-0 text-xs native-recommendation" aria-hidden="false"><span class="_DefaultDescription_native">推荐模型集</span></span></div><svg class="native-check" viewBox="0 0 16 16" aria-hidden="true"><path d="m3 8 3 3 7-7"/></svg></div>';
      });
      const original = await page.locator("#native-default-row").innerHTML();
      await inject();
      await switchToList(page);
      const row = page.locator("#native-default-row");
      assert.equal(await row.getByText("推荐模型集", { exact: true }).isVisible(), false);
      assert.equal(await row.locator(".codex-plus-model-default-name").innerText(), "GPT-6.1-Terra（寒鹤）");
      assert.equal(await row.locator("svg.native-check").count(), 1);
      assert.equal(await row.locator("[data-codex-plus-model-default-content]").evaluate((node: Element) =>
        getComputedStyle(node).flexDirection), "row");
      await row.click();
      assert.deepEqual((await assertNativeRowsUntouched(page)).clicks, ["default"]);
      await page.getByRole("button", { name: VIEW_LABELS.icons, exact: true }).click();
      assert.equal(await row.innerHTML(), original);
      assert.equal(await row.getByText("推荐模型集", { exact: true }).isVisible(), true);
      await switchToList(page);
      await inject();
      await page.evaluate(() => {
        const runtime = (window as any).__codexPlusModelPickerGroupsRuntime;
        runtime.dispose(); runtime.releaseDom();
      });
      assert.equal(await row.innerHTML(), original);
    },
  },
  {
    name: "interleaved native rows form continuous indented groups using consecutive CSS order",
    async run({ page }) {
      await switchToList(page);
      await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).click();
      const layout = await page.evaluate(() => {
        const headers = [...document.querySelectorAll<HTMLElement>("[data-codex-plus-model-group-id]")];
        const rows = [...document.querySelectorAll<HTMLElement>("[data-fixture-model]")];
        const nodes = [...headers, ...rows].filter(node => node.getClientRects().length);
        const items = nodes.sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top).map(node => ({
          kind: node.dataset.fixtureModel ? "row" : "header",
          id: node.getAttribute("data-codex-plus-model-group-id") || node.getAttribute("data-codex-plus-model-group"),
          slug: node.dataset.fixtureModel || "", order: Number(getComputedStyle(node).order),
          x: node.getBoundingClientRect().left, width: node.getBoundingClientRect().width,
          padding: parseFloat(getComputedStyle(node).paddingLeft),
        }));
        return { items, expected: (window as any).fixtureModels.map((item: any) => ({ slug: item.slug, group: item.group })) };
      });
      assert.equal(layout.items.filter((item: any) => item.kind === "header").length, 4);
      let group = "", headerPadding = 0;
      const seen = new Set();
      for (const item of layout.items) {
        if (item.kind === "header") {
          assert.equal(seen.has(item.id), false, "a provider must have exactly one visual section");
          seen.add(item.id); group = item.id; headerPadding = item.padding;
        } else {
          assert.equal(item.id, group, `${item.slug} must stay below its own group header`);
          assert.equal(layout.expected.find((entry: any) => entry.slug === item.slug).group, group);
          assert.ok(item.padding >= headerPadding + 12, `${item.slug} needs visible indentation`);
        }
      }
      const orders = layout.items.map((item: any) => item.order);
      assert.deepEqual(orders, Array.from({ length: orders.length }, (_, index) => orders[0] + index),
        "headers and rows must share one consecutive order sequence");
      const audit = await assertNativeRowsUntouched(page);
      assert.deepEqual(audit.ids, layout.expected.map((entry: any) => entry.slug), "visual order must not rewrite DOM order");
    },
  },
  {
    name: "list current group indicator follows selection and stays visible when collapsed",
    async run({ page }) {
      await switchToList(page);
      const fusheng = page.locator('[data-codex-plus-model-group-id="provider-fusheng"]');
      const hanhe = page.locator('[data-codex-plus-model-group-id="provider-hanhe"]');
      await page.waitForFunction(() => document.querySelector('[data-codex-plus-model-group-id="provider-fusheng"]')?.getAttribute("data-current") === "true");
      assert.equal(await fusheng.getAttribute("aria-label"), "浮生，5 个模型，当前使用");
      assert.equal(await hanhe.getAttribute("aria-label"), "寒鹤（默认供应商），3 个模型");
      await assertListIndicatorLayout(page, '[data-codex-plus-model-group-id="provider-fusheng"]', false);
      await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).click();
      await page.locator('[data-fixture-model="hanhe/gpt-6-sol"]').click();
      await page.waitForFunction(() => document.querySelector('[data-codex-plus-model-group-id="provider-hanhe"]')?.getAttribute("data-current") === "true");
      assert.equal(await fusheng.getAttribute("data-current"), "false");
      assert.equal(await fusheng.getAttribute("aria-label"), "浮生，5 个模型");
      assert.equal(await hanhe.getAttribute("aria-label"), "寒鹤（默认供应商），3 个模型，当前使用");
      await assertListIndicatorLayout(page, '[data-codex-plus-model-group-id="provider-hanhe"]', true);
      await page.getByRole("button", { name: VIEW_LABELS.collapse, exact: true }).click();
      assert.equal(await hanhe.evaluate((node: Element) => getComputedStyle(node, "::after").display), "block");
      await page.evaluate(() => {
        document.querySelectorAll("[data-fixture-model]").forEach(row => row.setAttribute("aria-checked", "false"));
        document.querySelector('[data-fixture-model="fusheng/gpt-6-sol"]')?.setAttribute("data-model-selected", "");
      });
      await page.waitForFunction(() => document.querySelector('[data-codex-plus-model-group-id="provider-fusheng"]')?.getAttribute("data-current") === "true");
      assert.equal(await hanhe.getAttribute("data-current"), "false");
      assert.equal(await fusheng.getAttribute("aria-label"), "浮生，5 个模型，当前使用");
      assert.equal(await hanhe.getAttribute("aria-label"), "寒鹤（默认供应商），3 个模型");
    },
  },
  {
    name: "long list group names keep the current indicator and badges clear at narrow widths",
    async run({ page }) {
      await page.evaluate(() => {
        const catalog = (window as any).codexModelCatalog;
        for (const [slug, entry] of Object.entries(catalog.modelMetadata) as Array<[string, any]>) {
          if (slug.startsWith("fusheng/")) entry.codex_plus_group = "非常长的非默认供应商名称用于测试截断显示";
          if (slug.startsWith("hanhe/")) entry.codex_plus_group = "非常长的默认供应商名称用于测试截断显示";
        }
        (window as any).__codexPlusModelPickerGroupsRuntime.scan();
      });
      await switchToList(page);
      const nonDefaultTitle = page.locator('[data-codex-plus-model-group-id="provider-fusheng"] .codex-plus-model-group-title');
      const defaultTitle = page.locator('[data-codex-plus-model-group-id="provider-hanhe"] .codex-plus-model-group-title');
      assert.equal(await nonDefaultTitle.textContent(), "非常长的非默认供应商名称用于测试截断显示");
      assert.equal(await defaultTitle.textContent(), "非常长的默认供应商名称用于测试截断显示");
      for (const width of [280, 320, 455]) {
        await page.setViewportSize({ width, height: 960 });
        if (width === 280) assert.equal(await nonDefaultTitle.evaluate((node: HTMLElement) => node.scrollWidth > node.clientWidth), true);
        await assertListIndicatorLayout(page, '[data-codex-plus-model-group-id="provider-fusheng"]', false);
      }
      await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).click();
      await page.locator('[data-fixture-model="hanhe/gpt-6-sol"]').click();
      await page.waitForFunction(() => document.querySelector('[data-codex-plus-model-group-id="provider-hanhe"]')?.getAttribute("data-current") === "true");
      for (const width of [280, 320, 455]) {
        await page.setViewportSize({ width, height: 960 });
        if (width === 280) assert.equal(await defaultTitle.evaluate((node: HTMLElement) => node.scrollWidth > node.clientWidth), true);
        await assertListIndicatorLayout(page, '[data-codex-plus-model-group-id="provider-hanhe"]', true);
      }
    },
  },
  {
    name: "reinjection while the model groups builtin is disabled restores both views",
    async run({ page, inject }) {
      for (const mode of ["icons", "list"]) {
        if (mode === "list") await switchToList(page);
        await page.evaluate(() => {
          (window as any).__codexPlusBuiltinInactive.add("model-picker-groups");
        });
        await inject(false);
        assert.equal(await page.locator("[data-fixture-model]:visible").count(), 10);
        assert.equal(await page.locator(".native-recommendation").isVisible(), true);
        assert.equal(await page.getByRole("button", { name: VIEW_LABELS.list, exact: true }).count(), 0);
        assert.equal(await page.getByRole("button", { name: VIEW_LABELS.icons, exact: true }).count(), 0);
        await assertNativeRowsUntouched(page);
        await page.evaluate(() => {
          (window as any).__codexPlusBuiltinInactive.delete("model-picker-groups");
          (window as any).__codexPlusModelPickerGroupsRuntime.sync();
        });
        const expected = mode === "list" ? VIEW_LABELS.icons : VIEW_LABELS.list;
        await page.getByRole("button", { name: expected, exact: true }).waitFor({ state: "visible" });
      }
    },
  },
  {
    name: "the model groups builtin disables and restores both views",
    async run({ page }) {
      for (const mode of ["icons", "list"]) {
        if (mode === "list") await switchToList(page);
        await page.evaluate(() => {
          (window as any).__codexPlusBuiltinInactive.add("model-picker-groups");
          (window as any).__codexPlusModelPickerGroupsRuntime.sync();
        });
        await page.waitForFunction(() => !document.querySelector('.ModelList-fixture [data-codex-plus-ext="builtin-model-picker-groups"]'));
        assert.equal(await page.locator("[data-fixture-model]:visible").count(), 10);
        assert.equal(await page.locator(".native-recommendation").isVisible(), true);
        assert.equal(await page.getByRole("button", { name: VIEW_LABELS.list, exact: true }).count(), 0);
        assert.equal(await page.getByRole("button", { name: VIEW_LABELS.icons, exact: true }).count(), 0);
        await page.evaluate(() => {
          (window as any).__codexPlusBuiltinInactive.delete("model-picker-groups");
          (window as any).__codexPlusModelPickerGroupsRuntime.sync();
        });
        const expected = mode === "list" ? VIEW_LABELS.icons : VIEW_LABELS.list;
        await page.getByRole("button", { name: expected, exact: true }).waitFor({ state: "visible" });
        await assertNativeRowsUntouched(page);
      }
    },
  },
  {
    name: "list group toggles, expand all and collapse all do not change a selected model",
    async run({ page }) {
      await switchToList(page);
      await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).click();
      assert.equal(await page.locator("[data-fixture-model]:visible").count(), 10);
      const header = page.locator('[data-codex-plus-model-group-id="provider-fusheng"]');
      assert.equal(await header.getAttribute("aria-expanded"), "true");
      await header.click();
      assert.equal(await header.getAttribute("aria-expanded"), "false");
      assert.equal(await page.locator('[data-fixture-model^="fusheng/"]:visible').count(), 0);
      assert.equal(await page.locator('[data-fixture-model^="hanhe/"]:visible').count(), 3);
      await page.getByRole("button", { name: VIEW_LABELS.collapse, exact: true }).click();
      assert.equal(await page.locator("[data-fixture-model]:visible").count(), 0);
      assert.equal(await page.locator("[data-codex-plus-model-group-id]:visible").count(), 4);
      assert.equal(await page.locator("#native-default-row").isVisible(), true);
      await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).click();
      assert.equal(await page.locator("[data-fixture-model]:visible").count(), 10);
      const audit = await assertNativeRowsUntouched(page);
      assert.equal(audit.model, "fusheng/gpt-6.1-sol");
      assert.equal(audit.effort, "xhigh");
      assert.deepEqual(audit.clicks, []);
    },
  },
  {
    name: "mode is stored under a separate key and survives menu remount and reinjection",
    async run({ page, inject }) {
      await switchToList(page);
      assert.equal(await page.evaluate((key: string) => localStorage.getItem(key), VIEW_MODE_KEY), "list");
      await page.evaluate(() => (window as any).fixtureMountMenu());
      await page.evaluate(() => (window as any).__codexPlusModelPickerGroupsRuntime.scan());
      assert.equal(await page.getByRole("button", { name: VIEW_LABELS.icons, exact: true }).isVisible(), true);
      await inject();
      assert.equal(await page.getByRole("button", { name: VIEW_LABELS.icons, exact: true }).count(), 1);
      assert.equal(await page.getByText("选择模型", { exact: true }).count(), 1);
      await page.reload({ waitUntil: "domcontentloaded" });
      await inject();
      assert.equal(await page.getByRole("button", { name: VIEW_LABELS.icons, exact: true }).isVisible(), true);
      const oldKey = await page.evaluate(() => JSON.parse(localStorage.getItem("codexPlus.modelPicker.collapsedGroups.v1") || "{}"));
      for (const key of ["mode", "viewMode", "activeGroupId"]) assert.equal(Object.hasOwn(oldKey, key), false);
      await assertNativeRowsUntouched(page);
      await page.getByRole("button", { name: VIEW_LABELS.icons, exact: true }).click();
      assert.equal(await page.evaluate((key: string) => localStorage.getItem(key), VIEW_MODE_KEY), "icons");
      assert.equal(await page.locator(".codex-plus-model-provider-grid").isVisible(), true);
    },
  },
  {
    name: "React removal of headers or order styles self-heals without replacing native rows",
    async run({ page, inject }) {
      await switchToList(page);
      await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).click();
      await page.evaluate(() => {
        document.querySelectorAll("[data-codex-plus-model-group-id]").forEach(node => node.remove());
        document.querySelectorAll<HTMLElement>("[data-fixture-model]").forEach(row => {
          row.style.removeProperty("order"); row.style.removeProperty("--codex-plus-model-order");
        });
        document.querySelector(".ModelList-fixture")?.removeAttribute("data-codex-plus-model-ordering");
        (window as any).__codexPlusModelPickerGroupsRuntime.scan();
      });
      assert.equal(await page.locator("[data-codex-plus-model-group-id]:visible").count(), 4);
      const orders = await page.locator("[data-fixture-model]").evaluateAll((rows: HTMLElement[]) =>
        rows.map(row => getComputedStyle(row).order));
      assert.ok(orders.every((value: string) => value !== "0"), "missing ordering styles must be repaired even if row labels are unchanged");
      await inject();
      assert.equal(await page.locator("[data-codex-plus-model-group-id]:visible").count(), 4);
      await assertNativeRowsUntouched(page);
    },
  },
  {
    name: "releasing list patches restores labels, ordering, actions and the native title",
    async run({ page }) {
      await switchToList(page);
      await page.evaluate(() => {
        const runtime = (window as any).__codexPlusModelPickerGroupsRuntime;
        runtime.releaseDom(); runtime.dispose();
      });
      assert.equal(await page.getByText("选择模型", { exact: true }).count(), 1);
      assert.equal(await page.locator(".native-recommendation").isVisible(), true);
      assert.equal(await page.locator("[data-fixture-model]:visible").count(), 10);
      assert.equal(await page.locator('[data-codex-plus-ext="builtin-model-picker-groups"]').count(), 0);
      const restored = await page.evaluate(() => (window as any).fixtureRememberedRows.every((row: HTMLElement, index: number) => {
        const original = (window as any).fixtureOriginalAttributes[index];
        return row.className === original.className && (row.getAttribute("style") || "") === (original.style || "");
      }));
      assert.equal(restored, true);
      await page.locator('[data-fixture-model="hanhe/gpt-6-sol"]').click();
      const audit = await assertNativeRowsUntouched(page);
      assert.deepEqual(audit.clicks, ["hanhe/gpt-6-sol"]);
    },
  },
  {
    name: "switching list back to icons exactly restores the main icon layout and default text",
    async run({ page }) {
      const original = await pickerIconSnapshot(page);
      await switchToList(page);
      await page.getByRole("button", { name: VIEW_LABELS.collapse, exact: true }).click();
      await page.getByRole("button", { name: VIEW_LABELS.icons, exact: true }).click();
      assert.deepEqual(await pickerIconSnapshot(page), original);
      assert.equal(await page.locator(".native-recommendation").isVisible(), true);
      assert.equal(await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).count(), 0);
      assert.equal(await page.getByRole("button", { name: VIEW_LABELS.collapse, exact: true }).count(), 0);
      await assertNativeRowsUntouched(page);
    },
  },
  {
    name: "the effective config identity wins, stays fresh and never falls back to a guessed default",
    async run({ page }) {
      await switchToList(page);
      const name = page.locator("#native-default-row .codex-plus-model-default-name");
      for (const [model, expected] of [
        ["fusheng/gpt-6.1-sol", "GPT-6.1-Sol（浮生）"],
        ["group/1", "模型组 1"],
        ["auto/balanced", "自动均衡"],
        ["missing/model", "missing/model"],
        ["", "未获取到默认模型"],
      ]) {
        await page.evaluate((value: string) => {
          (window as any).codexModelCatalog.effective_config_model = value;
          (window as any).__codexPlusModelPickerGroupsRuntime.scan();
        }, model);
        assert.equal(await name.innerText(), expected);
        assert.equal(await name.getAttribute("title"), expected);
      }
      await page.evaluate(() => {
        delete (window as any).codexModelCatalog.effective_config_model;
        (window as any).__codexPlusModelPickerGroupsRuntime.scan();
      });
      assert.equal(await name.innerText(), "未获取到默认模型", "legacy catalog/default provider cannot identify the live default");
      const audit = await assertNativeRowsUntouched(page);
      assert.equal(audit.model, "fusheng/gpt-6.1-sol");
      assert.equal(audit.effort, "xhigh");
      assert.deepEqual(audit.clicks, []);
    },
  },
  {
    name: "a single provider retains its native icon-mode list and can switch both ways",
    async run({ page, inject }) {
      await page.evaluate(() => {
        (window as any).__codexPlusModelPickerGroupsRuntime.releaseDom();
        (window as any).fixtureModels = (window as any).fixtureModels.filter((item: any) => item.provider === "浮生");
        (window as any).fixtureMountMenu();
      });
      await inject();
      assert.equal(await page.locator(".codex-plus-model-provider-grid").count(), 0);
      assert.equal(await page.locator("[data-fixture-model]:visible").count(), 5);
      const native = await page.locator("[data-fixture-model]").allTextContents();
      await switchToList(page);
      assert.equal(await page.locator("[data-codex-plus-model-group-id]:visible").count(), 1);
      await page.getByRole("button", { name: VIEW_LABELS.collapse, exact: true }).click();
      assert.equal(await page.locator("[data-fixture-model]:visible").count(), 0);
      await page.getByRole("button", { name: VIEW_LABELS.icons, exact: true }).click();
      assert.equal(await page.locator(".codex-plus-model-provider-grid").count(), 0);
      assert.equal(await page.locator("[data-fixture-model]:visible").count(), 5);
      assert.deepEqual(await page.locator("[data-fixture-model]").allTextContents(), native);
      assert.equal(await page.locator(".native-recommendation").isVisible(), true);
      await assertNativeRowsUntouched(page);
    },
  },
  {
    name: "list keyboard focus follows visual groups and leaves Escape to the native menu",
    async run({ page }) {
      await switchToList(page);
      const first = page.locator('[data-fixture-model="fusheng/gpt-6.1-sol"]');
      await first.focus();
      await page.keyboard.press("ArrowDown");
      assert.equal(await page.locator('[data-fixture-model="fusheng/gpt-6.1-terra"]').evaluate(
        (node: Element) => document.activeElement === node), true);
      const header = page.locator('[data-codex-plus-model-group-id="provider-fusheng"]');
      await header.focus();
      await page.keyboard.press("ArrowLeft");
      assert.equal(await header.getAttribute("aria-expanded"), "false");
      await page.keyboard.press("ArrowRight");
      assert.equal(await header.getAttribute("aria-expanded"), "true");
      await page.keyboard.press("Enter");
      assert.equal(await header.getAttribute("aria-expanded"), "false");
      await page.evaluate(() => {
        (window as any).fixtureEscape = 0;
        document.addEventListener("keydown", event => {
          if (event.key === "Escape") (window as any).fixtureEscape++;
        }, true);
      });
      await page.keyboard.press("Escape");
      assert.equal(await page.evaluate(() => (window as any).fixtureEscape), 1);
      const audit = await assertNativeRowsUntouched(page);
      assert.deepEqual(audit.clicks, []);
    },
  },
  {
    name: "order loss and title-control removal heal through the scheduled scan",
    async run({ page }) {
      await switchToList(page);
      await page.evaluate(() => (window as any).__codexPlusModelPickerGroupsRuntime.sync());
      const before = await page.locator("[data-fixture-model]").evaluateAll((rows: HTMLElement[]) =>
        rows.map(row => getComputedStyle(row).order));
      await page.evaluate(() => {
        document.querySelectorAll<HTMLElement>("[data-fixture-model]").forEach(row => {
          row.style.removeProperty("--codex-plus-model-order");
        });
      });
      await page.waitForFunction((expected: string[]) => {
        const current = [...document.querySelectorAll<HTMLElement>("[data-fixture-model]")].map(row => getComputedStyle(row).order);
        return JSON.stringify(current) === JSON.stringify(expected);
      }, before);
      await page.evaluate(() => {
        document.querySelector(".codex-plus-model-view-controls")?.remove();
      });
      await page.getByRole("button", { name: VIEW_LABELS.expand, exact: true }).waitFor({ state: "visible" });
      assert.equal(await page.getByText("选择模型", { exact: true }).count(), 1);
      await assertNativeRowsUntouched(page);
    },
  },
  {
    name: "reinjection and disposal restore original default accessibility and unrelated native styles",
    async run({ page, inject }) {
      await page.evaluate(() => {
        const row = document.querySelector<HTMLElement>('[data-fixture-model="fusheng/gpt-6.1-sol"]')!;
        row.style.color = "rgb(11, 22, 33)";
        document.querySelector(".native-recommendation")!.setAttribute("aria-hidden", "false");
      });
      await switchToList(page);
      await page.evaluate(() => {
        document.querySelector<HTMLElement>('[data-fixture-model="fusheng/gpt-6.1-sol"]')!.style.fontWeight = "600";
      });
      await inject();
      await page.evaluate(() => {
        const runtime = (window as any).__codexPlusModelPickerGroupsRuntime;
        runtime.dispose(); runtime.releaseDom();
      });
      assert.equal(await page.locator(".native-recommendation").getAttribute("aria-hidden"), "false");
      const restored = await page.locator('[data-fixture-model="fusheng/gpt-6.1-sol"]').evaluate((row: HTMLElement) => ({
        color: row.style.color, weight: row.style.fontWeight, order: row.style.getPropertyValue("--codex-plus-model-order"),
      }));
      assert.deepEqual(restored, { color: "rgb(11, 22, 33)", weight: "600", order: "" });
      assert.equal(await page.locator('[data-codex-plus-ext="builtin-model-picker-groups"]').count(), 0);
      await assertNativeRowsUntouched(page);
    },
  },
  {
    name: "narrow list layouts keep title and icon controls inside the menu without overlap",
    async run({ page }) {
      await switchToList(page);
      for (const width of [280, 320, 455, 900]) {
        await page.setViewportSize({ width, height: 960 });
        const boxes = await page.evaluate((names: string[]) => {
          const menu = document.querySelector('[data-model-picker-view]')!.getBoundingClientRect();
          const title = document.querySelector("#native-model-title")!.getBoundingClientRect();
          const buttons = names.map(name => [...document.querySelectorAll("button")]
            .find(button => button.getAttribute("aria-label") === name)!.getBoundingClientRect());
          return { left: menu.left, right: menu.right, scroll: document.documentElement.scrollWidth,
            items: [title, ...buttons].map(rect => ({ left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom })) };
        }, [VIEW_LABELS.expand, VIEW_LABELS.collapse, VIEW_LABELS.icons]);
        assert.ok(boxes.scroll <= width, `no page overflow at ${width}px`);
        for (const [index, item] of boxes.items.entries()) {
          assert.ok(item.left >= boxes.left && item.right <= boxes.right, `title control fits at ${width}px`);
          if (index) assert.ok(item.left >= boxes.items[index - 1].right - 1, `title controls do not overlap at ${width}px`);
        }
      }
    },
  },
];

const isTestEntry = process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url;
if (isTestEntry) {
  describe("model picker views in a real browser DOM", {
    timeout: 120000,
    skip: process.env.CODEX_PLUS_MODEL_PICKER_BROWSER_TESTS === "1"
      ? false : "Set CODEX_PLUS_MODEL_PICKER_BROWSER_TESTS=1 to run browser tests.",
  }, () => {
    let browser: any;
    before(async () => { browser = await launchPickerBrowser(); });
    after(async () => { await browser?.close(); });
    for (const scenario of pickerViewScenarios) {
      it(scenario.name, async () => {
        const fixture = await createPickerFixture(browser);
        try {
          await scenario.run(fixture, browser);
          assert.deepEqual(fixture.errors, [], "the real shard must not throw in the browser");
        } finally { await fixture.close(); }
      });
    }
  });
}
