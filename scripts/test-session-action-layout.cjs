// Optional real-browser regression; use existing Playwright via NODE_PATH.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
const root = path.resolve(__dirname, '..');
const output = process.env.CODEX_OVERLAY_TEST_OUTPUT;
const candidate = fs.readFileSync(path.join(root, 'assets/inject/renderer-inject.js'), 'utf8');
const extract = (source, name) => {
  const match = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, 'm'));
  assert.ok(match, name);
  return match[0];
};
// Frozen upstream per-row layout; shallow PR checkouts need no private Git history.
function syncActionGroupLayout(row, group) {
  if (!row || !group) return;
  if (group.dataset.codexActionLayoutStable === "true") return;
  const rowRect = row.getBoundingClientRect();
  const nativeButtons = nativeActionButtonsFromRow(row);
  const leftmostNative = nativeButtons
    .map((button) => button.getBoundingClientRect())
    .filter((rect) => rect.width > 0 && rect.height > 0)
    .sort((a, b) => a.left - b.left)[0];
  const gap = 8;
  const fallbackRight = 28;
  const right = leftmostNative
    ? Math.max(fallbackRight, Math.round(rowRect.right - leftmostNative.left + gap))
    : fallbackRight;
  const groupWidth = Math.ceil(group.getBoundingClientRect().width || 96);
  const titleNode = row.querySelector(selectors.threadTitle);
  const titleRect = titleNode?.getBoundingClientRect();
  const titleLeft = titleRect?.left || rowRect.left + 40;
  let effectiveRight = right;
  group.style.setProperty("--codex-session-actions-right", `${effectiveRight}px`);
  if (leftmostNative) {
    const nativeStyle = getComputedStyle(nativeButtons.find((button) => button.getBoundingClientRect().left === leftmostNative.left) || nativeButtons[0]);
    group.style.setProperty("--codex-session-action-color", nativeStyle.color);
    group.style.setProperty("--codex-session-action-hover-color", nativeStyle.color);
    group.style.setProperty("--codex-session-action-hover-background", nativeStyle.backgroundColor);
    const groupRight = group.getBoundingClientRect().right;
    const targetRight = leftmostNative.left - 2;
    if (Number.isFinite(groupRight) && Number.isFinite(targetRight)) {
      const renderScale = row.offsetWidth > 0 ? rowRect.width / row.offsetWidth : 1;
      effectiveRight = Math.max(0, right + (groupRight - targetRight) / Math.max(0.1, renderScale));
      group.style.setProperty("--codex-session-actions-right", `${effectiveRight}px`);
    }
  }
  const renderScale = row.offsetWidth > 0 ? rowRect.width / row.offsetWidth : 1;
  const finalGroupLeft = group.getBoundingClientRect().left;
  const titleMaxWidth = Math.max(24, (finalGroupLeft - titleLeft - 8) / Math.max(0.1, renderScale));
  row.style.setProperty("--codex-session-title-mask", `${effectiveRight + groupWidth + 12}px`);
  row.style.setProperty("--codex-session-title-max-width", `${titleMaxWidth}px`);
  group.dataset.codexActionLayoutStable = "true";
}
function syncActionGroupsLayout() {
  sessionRows().forEach(row => syncActionGroupLayout(row, actionGroupFromRow(row)));
}
const serialize = fn => fn.toString().split(String.fromCharCode(10)).map(line => "  " + line).join(String.fromCharCode(10));
const baseline = [extract(candidate, "actionGroupFromRow"), extract(candidate, "nativeActionButtonsFromRow"),
  serialize(syncActionGroupLayout), serialize(syncActionGroupsLayout)].join(String.fromCharCode(10));
