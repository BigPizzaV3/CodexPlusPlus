const { assert, chromium, sharp, shot: snapshot, pixel, near: assertNear, configure: configureOverlay } = require("./image-overlay-fixture.cjs");
// Pixel comparisons against the original upstream overlay, not selector coverage.
const fs = require("node:fs");
const path = require("node:path");
const results = [];
const out = process.env.CODEX_OVERLAY_TEST_OUTPUT;
const bitmap = async (color, width = 80, height = 80, format = "png") =>
  `data:image/${format};base64,${(await sharp({ create: { width, height, channels: 4, background: color } })
    .toFormat(format).toBuffer()).toString("base64")}`;
const at = async (page,id) => page.$eval(id,node => {
  const r = node.getBoundingClientRect();
  return { x:r.x, y:r.y, width:r.width, height:r.height };
});
const configure = (page, dataUrl, fitMode = "stretch", opacity = .2) =>
  configureOverlay(page, { dataUrl, fitMode, opacity });
async function reference(page, wall, fitMode, opacity) {
  await page.evaluate(({wall,fitMode,opacity}) => {
    window.__codexPlusImageOverlayCleanup?.(true);
    const fit = {
      fill:["cover","center center","no-repeat"],fit:["contain","center center","no-repeat"],
      stretch:["100% 100%","center center","no-repeat"],tile:["auto","left top","repeat"],
      center:["auto","center center","no-repeat"],
    }[fitMode];
    const div = document.createElement("div");
    div.id = "upstream-reference";
    Object.assign(div.style, {
      position:"fixed",inset:"0",width:"100vw",height:"100vh",backgroundImage:`url("${wall}")`,
      backgroundSize:fit[0],backgroundPosition:fit[1],backgroundRepeat:fit[2],opacity:String(opacity),
      pointerEvents:"none",zIndex:"2147483646",userSelect:"none",
    });
    document.documentElement.appendChild(div);
  }, {wall,fitMode,opacity});
  await page.waitForTimeout(50);
}
const fixture = (theme, photo, gif, zoom = 1) => `<!doctype html><html><head><style>
  html,body{margin:0;width:100%;height:100%;overflow:hidden;color:${theme==="dark"?"#eee":"#171717"};background:${theme==="dark"?"#181818":"#fafafa"};font:14px Arial}
  *{box-sizing:border-box}#shell{zoom:${zoom};height:100vh}nav{width:15%;height:100%;background:${theme==="dark"?"#343434":"#ededed"};float:left}
  main{margin-left:15%;padding:15px;position:relative}button,input,textarea{color:inherit;background:transparent;border:1px solid #888;padding:6px}
  #unknown{background:#42764c;padding:9px}.translucent{background:#78787870}#thread{height:200px;overflow:auto;position:relative}
  .media{width:64px;height:64px;display:block;border-radius:12px;object-fit:cover}#pictures{display:flex;gap:12px;padding:8px}
  #attachment{position:relative;width:64px;height:64px}#remove{position:absolute;top:0;right:0;width:22px;height:22px;padding:0;background:#fafafa;font-size:10px}
  #tail{height:900px;background:linear-gradient(#8882,#4442)}#browser{width:140px;height:80px;background:#28323c}
  #guest{width:100%;height:70px;border:0}#shadow{height:40px}#popup{display:none;position:absolute;top:45px;left:45px;width:210px;height:100px;background:#353535;border-radius:8px;z-index:10}
  #preview{display:none;position:fixed;inset:0;background:#242424ee;z-index:99}#preview img{width:200px;height:200px;margin:20px}
  </style></head><body><div id="shell"><nav>Navigation<br><button>Native menu</button><div class="translucent">Tree capsules</div></nav>
  <main><button id="menu" role="menuitem">Application</button><button id="model">Model selection</button><button>Summary</button>
  <h3>Ordinary text and <svg width="20" height="20"><circle cx="10" cy="10" r="8" fill="#62c879"/></svg> icons</h3>
  <div id="unknown">Unknown component with custom paint</div><div class="translucent"><input value="Input field"><textarea>Composer</textarea></div>
  <div id="thread"><div id="pictures"><div id="attachment"><img id="photo" class="media" src="${photo}"><button id="remove">X</button></div><img id="gif" class="media" src="${gif}">
  <canvas id="canvas" class="media" width="64" height="64"></canvas><video id="video" class="media" autoplay muted playsinline></video></div><div id="tail"></div></div>
  <div id="shadow"></div><iframe id="guest" srcdoc="<body style='background:#232323;color:white'>Code-review settings and provider<input value='ordinary'></body>"></iframe>
  <div id="browser" data-browser-sidebar-webview>Browser <button>Native</button></div>
  <div id="popup" role="menu">Popup above media<button>Ordinary command</button></div></main>
  <div id="preview" role="dialog"><div data-testid="image-preview-dismiss-area"><img src="${photo}"></div></div></div></body></html>`;
