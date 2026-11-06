const { assert, assets, check, configure, delta, html, near, pixel, rect, settle, shot } = require('./image-overlay-fixture.cjs');

const composer='<div id="composer" data-composer-surface-variant="default" data-composer-radius-variant="default"><div contenteditable="true" id="editor">Describe image</div></div>';
const viewer=()=>`<div id="viewer" class="codex-dialog" role="dialog"><div id="top"><div id="open" class="controlSurface-native"><button id="button">Open</button></div></div><img id="photo" src="${assets.photo}"><div id="tools" data-image-preview-toolbar role="toolbar" class="annotationTheme-native">Tools</div><div id="pen" class="pointer-events-auto rounded-full"><span role="slider" tabindex="0" aria-valuenow="4"></span></div><div id="target" data-image-preview-composer></div></div>`;
const css=`#host{position:absolute;left:60px;top:440px;width:680px}
 #composer{position:relative;width:680px;height:100px;background:#363636;border-radius:22px}#editor{padding:15px}
 #viewer{position:fixed;inset:0;z-index:50}#top{position:absolute;left:60px;top:20px}
 #open{display:flex;width:680px;height:45px;border-radius:30px;background:linear-gradient(#2d2d2d,#2d2d2d)}
 #open button{color:white;background:transparent}#photo{position:absolute;left:250px;top:130px;width:240px;height:160px}
 #tools{position:absolute;left:60px;top:330px;width:680px;height:40px;background:#2d2d2d;border-radius:15px}
 #pen{position:absolute;left:10px;top:180px;width:35px;height:180px;background:rgb(40 40 40 / .95);border-radius:30px}
 #target{position:absolute;left:60px;top:440px}`;
const colors=async(page,id,background)=>{
 const r=await rect(page,id),frame=await shot(page);
 [300,500].forEach((x,i)=>near(pixel(frame,x,r[1]+r[3]-8),[i?40:200,i?80:90,i?200:40].map(v=>Math.round(v*.3+background*.7)),id));
};
for(const late of [false,true])check(`viewer ${late?'late':'initial'}: chrome painted and image/composer preserved`,async page=>{
 await html(page,css,`<div id="host" data-thread-scroll-footer>${composer}</div>${late?'':viewer()}`);
 if(!late)await page.$eval('#composer',n=>document.getElementById('target').append(n));
 await configure(page,{dataUrl:assets.pattern,opacity:.3});
 if(late){await page.evaluate(markup=>document.body.insertAdjacentHTML('beforeend',markup),viewer());await page.$eval('#composer',n=>document.getElementById('target').append(n));await settle(page);}
 await colors(page,'#open',45);await colors(page,'#tools',45);await colors(page,'#composer',54);
 const r=await rect(page,'#photo');near(pixel(await shot(page),r[0]+50,r[1]+50),[220,40,50],'image remains untinted');
 assert.equal(await page.$eval('#pen',n=>n.hasAttribute('data-codex-plus-image-paint')),false,'translucent pen housing keeps native alpha');
 await page.$eval('#pen',n=>n.style.background='#282828');await settle(page);
 assert.ok(await page.$eval('#pen',n=>n.hasAttribute('data-codex-plus-image-paint')),'opaque pen housing is painted');
 await page.$eval('#pen',n=>n.style.background='rgb(40 40 40 / .95)');await settle(page);
 assert.equal(await page.$eval('#pen',n=>n.hasAttribute('data-codex-plus-image-paint')),false,'returning to alpha releases the extra paint');
 await page.locator('#editor').fill('Image question');await page.$eval('#button',n=>n.onclick=()=>window.clicked=true);await page.locator('#button').click();assert.ok(await page.evaluate(()=>window.clicked));
 await page.$eval('#tools',n=>n.style.background='#eeeeee');await settle(page);await colors(page,'#tools',238);
 await page.$eval('#viewer',n=>n.style.transform='translateX(-30px)');await settle(page);await colors(page,'#open',45);
 await page.setViewportSize({width:900,height:650});await settle(page);await colors(page,'#composer',54);
 await page.$eval('#composer',n=>document.getElementById('host').append(n));await page.$eval('#viewer',n=>n.remove());await settle(page);
 assert.equal(await page.locator('#editor').innerText(),'Image question','same editor and draft restored');await colors(page,'#composer',54);
});

const paintPosition = page => page.$eval("#composer", node => {
  const { x, y } = node.getBoundingClientRect();
  return { expected: `${-x}px ${-y}px`, actual: getComputedStyle(node, "::after").backgroundPosition };
});

check("viewer thumbnail animation does not rescan interaction cards", async page => {
  const cards = Array.from({ length: 30 }, () => '<div class="card" data-codex-approval-surface>Card</div>').join("");
  await html(page, css + ".card{height:50px;background:#2d2d2d}",
    `<div data-thread-scroll-footer>${cards}</div>${viewer()}`);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) {
      document.documentElement.className = "_opening_test_" + i;
      document.documentElement.style.setProperty("--image-preview-thumbnail-radius", i + "px");
      document.getElementById("photo").style.transform = `scale(${1 + i / 100})`;
      await new Promise(requestAnimationFrame);
    }
  }));
  assert.equal(work.styleReads, 0, "thumbnail-only animation must not read card styles");
  assert.equal(work.queries, 0, "thumbnail-only animation must not scan the document");
});
const resizingViewerCss = css + `
  #viewer{transform:translateX(-30px)}
  #target{top:auto;bottom:20px}
  #composer{height:auto;min-height:80px}
  #editor{white-space:pre-wrap}`;

check("viewer composer growth follows its bottom-anchored position", async page => {
  await html(page, resizingViewerCss, viewer());
  await page.$eval("#target", (node, markup) => node.innerHTML = markup, composer);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const before = await rect(page, "#composer");
  await page.locator("#editor").fill(Array(20).fill("Line").join("\n"));
  await settle(page);
  assert.ok((await rect(page, "#composer"))[1] < before[1], "typing grows the composer upward");
  const position = await paintPosition(page);
  assert.equal(position.actual, position.expected, "paint follows editor-driven resize");
  await colors(page, "#composer", 54);
});

for (const motion of ["transition", "animation"]) {
  check(`viewer ${motion} end follows the final transform`, async page => {
    await html(page, resizingViewerCss + `
      #viewer{transition:transform .15s linear}
      @keyframes moveViewer{to{transform:translateX(-80px)}}`, viewer());
    await page.$eval("#target", (node, markup) => node.innerHTML = markup, composer);
    await configure(page, { dataUrl: assets.pattern, opacity: .3 });
    await page.$eval("#viewer", async (node, kind) => {
      const ended = new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error(`${kind} did not finish`)), 3000);
        node.addEventListener(`${kind}end`, () => {
          clearTimeout(timeout);
          resolve();
        }, { once: true });
      });
      if (kind === "transition") node.style.transform = "translateX(-80px)";
      else node.style.animation = "moveViewer .15s linear forwards";
      await ended;
    }, motion);
    await settle(page);
    const position = await paintPosition(page);
    assert.equal(position.actual, position.expected, "paint follows final animated geometry");
    await colors(page, "#composer", 54);
  });
}