const install = source => `const actionGroupClass='codex-session-actions';
  const selectors={threadTitle:'.title'};
  const sessionRows=()=>Array.from(document.querySelectorAll('.row'));
  ${['actionGroupFromRow','nativeActionButtonsFromRow','syncActionGroupLayout','syncActionGroupsLayout'].map(n=>extract(source,n)).join('\n')}
  ${source.includes('function syncActionGroupLayouts(') ? extract(source,'syncActionGroupLayouts') : ''}
  window.layoutAll=()=>{
    ${source.includes('function syncActionGroupLayouts(') ? 'syncActionGroupsLayout(true);' : "sessionRows().forEach(row=>delete actionGroupFromRow(row).dataset.codexActionLayoutStable);syncActionGroupsLayout();"}
  };
  window.layoutStable=()=>syncActionGroupsLayout();
  window.layoutSingle=()=>syncActionGroupLayout(sessionRows()[0],actionGroupFromRow(sessionRows()[0]));`;
const html = (zoom, mode) => `<!doctype html><html><head><style>
  html,body{margin:0;background:#202020;color:#ddd;font:14px Arial}
  #sidebar{width:320px;zoom:${zoom};${mode==='transform'?'transform:scale(.9);transform-origin:top left;':''}}
  .row{position:relative;height:30px;border:1px solid #333;box-sizing:border-box;padding-left:40px}
  .title{display:inline-block;max-width:var(--codex-session-title-max-width,250px);white-space:nowrap;overflow:hidden}
  .native{position:absolute;right:8px;top:4px;width:20px;height:20px;color:rgb(160,175,190);background:rgb(36,48,44)}
  .codex-session-actions{position:absolute;right:var(--codex-session-actions-right,28px);top:50%;transform:translateY(-50%);display:inline-flex;gap:2px}
  .codex-session-actions button{width:20px;height:20px;border:0;padding:0}
  .clamp .codex-session-actions{right:max(66px,var(--codex-session-actions-right,28px))}
  .hidden{display:none}
  #body{position:absolute;left:420px;top:0;width:700px;display:flex;flex-wrap:wrap}
  #body span{display:inline-block;width:32px;height:22px;border:1px solid #333}
  </style></head><body><div id="sidebar" dir="${mode==='rtl'?'rtl':'ltr'}">
  ${Array.from({length:160},(_,i)=>`<div class="row ${mode==='clamp'?'clamp':''} ${i%19===0?'hidden':''}">
    <span class="title">Thread ${i} with a long title</span>
    ${i%7 ? '<button class="native" aria-label="Pin">P</button>' : ''}
    <div class="codex-session-actions" data-codex-action-layout-stable="true"><button>1</button><button>2</button></div>
    </div>`).join('')}</div>
  <div id="body">${'<span></span>'.repeat(3000)}</div></body></html>`;
