// Optional browser regression: supply installed Playwright/sharp/pngjs via NODE_PATH.
const fs = require('node:fs');
const path = require('node:path');
const cp = require('node:child_process');
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
const sharp = require('sharp');
const { PNG } = require('pngjs');
const root = path.resolve(__dirname, '..');
const output = process.env.CODEX_OVERLAY_TEST_OUTPUT;
const ref = process.env.CODEX_OVERLAY_TEST_REF;
const nativeDirectory = process.env.CODEX_OVERLAY_NATIVE_CSS_DIR;
const nativeCSS = nativeDirectory ? fs.readdirSync(nativeDirectory).filter(name => /^app-(initial|primary|shared)-.*\.css$/.test(name))
  .map(name => fs.readFileSync(path.join(nativeDirectory,name),'utf8')).join('\n') : '';
const source = ref ? cp.execFileSync('git', ['show', `${ref}:assets/inject/renderer-inject.js`], {cwd:root,encoding:'utf8',maxBuffer:8e6})
  : fs.readFileSync(path.join(root, 'assets/inject/renderer-inject.js'), 'utf8');
const extract = name => {
  const match = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, 'm'));
  assert.ok(match, name); return match[0];
};
const install = `const codexPlusImageOverlayId='codex-plus-image-overlay';
  const sendCodexPlusDiagnostic=()=>{};
  ${extract('installCodexPlusImageOverlayForeground')}
  ${extract('installCodexPlusImageOverlay')}
  window.installOverlay=installCodexPlusImageOverlay; installOverlay();`;
