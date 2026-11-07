const { assert, assets, check, configure, delta, html, near, pixel, rect, settle, shot } = require("./image-overlay-fixture.cjs");

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

// 2026-11-07 原生组件：QAOE 附件按钮、Sonner 中的 alert-root、Radix 上下文 tooltip。
// 用同坐标全屏壁纸的像素作参考，避免把“挂上标记”误当成正确合成。
for (const kind of ["annotation", "archive-toast", "context-tooltip"]) check(`native ${kind} receives full wallpaper strength`, async page => {
  const content = '<span>Native text</span><button id="action">Action</button><img id="icon" width="16" height="16">';
  const surface = kind === "annotation"
    ? '<div data-composer-surface-variant="default" data-composer-radius-variant="default"><button id="transient" data-composer-attachment-pill class="composer-attachment-surface">Annotation</button></div>'
    : kind === "archive-toast"
    ? `<div class="codex-toast-area"><ol data-sonner-toaster><li data-sonner-toast><div id="transient" class="alert-root">${content}</div></li></ol></div>`
    : `<div class="pointer-events-none fixed inset-0 zoom-adjusted-viewport contain-layout"><div data-radix-popper-content-wrapper><div id="transient" role="tooltip" class="Tooltip-fixture">Context window<svg id="tooltip-icon" width="16" height="16"><rect width="16" height="16" fill="#00ff00"/></svg></div></div></div>`;
  await html(page, `
    body{background:#181818}.codex-toast-area,.fixed{position:fixed;inset:0;z-index:60;pointer-events:none}
    .contain-layout{contain:layout}[data-radix-popper-content-wrapper]{position:fixed;transform:translate(60px,100px)}
    #transient{position:relative;display:block;width:600px;height:120px;background:#242424;border:0;border-radius:18px;color:white;pointer-events:auto}
    [data-composer-surface-variant], [data-sonner-toast]{position:absolute;left:60px;top:100px}
    [data-sonner-toast]{list-style:none;transform:translateY(0)}#icon{display:inline-block}#action{background:#ddd;color:#111}
  `, surface);
  if (kind === "archive-toast") await page.$eval('#icon', (n, src) => { n.src = src; }, assets.photo);
  const before = await rect(page, '#transient');
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const frame = await shot(page);
  for (const [index, x] of [300,500].entries()) near(pixel(frame,x,190), expected([36,36,36])[index], kind);
  assert.deepEqual(await rect(page, '#transient'), before, 'native positioning stays intact');
  await page.$eval('#transient', n => { n.style.background = 'rgba(36,36,36,.4)'; });
  await settle(page);
  assert.equal(await page.$eval('#transient', n => n.hasAttribute('data-codex-plus-image-paint')), false,
    'native alpha material is never painted again');
  if (kind === 'context-tooltip') {
    const r = await rect(page, '#tooltip-icon');
    near(pixel(await shot(page),r[0]+8,r[1]+8),[0,255,0],'alpha tooltip foreground is not tinted by wallpaper');
  }
  await page.$eval('#transient', n => { n.style.background = '#242424'; });
  await settle(page);
  near(pixel(await shot(page),300,190), expected([36,36,36])[0], 'opaque material resumes wallpaper');
  if (kind === "archive-toast") {
    await page.$eval('#action', n => { n.onclick = () => { window.actionClicked = true; }; });
    await page.locator('#action').click();
    assert.ok(await page.evaluate(() => window.actionClicked));
    const r = await rect(page, '#icon');
    near(pixel(frame,r[0]+8,r[1]+8), [220,40,50], 'notification icon remains native');
  }
});

check('native attachment identity can be added and removed without remounting', async page => {
  await html(page, '[data-composer-surface-variant]{position:absolute;left:60px;top:100px}#pill{position:relative;width:600px;height:120px;background:#242424;border:0}',
    '<div data-composer-surface-variant="default" data-composer-radius-variant="default"><button id="pill">Annotation</button></div>');
  await configure(page, {dataUrl:assets.pattern,opacity:.3});
  await page.$eval('#pill', n => n.setAttribute('data-composer-attachment-pill','true'));
  await settle(page);
  near(pixel(await shot(page),300,190), expected([36,36,36])[0], 'late native identity discovers pill');
  await page.$eval('#pill', n => n.removeAttribute('data-composer-attachment-pill'));
  await settle(page);
  assert.equal(await page.$eval('#pill', n => n.hasAttribute('data-codex-plus-image-paint')),false);
});