async function prepare(page, html) {
  await page.setContent(html);
  await page.evaluate(() => {
    const shadow = document.querySelector("#shadow").attachShadow({mode:"open"});
    shadow.innerHTML = "<style>div{background:#563a66;color:white;height:40px}</style><div>Native file-tree shadow rows</div>";
    const canvas = document.querySelector("#canvas");
    canvas.getContext("2d").fillStyle = "#19b450";
    canvas.getContext("2d").fillRect(0,0,64,64);
    window.testStream = canvas.captureStream(10);
    document.querySelector("#video").srcObject = window.testStream;
  });
  await page.waitForTimeout(150);
}
async function main() {
  const wall = await bitmap({r:40,g:80,b:200,alpha:1},160,90);
  const photo = await bitmap({r:220,g:40,b:50,alpha:1});
  const gif = await bitmap({r:30,g:180,b:60,alpha:1},80,80,"gif");
  const browser = await chromium.launch({channel:process.env.CODEX_TEST_BROWSER || "msedge",
    executablePath:process.env.CODEX_TEST_BROWSER_PATH,headless:true});
  try {
    for (const theme of ["dark","light"]) for (const width of (process.env.CODEX_COMPOSITE_FITS_ONLY?[]:[480,1088])) for (const zoom of [.8,1,1.25]) {
      const page = await browser.newPage({viewport:{width,height:900}});
      const errors = [];
      page.on("pageerror",error => errors.push(error.message));
      await prepare(page,fixture(theme,photo,gif,zoom));
      const native = await snapshot(page);
      await reference(page,wall,"stretch",.2);
      const baseline = await snapshot(page);
      await page.$eval("#upstream-reference",node=>node.remove());
      await configure(page,wall);
      const actual = await snapshot(page);
      const mediaBoxes = await page.evaluate(() => [...document.querySelectorAll("img,video,canvas,[data-browser-sidebar-webview]")]
        .map(node=>{const r=node.getBoundingClientRect();return {l:r.left-1,t:r.top-1,r:r.right+1,b:r.bottom+1};}));
      let ordinary = 0, mismatch = 0, maxError = 0;
      for(let y=0;y<actual.height;y++)for(let x=0;x<actual.width;x++){
        if(mediaBoxes.some(r=>x>=r.l&&x<=r.r&&y>=r.t&&y<=r.b))continue;
        const offset=(y*actual.width+x)*4;
        ordinary++;
        const error=Math.max(...[0,1,2].map(c=>Math.abs(actual.data[offset+c]-baseline.data[offset+c])));
        maxError=Math.max(maxError,error);
        if(error>1)mismatch++;
      }
      assert.equal(mismatch,0,`${theme}/${width}/${zoom}: ordinary full-frame mismatches=${mismatch}, max=${maxError}`);
      const remove=await at(page,"#remove");
      assertNear(pixel(actual,remove.x+3,remove.y+3),pixel(baseline,remove.x+3,remove.y+3),"ordinary attachment remove control");
      if(out && width===1088 && zoom===1){
        fs.mkdirSync(out,{recursive:true});
        await sharp(await page.screenshot()).resize({width:960}).jpeg({quality:78}).toFile(path.join(out,`composite-${theme}-review.jpg`));
      }
      for(const id of ["#photo","#gif","#canvas","#video","#browser"]){
        const r=await at(page,id);
        if(r.x+r.width/2<width){
          assertNear(pixel(actual,r.x+r.width/2,r.y+r.height/2),pixel(native,r.x+r.width/2,r.y+r.height/2),id);
        }
      }
      const before = await page.$eval("#photo",node=>({parent:node.parentElement.id,src:node.src,style:node.getAttribute("style")}));
      await page.$eval("#thread",node=>{node.scrollTop=20;});
      await page.waitForTimeout(60);
      const scrolling = await snapshot(page);
      const r=await at(page,"#photo");
      assertNear(pixel(scrolling,r.x+r.width/2,r.y+r.height/2),[220,40,50],"scrolled native photo");
      const scrolledRemove=await at(page,"#remove");
      assertNear(pixel(scrolling,scrolledRemove.x+3,scrolledRemove.y+scrolledRemove.height-3),[208,216,240],"ordinary image control scrolls with native tint");
      assert.deepEqual(await page.$eval("#photo",node=>({parent:node.parentElement.id,src:node.src,style:node.getAttribute("style")})),before);
      await page.$eval("#thread",node=>{node.scrollTop=0;});
      await page.$eval("#popup",node=>{node.style.display="block";});
      await page.waitForTimeout(60);
      const popup=await at(page,"#popup");
      const menuShot=await snapshot(page);
      await page.$eval("#popup",node=>{node.style.visibility="hidden";});
      const popupUnderlay=await snapshot(page);
      await page.$eval("#popup",node=>{node.style.visibility="visible";});
      const popupExpected=pixel(popupUnderlay,popup.x+5,popup.y+45).map(v=>Math.round(53*.8+v*.2));
      assertNear(pixel(menuShot,popup.x+5,popup.y+45),popupExpected,"popup alpha over real underlying surface",2);
      await page.$eval("#popup",node=>{node.style.display="none";});
      await page.$eval("#preview",node=>{node.style.display="block";});
      await page.waitForTimeout(60);
      const previewNative=await page.evaluate(()=>{
        window.__codexPlusImageOverlayCleanup?.(true);
        return true;
      });
      assert.ok(previewNative);
      await reference(page,wall,"stretch",.2);
      await page.$eval("#preview",node=>{node.style.zIndex="2147483647";});
      const previewRef=await snapshot(page);
      await page.$eval("#upstream-reference",node=>node.remove());
      await configure(page,wall);
      const previewActual=await snapshot(page);
      for(let y=0;y<previewActual.height;y++)for(let x=0;x<previewActual.width;x++){
        if(mediaBoxes.some(r=>x>=r.l&&x<=r.r&&y>=r.t&&y<=r.b))continue;
        assertNear(pixel(previewActual,x,y),pixel(previewRef,x,y),"native viewer over persistent upstream wallpaper");
      }
      await page.$eval("#preview",node=>{node.style.display="none";});
      await page.evaluate(()=>window.__codexPlusImageOverlayCleanup?.());
      assert.equal(await page.locator("#codex-plus-image-overlay").count(),0);
      assert.deepEqual(errors,[]);
      results.push({theme,width,zoom,ordinaryPixels:ordinary,maxError,passed:true});
      await page.close();
    }
    const page=await browser.newPage({viewport:{width:640,height:480}});
    await page.setContent("<style>body{margin:0;background:#333;color:#eee}</style><h3>Ordinary controls</h3><button>Native button</button>");
    for(const fit of ["fill","fit","stretch","tile","center"]){
      for(const opacity of [.01,.2,.75,1]){
        await reference(page,wall,fit,opacity);
        const baseline=await snapshot(page);
        await page.$eval("#upstream-reference",node=>node.remove());
        await configure(page,wall,fit,opacity);
        const actual=await snapshot(page);
        if(Buffer.compare(actual.data,baseline.data)){
          const differences=[];
          for(let i=0;i<actual.data.length && differences.length<8;i+=4){
            if([0,1,2].some(c=>actual.data[i+c]!==baseline.data[i+c]))
              differences.push({x:(i/4)%actual.width,y:Math.floor(i/4/actual.width),actual:[...actual.data.subarray(i,i+3)],expected:[...baseline.data.subarray(i,i+3)]});
          }
          console.log(JSON.stringify({fit,opacity,differences}));
        }
        assert.equal(Buffer.compare(actual.data,baseline.data),0,`${fit}/${opacity} exact upstream pixels`);
        await page.evaluate(()=>window.__codexPlusImageOverlayCleanup?.(true));
      }
    }
    results.push({group:"all-fits-and-strengths",comparisons:20,passed:true});
    await page.close();
    await lifecycle(browser,wall,photo);
    console.log(JSON.stringify({groups:results.length,results},null,2));
    if(out){fs.mkdirSync(out,{recursive:true});fs.writeFileSync(path.join(out,"composite-verification.json"),JSON.stringify(results,null,2));}
  } finally {await browser.close();}
}
async function lifecycle(browser,wall,photo){
  const page=await browser.newPage({viewport:{width:700,height:500}});
  await page.setContent(`<style>body{margin:0;background:#181818}#host{padding:20px}img{width:80px;height:80px}</style><div id="host"></div>`);
  await configure(page,wall);
  const resource=await page.evaluate(()=>window.__codexPlusImageOverlayBlobUrl);
  await page.evaluate(photo=>{
    const image=document.createElement("img");image.id="dynamic";image.src=photo;
    document.querySelector("#host").appendChild(image);
  },photo);
  await page.waitForTimeout(80);
  let r=await at(page,"#dynamic"),shot=await snapshot(page);
  assertNear(pixel(shot,r.x+40,r.y+40),[220,40,50],"dynamic media excludes on mount");
  await page.evaluate(()=>{
    document.querySelector("#dynamic").animate([{transform:"translateX(0)"},{transform:"translateX(100px)"}],
      {duration:400,fill:"forwards",easing:"linear"});
  });
  await page.waitForTimeout(160);
  shot=await snapshot(page);r=await at(page,"#dynamic");
  assertNear(pixel(shot,r.x+40,r.y+40),[220,40,50],"native WAAPI image transition is not reconstructed");
  await page.waitForTimeout(320);
  await page.setViewportSize({width:1000,height:700});
  await page.waitForTimeout(80);
  await configure(page,wall,"fill",.2);
  assert.equal(await page.evaluate(()=>window.__codexPlusImageOverlayBlobUrl),resource,"same image retains decoded blob");
  r=await at(page,"#dynamic");shot=await snapshot(page);
  assertNear(pixel(shot,r.x+40,r.y+40),[220,40,50],"resize retains media exclusion");
  await page.evaluate(()=>{
    document.querySelector("#dynamic").style.visibility="hidden";
  });
  await page.waitForTimeout(80);
  shot=await snapshot(page);
  assertNear(pixel(shot,r.x+40,r.y+40),[27,35,59],"hidden media leaves ordinary tint");
  await page.evaluate(()=>{
    window.__CODEX_PLUS_IMAGE_OVERLAY__.enabled=false;
    installCodexPlusImageOverlay();
  });
  assert.equal(await page.locator("#codex-plus-image-overlay").count(),0);
  assert.equal(await page.evaluate(()=>window.__codexPlusImageOverlayBlobUrl),"");
  assert.equal(await page.evaluate(()=>window.testStream?.active ?? true),true);
  results.push({group:"lifecycle-and-native-animation",passed:true});
  await page.close();
}
main().catch(error=>{console.error(error);process.exitCode=1;});