async function bitmap(r,g,b,format='png',alpha=1) {
  return `data:image/${format};base64,` + (await sharp({create:{width:80,height:80,channels:4,background:{r,g,b,alpha}}})[format]().toBuffer()).toString('base64');
}
async function main() {
 const red=await bitmap(220,40,50), green=await bitmap(30,180,60,'gif'), wall=await bitmap(40,80,200),clear=await bitmap(0,0,0,'png',0);
 const browser=await chromium.launch({channel:'msedge',headless:true});
 const results=[];
 try {
  for(const width of (process.env.CODEX_OVERLAY_QUICK ? [1088] : [480,1088,1727])) for(const zoom of (process.env.CODEX_OVERLAY_QUICK ? [1] : [.8,1,1.25])) {
   const page=await browser.newPage({viewport:{width,height:900},deviceScaleFactor:1});
   const errors=[];page.on('pageerror',e=>errors.push(e.message));
   let maxWheelDrift=0,maxNestedDrift=0;
   await page.route('**/*',route=>route.request().url().startsWith('data:')?route.continue():route.abort());
   await page.setContent(`<!doctype html><html><head><style>${nativeCSS}</style><style>
    *{box-sizing:border-box}[hidden]:not([hidden="until-found"]){display:none!important}html{--color-surface:#181818}body{margin:0;background:#202020;color:#eee;font:14px Arial;overflow:hidden}
    #root{height:900px}#shell{zoom:${zoom};isolation:isolate;display:grid;grid-template-columns:52px 1fr;height:680px;overflow:hidden}
    nav{background:#303030}main{min-width:0;background:#222}header{height:45px;background:#333}
    #scroller{height:360px;overflow:auto;position:relative;border:0;padding:10px}#spacer{height:0}
    #row{display:flex;gap:8px;flex-wrap:wrap}#row img,#row video,#row canvas{width:70px;height:70px;object-fit:contain;border-radius:6px}
    #controlled{width:180px!important;height:110px!important}#clip{position:relative;overflow:hidden;width:50px;height:50px}#clip img{width:80px;height:80px}
    #footer{position:relative;height:190px;background:#333;padding:12px}textarea{background:#444;color:white;width:100%;height:80px}
    #side{position:relative;overflow:auto;height:120px;background:#292929}#side>div{height:190px;display:flex;align-items:center;justify-content:center}
    #side img{width:140px;height:160px;object-fit:contain}#tail{height:600px}
    #menu{position:fixed;left:50px;top:40px;width:260px;height:140px;background:#777;z-index:2147483647}
    .codex-session-more-menu{position:fixed;display:none}#root-modal{position:fixed;inset:0;background:#181818;z-index:80;display:grid;place-items:center}
    #root-modal img{width:120px;height:120px}#ordinary{height:60px;background:#363636}
   </style></head><body><div id="root"><div id="shell"><nav>Rail</nav><main><header>Header</header>
    <div id="scroller" class="thread-scroll-container"><div id="spacer"></div><div id="row">
     <img id="photo" src="${red}"><img id="gif" src="${green}"><img id="clear" src="${clear}">
     <video id="video" muted playsinline></video><canvas id="canvas" width="80" height="80"></canvas><div id="control-host"><video id="controlled" controls muted playsinline></video></div>
    </div><div id="clip"><img id="clipped" src="${red}"></div><div id="tail"></div></div>
    <div id="footer" data-thread-scroll-footer><textarea>Composer</textarea><div id="ordinary">Settings / Markdown / review</div></div>
    <div id="side"><div><img id="side-photo" src="${red}"></div></div>
   </main></div></div><div id="menus">${'<div class="codex-session-more-menu" role="menu" hidden></div>'.repeat(56)}</div>
   <div id="menu" role="menu" hidden></div></body></html>`);
   await page.evaluate(({wall})=>{
    window.__CODEX_PLUS_IMAGE_OVERLAY__={enabled:true,dataUrl:wall,opacity:.2,fitMode:'stretch'};
    window.feed=document.createElement('canvas');feed.width=80;feed.height=80;
    window.paint=(color)=>{feed.getContext('2d').fillStyle=color;feed.getContext('2d').fillRect(0,0,80,80)};
    paint('rgb(220,40,50)');window.stream=feed.captureStream(15);document.querySelector('#video').srcObject=stream;
    document.querySelector('#canvas').getContext('2d').fillStyle='rgb(220,40,50)';document.querySelector('#canvas').getContext('2d').fillRect(0,0,80,80);
    window.paintTimer=setInterval(()=>{feed.getContext('2d').fillRect(0,0,80,80)},70);
    window.controlledSource=document.querySelector('#controlled');controlledSource.srcObject=stream;
    return Promise.all([document.querySelector('#video').play(),controlledSource.play()]);
   },{wall});
   const geometryBefore=await page.locator('#controlled').boundingBox();
   const neighborBefore=await page.locator('#clip').boundingBox();
   await page.locator('textarea').focus();
   await page.evaluate(()=>{
    window.rafCalls=0;window.deferredOverlayFrames=[];const raf=window.requestAnimationFrame;
    window.requestAnimationFrame=cb=>{window.rafCalls++;return raf(time=>{
     if(window.deferOverlayFrames)window.deferredOverlayFrames.push(cb);else cb(time);
    })};
    window.resumeOverlayFrames=()=>{window.deferOverlayFrames=false;for(const cb of window.deferredOverlayFrames.splice(0))raf(cb)};
   });
   await page.evaluate(install);
   assert.equal(await page.locator('textarea').evaluate(e=>e===document.activeElement),true,'promotion does not steal focus');
   const screenshot=async()=>PNG.sync.read(await page.screenshot());
   const pixel=(image,x,y)=>Array.from(image.data.subarray((Math.floor(y)*image.width+Math.floor(x))*4,(Math.floor(y)*image.width+Math.floor(x))*4+3));
   const at=async(selector,fx=.5,fy=.5)=>page.locator(selector).evaluate((e,{fx,fy})=>{const r=e.getBoundingClientRect();return [r.x+r.width*fx,r.y+r.height*fy]},{fx,fy});
   const near=(actual,expected)=>actual.every((v,i)=>Math.abs(v-expected[i])<=3);
   const expectColor=async(selector,expected,label)=>{
    let actual;
    for(let i=0;i<25;i++){const [x,y]=await at(selector);actual=pixel(await screenshot(),x,y);if(near(actual,expected))return;await page.waitForTimeout(40)}
    assert.fail(`${width}/${zoom} ${label}: expected ${expected}, got ${actual}`);
   };
   await expectColor('#photo',[220,40,50],'hidden menus must not suppress image');
   await expectColor('#gif',[30,180,60],'GIF');
   await expectColor('#clear',[24,24,24],'transparent image backplane');
   await expectColor('#video',[220,40,50],'video first frame');
   await expectColor('#canvas',[220,40,50],'canvas first frame');
   // Wheel scrolling is compositor-driven. Waiting for layout to settle hid
   // the old fixed-copy lag, so deliberately defer only injected RAF work.
   await page.evaluate(()=>{window.deferOverlayFrames=true});
   const wheelBox=await page.locator('#scroller').boundingBox();
   await page.mouse.move(wheelBox.x+wheelBox.width-20,wheelBox.y+wheelBox.height/2);
   for(const delta of [14,14,-10,-18]) {
    await page.mouse.wheel(0,delta);
    await page.waitForTimeout(90);
    const drift=await page.evaluate(()=>['photo','gif','clear','video','canvas'].map((id,index)=>{
     const source=document.getElementById(id).getBoundingClientRect();
     const copy=document.querySelectorAll('.codex-plus-media-copy')[index].getBoundingClientRect();
     return {id,dx:Math.abs(source.x-copy.x),dy:Math.abs(source.y-copy.y)};
    }));
    assert.ok(drift.every(x=>x.dx<1 && x.dy<1),'wheel motion must not wait for injected layout: '+JSON.stringify(drift));
    maxWheelDrift=Math.max(maxWheelDrift,...drift.flatMap(x=>[x.dx,x.dy]));
   }
   await page.evaluate(()=>{document.querySelector('#scroller').scrollTop=0;window.resumeOverlayFrames()});
   await page.waitForTimeout(80);
   assert.equal(await page.locator('#controlled').evaluate(e=>e===window.controlledSource && e.controls && e.srcObject===window.stream),true,'original video and controls retained');
   const controlGeometry=await page.locator('#controlled').boundingBox();
   const placeholderGeometry=await page.locator('#row > [data-codex-plus-ext="image-overlay"]').boundingBox();
   for(const key of ['x','y','width','height'])assert.ok(Math.abs(controlGeometry[key]-placeholderGeometry[key])<1,`native controls geometry ${key}: ${controlGeometry[key]} vs ${placeholderGeometry[key]}`);
   for(const key of ['x','y','width','height'])assert.ok(Math.abs(controlGeometry[key]-geometryBefore[key])<1,`pre-injection geometry ${key}`);
   assert.deepEqual(await page.locator('#clip').boundingBox(),neighborBefore,'adjacent content layout preserved');
   await page.locator('#controlled').hover();
   const controlShot=await screenshot();
   let controlPixels=0;
   for(let y=controlGeometry.y+controlGeometry.height*.6;y<controlGeometry.y+controlGeometry.height-3;y+=2)
    for(let x=controlGeometry.x+5;x<controlGeometry.x+controlGeometry.width-5;x+=2)if(!near(pixel(controlShot,x,y),[220,40,50]))controlPixels++;
   assert.ok(controlPixels>100,'native playback controls are painted, not covered by a video-frame copy');
   await page.locator('#controlled').evaluate(e=>e.requestFullscreen());
   assert.equal(await page.evaluate(()=>document.fullscreenElement===controlledSource),true,'fullscreen uses the original video');
   await page.evaluate(()=>document.exitFullscreen());
   await page.waitForTimeout(80);
   await page.evaluate(()=>document.querySelector('#row').style.opacity='0');
   await page.waitForTimeout(80);
   assert.equal(await page.locator('#control-host').evaluate(e=>e.matches(':popover-open')),false,'hidden ancestor must hide promoted video');
   await page.evaluate(()=>document.querySelector('#row').style.opacity='1');
   await page.waitForTimeout(80);
   assert.equal(await page.locator('#control-host').evaluate(e=>e.matches(':popover-open')),true,'visible ancestor restores promoted video');
   if(output&&width===1088&&zoom===1) await page.screenshot({path:path.join(output,'foreground-fixture.png')});
   const ordinaryPoints=[[10,200],[width-8,850],[70*zoom,430*zoom],[75*zoom,535*zoom]];
   const ordinaryBefore=await screenshot();
   const originalGeometry=await page.locator('#codex-plus-image-overlay').boundingBox();
   await page.evaluate(()=>{document.querySelector('#menu').hidden=false});
   const [mx,my]=await at('#menu');
   assert.ok(near(pixel(await screenshot(),mx,my),[119,119,119]),'open menu must paint over media');
   await page.evaluate(()=>{document.querySelector('#menu').hidden=true});
   await expectColor('#photo',[220,40,50],'hidden toggle restores image');
   await page.evaluate(()=>{document.querySelector('#spacer').style.height='35px'});
   await expectColor('#photo',[220,40,50],'position-only ancestor layout');
   await page.evaluate(()=>{document.querySelector('#scroller').scrollTop=60;document.querySelector('#side').scrollTop=45});
   await page.waitForTimeout(80);
   const box=await page.locator('#scroller').boundingBox();
   const scrollShot=await screenshot();
   assert.ok(!near(pixel(scrollShot,box.x+25*zoom,box.y-4),[220,40,50]),'image must not escape scroll top');
   const clip=await page.locator('#clip').boundingBox();
   assert.ok(!near(pixel(scrollShot,clip.x+clip.width+5,clip.y+25*zoom),[220,40,50]),'image must not escape horizontal clip');
   await page.evaluate(()=>{document.querySelector('#scroller').scrollTop=0;document.querySelector('#spacer').style.height='0px'});
   await expectColor('#photo',[220,40,50],'scroll back');
   await page.evaluate(({red})=>{
    const nested=document.createElement('div');nested.id='nested';
    nested.style.cssText='width:160px;height:140px;overflow:auto;border:3px solid white;position:relative';
    const content=document.createElement('div');content.id='nested-content';
    content.style.cssText='width:500px;height:500px;padding:32px';
    const img=new Image();img.id='nested-photo';img.src=red;img.style.cssText='width:80px;height:80px';
    content.append(img);nested.append(content);document.querySelector('#scroller').prepend(nested);
   },{red});
   await expectColor('#nested-photo',[220,40,50],'nested initial');
   for(const rtl of [false,true]) {
    await page.evaluate(rtl=>{document.querySelector('#nested').style.direction=rtl?'rtl':'ltr'},rtl);
    await page.waitForTimeout(80);
    await page.evaluate(()=>{window.deferOverlayFrames=true});
    for(const amount of [12,24,8,0]) {
     await page.evaluate(({amount,rtl})=>{
      document.querySelector('#nested').scrollTop=amount;
      document.querySelector('#nested').scrollLeft=rtl?-amount:amount;
      document.querySelector('#scroller').scrollTop=amount/2;
     },{amount,rtl});
     await page.waitForTimeout(60);
     const delta=await page.evaluate(()=>{
      const source=document.querySelector('#nested-photo').getBoundingClientRect();
      const copy=[...document.querySelectorAll('.codex-plus-media-copy')].at(-1).getBoundingClientRect();
      return [Math.abs(source.x-copy.x),Math.abs(source.y-copy.y)];
     });
     assert.ok(delta.every(d=>d<1),'nested scroll alignment rtl='+rtl+': '+delta);
     maxNestedDrift=Math.max(maxNestedDrift,...delta);
    }
    await page.evaluate(()=>window.resumeOverlayFrames());
   }
   await page.evaluate(()=>{document.querySelector('#nested-content').style.height='730px';document.querySelector('#nested').style.direction='ltr'});
   await page.waitForTimeout(80);
   await page.evaluate(()=>{document.querySelector('#nested').style.overflow='clip'});
   await page.waitForTimeout(80);
   await page.evaluate(()=>{document.querySelector('#nested').style.overflow='auto'});
   await page.waitForTimeout(80);
   await page.evaluate(()=>{window.deferOverlayFrames=true;document.querySelector('#nested').scrollTop=26});
   await page.waitForTimeout(80);
   await expectColor('#nested-photo',[220,40,50],'changed scroll extent');
   await page.evaluate(()=>{window.resumeOverlayFrames();document.querySelector('#nested').remove();document.querySelector('#scroller').scrollTop=0});
   await page.waitForTimeout(80);
   await page.evaluate(()=>{
    const photo=document.querySelector('#photo'),scale=photo.currentCSSZoom||1;
    photo.style.cssText='position:fixed;left:'+(innerWidth-100)/scale+'px;top:'+220/scale+'px';
   });
   await expectColor('#photo',[220,40,50],'fixed media fallback');
   assert.equal(await page.locator('.codex-plus-media-copy').first().evaluate(e=>e.closest('.codex-plus-media-scrollport')),null,'fixed media must not follow ancestor scroll');
   await page.evaluate(()=>{document.querySelector('#scroller').scrollTop=40});
   await page.waitForTimeout(80);
   await expectColor('#photo',[220,40,50],'fixed media after scroll');
   await page.evaluate(()=>{document.querySelector('#photo').removeAttribute('style');document.querySelector('#scroller').scrollTop=0});
   await page.waitForTimeout(80);
   await page.evaluate(()=>paint('rgb(30,180,60)'));
   await expectColor('#video',[30,180,60],'video frame update');
   await page.evaluate(()=>document.querySelector('#video').pause());
   await page.evaluate(()=>paint('rgb(220,40,50)'));
   await page.waitForTimeout(140);
   await expectColor('#video',[30,180,60],'paused video retains frame');
   await page.evaluate(()=>{document.querySelector('#video').srcObject=null;document.querySelector('#video').removeAttribute('src');document.querySelector('#video').load()});
   await page.waitForTimeout(80);
   const emptyVideo=await at('#video');
   assert.ok(!near(pixel(await screenshot(),...emptyVideo),[30,180,60]),'empty video clears stale frame');
   await page.waitForTimeout(150);
   const idleStart=await page.evaluate(()=>rafCalls);
   await page.waitForTimeout(250);
   assert.ok((await page.evaluate(()=>rafCalls))-idleStart<=2,'empty video must not keep scheduling layout frames');
   await page.evaluate(({red})=>{
    const modal=document.createElement('div');modal.id='root-modal';modal.setAttribute('role','dialog');
    modal.innerHTML='<div data-testid="image-preview-dismiss-area"><img src="'+red+'"></div>';
    document.querySelector('#root').append(modal);
   },{red});
   await expectColor('#root-modal img',[220,40,50],'preview beneath root is not excluded');
   await page.evaluate(()=>document.querySelector('#root-modal').remove());
   await expectColor('#photo',[220,40,50],'modal close');
   await page.evaluate(({red})=>{
    const modal=document.createElement('div');modal.id='root-modal';modal.setAttribute('role','dialog');
    modal.style.cssText='inset:auto;left:180px;top:220px;width:140px;height:140px';
    modal.innerHTML='<div data-testid="image-preview-dismiss-area"><img src="'+red+'"></div>';
    document.querySelector('#shell').append(modal);
   },{red});
   await expectColor('#root-modal img',[220,40,50],'isolated inline preview fallback');
   assert.equal(await page.locator('#shell').evaluate(e=>getComputedStyle(e).zIndex),'auto','ordinary shell must never be raised');
   await page.evaluate(()=>document.querySelector('#root-modal').remove());
   await page.evaluate(({red})=>{const img=new Image();img.id='dynamic';img.src=red;document.querySelector('#row').append(img)},{red});
   await expectColor('#dynamic',[220,40,50],'dynamic image');
   await page.evaluate(()=>document.querySelector('#dynamic').remove());
   if(width===1088&&zoom===1) {
    const perf=await page.evaluate(async({red})=>{
      let reads=0;const original=Element.prototype.getBoundingClientRect;
      Element.prototype.getBoundingClientRect=function(){reads++;return original.call(this)};
      const wait=()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
      const bank=document.createElement('div');bank.style.cssText='position:absolute;top:3000px';
      for(let i=0;i<200;i++){const image=new Image();image.src=red;image.style.cssText='width:80px;height:80px';bank.append(image)}
      document.body.append(bank);await wait();await wait();
      reads=0;
      const text=document.querySelector('nav').firstChild;
      for(let i=0;i<30;i++){text.data=String(i);await wait()}
      const unrelatedReads=reads;reads=0;
      const related=document.createTextNode('');document.querySelector('#tail').append(related);
      for(let i=0;i<30;i++){related.data=String(i);await wait()}
      const relatedReads=reads;
      const start=performance.now();bank.remove();await wait();const cleanupMs=performance.now()-start;
      related.remove();Element.prototype.getBoundingClientRect=original;
      return {unrelatedReads,relatedReads,cleanupMs};
    },{red});
    assert.ok(perf.unrelatedReads<100,`unrelated streaming must not refresh media: ${JSON.stringify(perf)}`);
    assert.ok(perf.relatedReads<2000,`shared geometry work bounded: ${JSON.stringify(perf)}`);
    if(output)fs.writeFileSync(path.join(output,'performance.json'),JSON.stringify(perf,null,2));
   }
   assert.deepEqual(await page.locator('#codex-plus-image-overlay').boundingBox(),originalGeometry,'complete overlay geometry');
   const finalShot=await screenshot();
   for(const [x,y] of ordinaryPoints)assert.deepEqual(pixel(finalShot,x,y),pixel(ordinaryBefore,x,y),'ordinary injected paint');
   // Ordinary surfaces must match the exact original upstream tint, not just the new implementation before/after.
   await page.evaluate(()=>window.__codexPlusImageOverlayCleanup());
   const baseline=await screenshot();
   for(const [x,y] of ordinaryPoints)assert.deepEqual(pixel(finalShot,x,y),pixel(baseline,x,y),'ordinary upstream overlay equivalence');
   assert.equal(await page.locator('.codex-plus-media-plane').count(),0,'cleanup planes');
   assert.equal(await page.evaluate(()=>document.getAnimations().filter(a=>a.timeline instanceof ScrollTimeline).length),0,'cleanup scroll animations');
   assert.equal(await page.locator('#controlled').getAttribute('popover'),null,'native video restored');
   assert.equal(await page.locator('#row > [data-codex-plus-ext="image-overlay"]').count(),0,'placeholder removed');
   assert.deepEqual(await page.locator('#controlled').boundingBox(),geometryBefore,'native geometry restored on cleanup');
   assert.equal(await page.evaluate(()=>stream.getTracks().every(t=>t.readyState==='live')),true,'source stream preserved');
   for(let i=0;i<3;i++)await page.evaluate(()=>{installOverlay();window.__codexPlusImageOverlayCleanup()});
   assert.equal(await page.locator('.codex-plus-media-plane').count(),0,'repeated cleanup');
   await page.evaluate(()=>{clearInterval(paintTimer);stream.getTracks().forEach(t=>t.stop())});
   assert.deepEqual(errors,[],'page errors');
   results.push({width,zoom,passed:true,maxWheelDrift,maxNestedDrift});
   await page.close();
   console.log(`PASS ${width} zoom=${zoom}`);
  }
  if(output)fs.writeFileSync(path.join(output,'browser-verification.json'),JSON.stringify(results,null,2));
 }finally{await browser.close()}
}
main().catch(e=>{console.error(e);process.exitCode=1});