check('tooltip classification changes release the previous popup material', async page => {
  await html(page, '.fixed{position:fixed;inset:0;z-index:60;pointer-events:none;contain:layout}[data-radix-popper-content-wrapper]{position:fixed;transform:translate(60px,100px)}#tip{width:600px;height:120px;background:#242424}',
    '<div class="fixed inset-0 pointer-events-none zoom-adjusted-viewport contain-layout"><div data-radix-popper-content-wrapper><div id="tip" role="tooltip">Context</div></div></div>');
  await configure(page,{dataUrl:assets.pattern,opacity:.3});
  for (const name of ['Tooltip-fixture','','Tooltip-fixture']) {
    await page.$eval('#tip',(n,c)=>{n.className=c;},name); await settle(page);
    if (name) near(pixel(await shot(page),300,190),expected([36,36,36])[0],'native tooltip replaces generic alpha material');
    else assert.equal(await page.$eval('#tip',n=>n.hasAttribute('data-codex-plus-image-paint')),false);
  }
  await page.$eval('#tip',n=>{n.style.background='var(--tooltip-color,#242424)';});
  await page.evaluate(()=>document.documentElement.style.setProperty('--tooltip-color','rgba(36,36,36,.4)'));
  await settle(page);
  assert.equal(await page.$eval('#tip',n=>n.hasAttribute('data-codex-plus-image-paint')),false,'inherited alpha keeps native material');
  await page.evaluate(()=>document.documentElement.style.setProperty('--tooltip-color','#242424'));
  await settle(page);
  near(pixel(await shot(page),300,190),expected([36,36,36])[0],'root theme restores opaque tooltip');
});

check('late Sonner identity discovers existing notification material', async page => {
  await html(page,'.codex-toast-area{position:fixed;inset:0;z-index:60}li{position:absolute;left:60px;top:100px;list-style:none}.alert-root{width:600px;height:120px;background:#242424}',
    '<div class="codex-toast-area"><ol><li id="toast"><div class="alert-root">Archived</div></li></ol></div>');
  await configure(page,{dataUrl:assets.pattern,opacity:.3});
  await page.$eval('#toast',n=>n.setAttribute('data-sonner-toast','true')); await settle(page);
  near(pixel(await shot(page),300,190),expected([36,36,36])[0],'late toast marker discovers opaque card');
});

check("streaming text and unrelated attribute changes do not rescan local surfaces", async page => {
  const cards = Array.from({ length: 30 }, () => '<div data-codex-approval-surface style="height:10px;background:#2d2d2d">Card</div>').join("");
  await html(page, css, `<div id="stream"><span>Text</span></div><div data-thread-scroll-footer>${cards}${composer}</div>`);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const work = await delta(page, () => page.evaluate(async () => {
    const stream = document.getElementById("stream");
    for (let i = 0; i < 20; i++) {
      stream.firstChild.firstChild.data = "Streaming " + i;
      stream.className = "streaming-state-" + i;
      stream.style.setProperty("--stream-progress", String(i));
      await new Promise(requestAnimationFrame);
    }
  }));
  assert.equal(work.queries, 0, "unrelated stream must not query surface subtrees");
  assert.equal(work.styleReads, 0, "unrelated stream must not read surface styles");
});

check("tracked ancestors retain theme discovery after candidate reparenting", async page => {
  await html(page, css + "#card{background:var(--card,transparent)}.dark{--card:#2d2d2d}",
    `<div id="host" data-thread-scroll-footer><div id="first">${card("data-composer-rail-item")}</div><div id="second"></div></div>`);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const painted = () => page.$eval("#card", node => node.hasAttribute("data-codex-plus-image-paint"));
  assert.equal(await painted(), false);
  await page.$eval("#first", node => node.classList.add("dark"));
  await settle(page);
  assert.equal(await painted(), true, "ancestor class discovers previously transparent card");
  await page.$eval("#card", node => document.getElementById("second").append(node));
  await settle(page);
  assert.equal(await painted(), false, "reparenting removes inherited opaque background");
  await page.$eval("#second", node => node.style.setProperty("--card", "#eeeeee"));
  await settle(page);
  assert.equal(await painted(), true, "new ancestor is tracked for inherited theme changes");
});

for (const [name, background, painted] of [
  ["opaque color", "#2d2d2d", true],
  ["transparent color", "transparent", false],
  ["low alpha color", "rgba(45,45,45,.04)", false],
  ["high alpha color", "rgb(45 45 45 / .95)", false],
  ["modern alpha color", "color(srgb .2 .2 .2 / .5)", false],
  ["opaque gradient", "linear-gradient(#2d2d2d,#555)", true],
  ["transparent gradient", "linear-gradient(#2d2d2d,transparent)", false],
  ["partial gradient", "linear-gradient(#2d2d2d,#555) center / 50% 50% no-repeat", false],
  ["image background", `url('${assets.photo}')`, false],
]) check(`local ${name} follows the opaque material policy`, async page => {
  await html(page, css, `<div id="host" data-thread-scroll-footer>${composer}</div>`);
  await page.$eval("#composer", (node, markup) => node.innerHTML = markup, card("data-composer-rail-item"));
  await page.$eval("#card", (node, background) => node.style.background = background, background);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  assert.equal(await page.$eval("#card", node => node.hasAttribute("data-codex-plus-image-paint")), painted, name);
  if (!painted) {
    const reference = await shot(page);
    await page.$eval("#card", node => node.removeAttribute("data-composer-rail-item"));
    await settle(page);
    assert.deepEqual(await samples(page, await shot(page)), await samples(page, reference), "semantic identity must not add a second layer over partial material");
  }
});

