const { assert, assets, check, configure, html, near, pixel, rect, settle, shot } = require("./image-overlay-fixture.cjs");

const shells = [
  ['question', 'data-codex-composer-request-navigation'],
  ['approval', 'data-codex-approval-surface'],
  ['legacy-card', 'class="flex flex-col overflow-hidden rounded-3xl border bg-surface-elevated-secondary"'],
  ['rail', 'data-composer-rail-item="present"'],
  ['tip', 'class="group relative flex flex-wrap rounded-2xl bg-surface-elevated-secondary"'],
];
const css = `#host{position:absolute;left:60px;top:100px;width:680px}
  #card{background:var(--card,#2d2d2d);border-radius:24px;height:180px;box-sizing:border-box}
  #content{padding:20px}#photo{width:30px;height:30px}button{background:#414141;color:white}
  #editor{height:35px}#composer{position:relative;background:#2d2d2d;height:80px;margin-top:12px;border-radius:24px}`;
const composer = '<div id="composer" data-composer-surface-variant="default" data-composer-radius-variant="default"><div contenteditable="true">Composer</div></div>';
const card = attrs => `<div id="card" ${attrs}><div id="content"><img id="photo" src="${assets.photo}"><div id="editor" contenteditable="true">Draft</div><button id="action">Action</button></div></div>`;
const samples = async (page, frame) => { const r = await rect(page, '#card'); return [300,500].map(x => pixel(frame,x,r[1]+155)); };
const expected = color => [[200,90,40],[40,80,200]].map(w => w.map((v,i)=>Math.round(v*.3+color[i]*.7)));

for (const host of ['footer','dialog','timeline']) for (const [name, attrs] of shells)
  check(`${host} ${name}: wallpaper background, native content and geometry`, async page => {
    await html(page,css,`<div class="thread-scroll-container"><div id="host" ${host==='footer'?'data-thread-scroll-footer':host==='dialog'?'role="dialog"':''}>${card(attrs)}${composer}</div></div>`);
    const geometry=await rect(page,'#card');
    await page.$eval('#action',n=>n.onclick=()=>window.actionClicked=true);
    await configure(page,{dataUrl:assets.pattern,opacity:.3});
    const actual=await samples(page,await shot(page));
    actual.forEach((c,i)=>near(c,expected([45,45,45])[i],name+' background'));
    const photo=await rect(page,'#photo');near(pixel(await shot(page),photo[0]+15,photo[1]+15),[220,40,50],'photo stays native');
    assert.deepEqual(await rect(page,'#card'),geometry);
    await page.locator('#editor').fill('Edited');await page.locator('#action').click();
    assert.equal(await page.evaluate(()=>window.actionClicked),true);
    if(host==='timeline')assert.equal(await page.$eval('#card',n=>n.hasAttribute('data-codex-plus-image-paint')),false,'timeline uses global wallpaper once');
  });

check('late rail, transparent variant, theme change and reparenting do not double paint', async page=>{
  await html(page,css,'<div id="host" data-thread-scroll-footer>'+composer+'</div><div id="timeline" style="position:absolute;left:60px;top:100px;width:680px"></div>');
  await configure(page,{dataUrl:assets.pattern,opacity:.3});
  await page.$eval('#host',(n,markup)=>n.insertAdjacentHTML('afterbegin',markup),card('data-composer-rail-item="present"'));
  await settle(page);
  (await samples(page,await shot(page))).forEach((c,i)=>near(c,expected([45,45,45])[i],'late rail'));
  await page.$eval('#card',n=>{n.style.background='transparent';document.getElementById('composer').append(n)});
  await settle(page);
  assert.equal(await page.$eval('#card',n=>n.hasAttribute('data-codex-plus-image-paint')),false,'transparent embedded rail has no second paint');
  await page.$eval('#card',n=>{n.style.background='#eeeeee';document.getElementById('host').prepend(n)});await settle(page);
  (await samples(page,await shot(page))).forEach((c,i)=>near(c,expected([238,238,238])[i],'light surface'));
  await page.$eval('#card',n=>document.getElementById('timeline').append(n));await settle(page);
  (await samples(page,await shot(page))).forEach((c,i)=>near(c,expected([238,238,238])[i],'moved to timeline, one blend'));
});

check('progress capsule uses its own background inside animated portal',async page=>{
  await html(page,css,`<div id="host" data-thread-scroll-footer><div data-in-progress-fixed-content style="transform:translateY(0)">${card('class="rounded-3xl border bg-surface-elevated-secondary"')}</div>${composer}</div>`);
  await configure(page,{dataUrl:assets.pattern,opacity:.3});
  (await samples(page,await shot(page))).forEach((c,i)=>near(c,expected([45,45,45])[i],'progress capsule'));
});

check('late card identity and nested gallery backgrounds retain independent paints',async page=>{
  await html(page,css,`<div id="host" role="dialog"><div class="tortilla-gallery bg-surface-elevated-secondary" style="background:#363636;padding:10px">${card('')}</div></div>`);
  await configure(page,{dataUrl:assets.pattern,opacity:.3});
  await page.$eval('#card',n=>n.setAttribute('data-codex-approval-surface','true'));await settle(page);
  (await samples(page,await shot(page))).forEach((c,i)=>near(c,expected([45,45,45])[i],'late card identity'));
  await page.$eval('.tortilla-gallery',n=>n.style.background='#444');await settle(page);
  (await samples(page,await shot(page))).forEach((c,i)=>near(c,expected([45,45,45])[i],'nested surface survives parent change'));
  await page.$eval('#card',n=>n.removeAttribute('data-codex-approval-surface'));await settle(page);
  assert.equal(await page.$eval('#card',n=>n.hasAttribute('data-codex-plus-image-paint')),false,'removed identity releases paint');
});