async function run(page, source, zoom, mode) {
  await page.setContent(html(zoom,mode));
  await page.evaluate(install(source));
  await page.evaluate(()=>{
    let dirty=true,flushes=0,reads=0,writes=0;
    const rect=Element.prototype.getBoundingClientRect,set=CSSStyleDeclaration.prototype.setProperty;
    Element.prototype.getBoundingClientRect=function(...args){
      reads++;if(dirty){flushes++;dirty=false;}return rect.apply(this,args);
    };
    CSSStyleDeclaration.prototype.setProperty=function(...args){
      writes++;if(this.getPropertyValue(args[0])!==String(args[1]))dirty=true;
      return set.apply(this,args);
    };
    window.resetMetrics=()=>{dirty=true;flushes=reads=writes=0;};
    window.metrics=()=>({flushes,reads,writes});
  });
  const cdp=await page.context().newCDPSession(page);
  await cdp.send('Performance.enable');
  const before=await cdp.send('Performance.getMetrics');
  const results=[];
  for(const width of [320,280,360,320]){
    results.push(await page.evaluate(width=>{
      document.getElementById('sidebar').style.width=width+'px';
      resetMetrics();const start=performance.now();layoutAll();const ms=performance.now()-start;
      const metrics=window.metrics();
      const rows=Array.from(document.querySelectorAll('.row'),row=>{
        const group=row.querySelector('.codex-session-actions');
        const values=['--codex-session-title-mask','--codex-session-title-max-width'].map(k=>row.style.getPropertyValue(k));
        return [...values,...['--codex-session-actions-right','--codex-session-action-color','--codex-session-action-hover-color',
          '--codex-session-action-hover-background'].map(k=>group.style.getPropertyValue(k)),
          group.dataset.codexActionLayoutStable];
      });
      return {ms,metrics,rows};
    },width));
  }
  const after=await cdp.send('Performance.getMetrics');
  const metric=(values,name)=>values.metrics.find(m=>m.name===name)?.value || 0;
  const chromium={layouts:metric(after,'LayoutCount')-metric(before,'LayoutCount'),
    styles:metric(after,'RecalcStyleCount')-metric(before,'RecalcStyleCount')};
  await cdp.send('Performance.disable');await cdp.detach();
  const stable=await page.evaluate(()=>{resetMetrics();layoutStable();return metrics();});
  assert.equal(stable.reads,0,'stable layouts must not be remeasured');
  const single=await page.evaluate(()=>{
    const group=document.querySelector('.codex-session-actions');
    delete group.dataset.codexActionLayoutStable;layoutSingle();return group.dataset.codexActionLayoutStable;
  });
  assert.equal(single,'true','single-row callers still work');
  await page.evaluate(()=>{document.getElementById('sidebar').remove();layoutAll();});
  return {results,chromium};
}
async function main(){
  const browser=await chromium.launch({channel:process.env.CODEX_TEST_BROWSER || 'msedge',
    executablePath:process.env.CODEX_TEST_BROWSER_PATH,headless:true}),results=[];
  try{
    for(const [zoom,mode] of [[.8,'ltr'],[1,'ltr'],[1.25,'ltr'],[1,'rtl'],[1,'transform'],[1,'clamp']]){
      const page=await browser.newPage({viewport:{width:1280,height:720}});
      const errors=[];page.on('pageerror',e=>errors.push(e.message));
      const oldRun=await run(page,baseline,zoom,mode),nextRun=await run(page,candidate,zoom,mode);
      const old=oldRun.results,next=nextRun.results;
      for(let i=0;i<old.length;i++){
        assert.deepEqual(next[i].rows,old[i].rows,`original positioning/colors/title mask at ${zoom}/${mode}/${i}`);
        assert.ok(next[i].metrics.flushes<=3,`bounded read/write phases: ${JSON.stringify(next[i].metrics)}`);
        assert.ok(old[i].metrics.flushes>100,'baseline reproduces per-row thrashing');
      }
      assert.ok(nextRun.chromium.layouts<=16,`actual Chromium layouts stay bounded: ${JSON.stringify(nextRun.chromium)}`);
      assert.ok(oldRun.chromium.layouts>300,`baseline actual Chromium thrashing: ${JSON.stringify(oldRun.chromium)}`);
      assert.deepEqual(errors,[]);
      results.push({zoom,mode,old:old.map(({ms,metrics})=>({ms,metrics})),next:next.map(({ms,metrics})=>({ms,metrics})),
        chromium:{old:oldRun.chromium,next:nextRun.chromium}});
      await page.close();console.log(`PASS sidebar ${zoom}/${mode}`);
    }
    if(output)fs.writeFileSync(path.join(output,'sidebar-layout-verification.json'),JSON.stringify(results,null,2));
    console.log(JSON.stringify({cases:results.length,oldMaxMs:Math.max(...results.flatMap(r=>r.old.map(x=>x.ms))),
      nextMaxMs:Math.max(...results.flatMap(r=>r.next.map(x=>x.ms))),
      oldFlushes:results[0].old.map(x=>x.metrics.flushes),nextFlushes:results[0].next.map(x=>x.metrics.flushes),
      chromium:results.map(r=>({zoom:r.zoom,mode:r.mode,...r.chromium}))}));
  }finally{await browser.close();}
}
main().catch(e=>{console.error(e);process.exitCode=1;});