check("late gallery discovery tracks transparent candidates before ancestor theme changes", async page => {
  await html(page, css + "#card{background:var(--card,transparent)}",
    '<div data-thread-scroll-footer><div id="host"><div id="theme"><div id="card" class="bg-surface-elevated-secondary">Card</div></div></div></div>');
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  await page.$eval("#host", node => node.classList.add("tortilla-gallery"));
  await settle(page);
  assert.equal(await page.$eval("#card", node => node.hasAttribute("data-codex-plus-image-paint")), false);
  await page.$eval("#theme", node => node.style.setProperty("--card", "#2d2d2d"));
  await settle(page);
  assert.equal(await page.$eval("#card", node => node.hasAttribute("data-codex-plus-image-paint")), true,
    "late transparent candidate must track its own theme ancestors");
});

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

for (const host of ["progress", "footer", "dialog", "gallery"]) {
  check(`late ${host} ancestor identity discovers and retires descendant paint`, async page => {
    const progress = host === "progress";
    const cardAttributes = progress || host === "gallery"
      ? 'class="rounded-3xl border bg-surface-elevated-secondary"' : "data-codex-approval-surface";
    const outerAttributes = progress || host === "gallery" ? "data-thread-scroll-footer" : "";
    await html(page, css, `<div style="position:relative" ${outerAttributes}><div id="host">${card(cardAttributes)}</div></div>`);
    await configure(page, { dataUrl: assets.pattern, opacity: .3 });
    const painted = () => page.$eval("#card", node => node.hasAttribute("data-codex-plus-image-paint"));
    assert.equal(await painted(), false, "no local paint before ancestor identity");
    const attribute = host === "progress" ? "data-in-progress-fixed-content"
      : host === "footer" ? "data-thread-scroll-footer" : host === "dialog" ? "role" : "class";
    await page.$eval("#host", (node, { attribute, host }) =>
      node.setAttribute(attribute, host === "dialog" ? "dialog" : host === "gallery" ? "tortilla-gallery" : ""), { attribute, host });
    await settle(page);
    assert.equal(await painted(), true, "ancestor identity discovers existing descendants");
    (await samples(page, await shot(page))).forEach((color, i) => near(color, expected([45,45,45])[i]));
    await page.$eval("#host", (node, attribute) => node.removeAttribute(attribute), attribute);
    await settle(page);
    assert.equal(await painted(), false, "removing ancestor identity releases local paint");
    const photo = await rect(page, "#photo");
    near(pixel(await shot(page), photo[0] + 15, photo[1] + 15), [220,40,50], "retired boundary preserves native media");
  });
}

check("ancestor theme and CSS variables reevaluate transparent nested cards", async page => {
  await html(page, css + `#composer{height:200px;background:#363636}
    #card{background:var(--card,transparent)}.dark #card{background:var(--card,#2d2d2d)}`,
    `<div id="host" data-thread-scroll-footer>${composer}</div>`);
  await page.$eval("#composer", (node, markup) => node.innerHTML = markup, card("data-composer-rail-item"));
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const painted = () => page.$eval("#card", node => node.hasAttribute("data-codex-plus-image-paint"));
  assert.equal(await painted(), false, "transparent card uses the composer's paint");
  await page.$eval("body", node => node.classList.add("dark"));
  await settle(page);
  assert.equal(await painted(), true, "ancestor class reveals an opaque card");
  (await samples(page, await shot(page))).forEach((color, i) => near(color, expected([45,45,45])[i]));
  await page.$eval("body", node => node.style.setProperty("--card", "transparent"));
  await settle(page);
  assert.equal(await painted(), false, "inherited transparency releases the second paint");
  (await samples(page, await shot(page))).forEach((color, i) => near(color, expected([54,54,54])[i]));
  await page.$eval("body", node => node.style.setProperty("--card", "#eeeeee"));
  await settle(page);
  assert.equal(await painted(), true, "inherited opaque color restores paint");
  (await samples(page, await shot(page))).forEach((color, i) => near(color, expected([238,238,238])[i]));
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
