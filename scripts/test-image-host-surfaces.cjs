const { assert, assets, check, configure, delta, html, near, pixel, rect, settle, shot } = require("./image-overlay-fixture.cjs");

// 来自原生请求卡片的结构，以及没有业务标记的对照。验收只看像素、布局和交互。
const composer = '<div id="composer" data-composer-surface-variant="default" data-composer-radius-variant="default"><div id="editor" class="ProseMirror" contenteditable="true">Draft</div></div>';
const card = (id, attributes = "", classes = "", contents = "") =>
  `<div id="${id}" class="card ${classes}" ${attributes}><span>${id}</span>${contents}</div>`;
const family = () => [
  card("question", 'data-codex-composer-request-navigation="true"', "@container/request-card", '<div contenteditable="true">Reply</div>'),
  card("approval", 'data-codex-approval-surface="true"', "@container/approval-card", '<button id="action">Diagnostic action</button>'),
  card("form", "", "@container/request-card", '<form data-codex-character-input-boundary><input aria-label="Field" value="Draft"></form>'),
  card("auth", "", "", '<input aria-label="Account" value="Example">'),
  card("unknown", "", "", `<img id="native-photo" src="${assets.photo}">`),
  card("gradient", "", "gradient"),
  '<div data-above-composer-portal><div data-in-progress-fixed-content><div class="pill-clip">' + card("pill") + '</div></div></div>',
].join("");
const style = `
  *{box-sizing:border-box}#stage{position:relative;margin:20px 40px;width:720px}
  #thread,#host{position:relative}#host.dialog{background:#222;padding:4px}
  .card{position:relative;height:58px;margin:6px 0;background:var(--card-color,#2d2d2d);
    border:1px solid #555;border-radius:16px;overflow:hidden;padding:5px 14px;color:white}
  .card.gradient{background-color:transparent;background-image:linear-gradient(90deg,#2d2d2d,#696969)}
  .card span{font-size:12px}.card img{position:absolute;right:24px;top:8px;width:24px;height:24px}
  .card button{position:absolute;right:100px;top:6px}
  .card input,.card [contenteditable]{position:absolute;top:4px;left:120px;max-width:160px;background:transparent;color:white}
  [class~="@container/request-card"],[class~="@container/approval-card"]{container-type:inline-size}
  .pill-clip{position:relative;z-index:10;overflow:hidden;border-radius:24px}
  #composer{position:relative;height:70px;margin-top:8px;padding:14px;background:#2d2d2d;border-radius:20px}
`;
const blend = (color, wall, opacity = .3) => color.map((value, i) => Math.round(value * (1 - opacity) + wall[i] * opacity));
const wallAt = (page, x) => x < page.viewportSize().width / 2 ? [200, 90, 40] : [40, 80, 200];
const samplePoints = async (page, selector, dy = .8) => {
  const [left, top, width, height] = await rect(page, selector);
  const middle = page.viewportSize().width / 2;
  assert.ok(left < middle - 40 && left + width > middle + 40, `${selector} crosses the wallpaper seam`);
  return [middle - 40, middle + 40].map(x => [x, top + height * dy]);
};
const expectBlend = async (page, actual, native, selector, label = selector, dy = .8) => {
  for (const [x, y] of await samplePoints(page, selector, dy))
    near(pixel(actual, x, y), blend(pixel(native, x, y), wallAt(page, x)), `${label} at ${x}`);
};
const expectNativeMedia = async (page, frame, selector, color) => {
  const [left, top, width, height] = await rect(page, selector);
  near(pixel(frame, left + width / 2, top + height / 2), color, `${selector} retains native color`);
};
const readyImages = page => page.evaluate(() => Promise.all([...document.images].map(image => image.decode())));
const disable = async page => {
  await page.evaluate(() => {
    window.__CODEX_PLUS_IMAGE_OVERLAY__.enabled = false;
    installCodexPlusImageOverlay();
  });
  await settle(page);
};
async function mount(page, host, markup, extraCss = "") {
  const contents = `<div id="cards">${markup}</div>${composer}`;
  await html(page, style + extraCss, `<div id="stage">${host === "footer"
    ? `<div id="thread" class="thread-scroll-container"><div id="host" data-thread-scroll-footer>${contents}</div></div>`
    : `<div id="host" ${host === "dialog" ? 'class="dialog" role="dialog"' : 'class="thread-scroll-container"'}>${contents}</div>`}</div>`);
  await readyImages(page);
}

for (const host of ["footer", "dialog", "timeline"]) for (const late of [false, true])
check(`${host} shared backgrounds cover unlabelled cards and gradients on ${late ? "late" : "initial"} mount`, async page => {
  await mount(page, host, family());
  const native = await shot(page);
  const ids = ["question", "approval", "form", "auth", "unknown", "gradient", "pill"];
  const geometry = await Promise.all(ids.map(id => rect(page, `#${id}`)));
  await page.evaluate(() => {
    window.nativeEditor = document.getElementById("editor");
    window.editorMutations = 0;
    window.editorObserver = new MutationObserver(records => { window.editorMutations += records.length; });
    window.editorObserver.observe(window.nativeEditor, { attributes: true, childList: true, subtree: true });
    document.getElementById("action").onclick = () => { window.clicked = true; };
  });
  if (late) await page.evaluate(() => { window.lateCards = document.getElementById("cards"); window.lateCards.remove(); });
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  if (late) {
    await page.evaluate(() => document.getElementById("host").insertBefore(window.lateCards, document.getElementById("composer")));
    await settle(page);
  }
  assert.deepEqual(await Promise.all(ids.map(id => rect(page, `#${id}`))), geometry, "native card geometry is unchanged");
  assert.ok(await page.evaluate(() => window.nativeEditor === document.getElementById("editor") && window.editorMutations === 0), "editor DOM remains owned by the native app");
  await page.evaluate(() => window.editorObserver.disconnect());
  const actual = await shot(page);
  for (const id of ids) await expectBlend(page, actual, native, `#${id}`);
  await expectNativeMedia(page, actual, "#native-photo", [220, 40, 50]);
  await page.locator("#action").click();
  assert.equal(await page.evaluate(() => window.clicked), true);
  await page.locator("#editor").fill("Kept draft");
  assert.equal(await page.locator("#editor").innerText(), "Kept draft");
  await disable(page);
  const restored = await shot(page);
  for (const id of ids) for (const [x, y] of await samplePoints(page, `#${id}`))
    near(pixel(restored, x, y), pixel(native, x, y), `${id} restores its native background`);
}, { width: 800, height: 700 });

check("ordinary card receives one wallpaper blend as its ancestor changes none to translate and scale then back", async page => {
  await mount(page, "footer", card("dynamic"),
    '#stage{transform-origin:0 0}#dynamic{height:140px}');
  const changes = ["none", "translateX(20px) scale(.9)", "translateX(-10px) scale(.85)", "none"];
  const native = [];
  for (const transform of changes) {
    await page.$eval("#stage", (node, transform) => { node.style.transform = transform; }, transform);
    native.push({ frame: await shot(page), geometry: await rect(page, "#dynamic") });
  }
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  for (const [index, transform] of changes.entries()) {
    await page.$eval("#stage", (node, transform) => { node.style.transform = transform; }, transform);
    await settle(page);
    assert.deepEqual(await rect(page, "#dynamic"), native[index].geometry, `native geometry under ${transform}`);
    const actual = await shot(page);
    await expectBlend(page, actual, native[index].frame, "#dynamic", `dynamic ${transform}`);
  }
  await disable(page);
  assert.equal(await page.$eval("#stage", node => getComputedStyle(node).zIndex), "auto", "native ancestor stacking is preserved");
  assert.equal(await page.$eval("#stage", node => getComputedStyle(node).transform), "none", "disable preserves latest native transform");
});

check("native occupied pseudo elements and absolute child containing blocks survive shared painting", async page => {
  await mount(page, "footer", '<div id="relative"><div id="static-card" class="card"><button id="absolute">Native action</button></div></div>', `
    #relative{position:relative;height:210px;padding:25px 0}#static-card{position:static;height:150px}
    #static-card::before{content:"before";position:absolute;top:4px;left:8px;color:#00ff00;pointer-events:none}
    #static-card::after{content:"after";position:absolute;bottom:4px;left:8px;color:#00ff00;pointer-events:none}
    #absolute{position:absolute;right:12px;top:4px;height:25px}
  `);
  const pseudo = () => page.$eval("#static-card", node => ["::before", "::after"].map(which => {
    const value = getComputedStyle(node, which);
    return [value.content, value.position, value.top, value.bottom, value.left, value.color];
  }));
  const originalPseudo = await pseudo(), geometry = await rect(page, "#absolute"), native = await shot(page);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  assert.deepEqual(await pseudo(), originalPseudo, "native pseudo elements remain intact");
  assert.deepEqual(await rect(page, "#absolute"), geometry, "painting does not create a new containing block");
  await expectBlend(page, await shot(page), native, "#static-card");
  await page.$eval("#absolute", node => { node.onclick = () => { window.clicked = true; }; });
  await page.locator("#absolute").click();
  assert.equal(await page.evaluate(() => window.clicked), true);
});

check("transformed host preserves ordinary external sibling ordering and clicks", async page => {
  await html(page, style + `
    #stage{position:absolute;left:0;top:0;height:400px;transform:translateX(10px);background:#303030}
    #sibling{position:absolute;left:100px;top:100px;width:180px;height:80px;z-index:10;background:#246824}
  `, `<div id="stage"><div class="thread-scroll-container"><div id="host" data-thread-scroll-footer>${composer}</div></div></div>
    <div id="sibling">Native sibling pane</div>`);
  const hit = () => page.evaluate(() => document.elementFromPoint(150, 130)?.id);
  const nativeGeometry = await Promise.all(["#stage", "#sibling"].map(selector => rect(page, selector)));
  assert.equal(await hit(), "sibling");
  await page.$eval("#sibling", node => { node.onclick = () => { window.siblingClicked = true; }; });
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  assert.equal(await hit(), "sibling", "ordinary external sibling keeps its native occlusion order");
  assert.deepEqual(await Promise.all(["#stage", "#sibling"].map(selector => rect(page, selector))), nativeGeometry);
  await page.mouse.click(150, 130);
  assert.equal(await page.evaluate(() => window.siblingClicked), true);
});

for (const initialTransform of ["none", "translateX(10px)"])
check(`popup without media keeps one wallpaper blend from ${initialTransform} through ancestor changes`, async page => {
  await html(page, `
    #outer{position:relative;margin:40px;width:700px;transform:${initialTransform}}
    #popup{position:relative}#popup-card{height:160px;background:#303030}
  `, '<div id="outer"><div id="popup" role="dialog"><div id="popup-card"></div></div></div>');
  const native = pixel(await shot(page), 300, 100);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  for (const transform of [initialTransform, "translateX(20px)", "none"]) {
    await page.$eval("#outer", (node, transform) => { node.style.transform = transform; }, transform);
    await settle(page);
    near(pixel(await shot(page), 300, 100), blend(native, wallAt(page, 300)),
      `popup without media owns one composition under ${transform}`);
  }
});

check("partial native gradient keeps transparent gaps free from repeated wallpaper", async page => {
  await mount(page, "footer", '<div id="gradient-parent"><div id="partial-gradient"></div></div>', `
    #gradient-parent{background:#303030}
    #partial-gradient{height:160px;background:linear-gradient(#555,#555) left top / 100px 100% no-repeat}
  `);
  const native = await shot(page);
  const nativeBackground = await page.$eval("#partial-gradient", node => getComputedStyle(node).backgroundImage);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  assert.equal(await page.$eval("#partial-gradient", node => getComputedStyle(node).backgroundImage), nativeBackground,
    "partially painted native gradient does not acquire a full-frame wallpaper");
  await expectBlend(page, await shot(page), native, "#partial-gradient", "transparent gaps reveal the parent wallpaper exactly once");
});

check("popup without media respects ancestor will-change transform composition", async page => {
  await html(page, `
    #outer{position:relative;margin:40px;width:700px;will-change:transform}
    #popup{position:relative}#popup-card{height:160px;background:#303030}
  `, '<div id="outer"><div id="popup" role="dialog"><div id="popup-card"></div></div></div>');
  const native = pixel(await shot(page), 300, 100);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  for (const willChange of ["transform", "auto", "transform", "auto"]) {
    await page.$eval("#outer", (node, value) => { node.style.willChange = value; }, willChange);
    await settle(page);
    near(pixel(await shot(page), 300, 100), blend(native, wallAt(page, 300)),
      `popup under will-change:${willChange} receives one wallpaper layer`);
  }
});

check("modern translucent colors retain native background images without local wallpaper", async page => {
  await mount(page, "footer", '<div id="alpha-parent"><div id="alpha-card"></div></div>', `
    #alpha-parent{background:#303030}#alpha-card{height:160px}
  `);
  const colors = ["color(srgb 1 1 1 / .5)", "oklch(.6 .1 40 / .4)", "rgb(80 80 80 / .5)"];
  const native = [];
  for (const color of colors) {
    await page.$eval("#alpha-card", (node, color) => { node.style.background = color; }, color);
    native.push(await page.$eval("#alpha-card", node => {
      const paint = getComputedStyle(node); return [paint.backgroundColor, paint.backgroundImage];
    }));
  }
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  for (const [index, color] of colors.entries()) {
    await page.$eval("#alpha-card", node => { node.style.background = "#505050"; });
    await settle(page);
    assert.notEqual(await page.$eval("#alpha-card", node => getComputedStyle(node).backgroundImage), "none",
      "opaque stage is painted before returning to native translucent ownership");
    await page.$eval("#alpha-card", (node, color) => { node.style.background = color; }, color);
    await settle(page);
    assert.deepEqual(await page.$eval("#alpha-card", node => {
      const paint = getComputedStyle(node); return [paint.backgroundColor, paint.backgroundImage];
    }), native[index], `${color} releases local wallpaper without rewriting native alpha`);
  }
});

check("shared gradient material responds to native hover and focus without double blending", async page => {
  await mount(page, "footer", card("interactive", "", "gradient", '<input id="focus-field" aria-label="Field">'), `
    #interactive{height:140px}#interactive:hover{background-image:linear-gradient(90deg,#505050,#787878)}
    #interactive:focus-within{background-image:linear-gradient(90deg,#707070,#a0a0a0)}
  `);
  const states = [
    async () => { await page.$eval("#focus-field", node => node.blur()); await page.mouse.move(790, 580); },
    async () => { await page.locator("#interactive").hover({ position: { x: 500, y: 80 } }); },
    async () => { await page.locator("#focus-field").focus(); },
  ];
  const native = [];
  for (const state of states) { await state(); native.push(await shot(page)); }
  await states[0]();
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  for (let index = 0; index < states.length; index++) {
    await states[index](); await settle(page);
    await expectBlend(page, await shot(page), native[index], "#interactive", `gradient state ${index}`);
  }
});

check("moving painted cards into a third party subtree releases local material styles", async page => {
  await mount(page, "footer", card("migrating", "", "gradient"), `
    #migrating{height:100px}#third-party{position:relative;height:150px}
  `);
  await page.$eval("#host", node => {
    const external = document.createElement("div"); external.id = "third-party";
    external.setAttribute("data-codex-plus-ext", "third-party"); node.append(external);
  });
  const nativeStyle = await page.$eval("#migrating", node => {
    const s = getComputedStyle(node); return [s.backgroundColor, s.backgroundImage, s.position, s.isolation];
  });
  const native = await shot(page);
  const originalPoints = await samplePoints(page, "#migrating");
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  await expectBlend(page, await shot(page), native, "#migrating");
  await page.evaluate(() => document.getElementById("third-party").append(document.getElementById("migrating")));
  await settle(page);
  assert.deepEqual(await page.$eval("#migrating", node => {
    const s = getComputedStyle(node); return [s.backgroundColor, s.backgroundImage, s.position, s.isolation];
  }), nativeStyle, "extension subtree regains native material ownership");
  const inside = await shot(page);
  // 同一渐变宽度，直接比较移动前后的原生局部采样，而非假定扩展应接受壁纸。
  for (const [index, [x, y]] of (await samplePoints(page, "#migrating")).entries())
    near(pixel(inside, x, y), pixel(native, ...originalPoints[index]), "foreign extension does not retain local wallpaper");
  await page.evaluate(() => document.getElementById("cards").append(document.getElementById("migrating")));
  await settle(page);
  await expectBlend(page, await shot(page), native, "#migrating", "returning to native host resumes painting");
});

check("unrelated streaming text and DOM changes do not rescan shared host backgrounds", async page => {
  await mount(page, "footer", card("stable"));
  await page.evaluate(() => {
    const stream = document.createElement("div"); stream.id = "unrelated-stream"; document.body.append(stream);
  });
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) {
      const p = document.createElement("p"); p.textContent = "Streaming text";
      document.getElementById("unrelated-stream").append(p);
      await new Promise(requestAnimationFrame);
    }
  }));
  for (const key of ["frames", "queries", "rectReads", "layerWrites", "styleCommits"])
    assert.equal(work[key], 0, `unrelated stream leaves backgrounds idle: ${key}`);
});

for (const host of ["footer", "dialog"]) check(`${host} semantic identity removal releases nested material until host returns`, async page => {
  await mount(page, host, card("identity-card", "", "gradient"));
  const native = await shot(page);
  const nativeBackground = await page.$eval("#identity-card", node => getComputedStyle(node).backgroundImage);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  await expectBlend(page, await shot(page), native, "#identity-card", "initial host material");
  const attribute = host === "footer" ? "data-thread-scroll-footer" : "role";
  await page.$eval("#host", (node, attribute) => node.removeAttribute(attribute), attribute);
  await settle(page);
  assert.equal(await page.$eval("#identity-card", node => getComputedStyle(node).backgroundImage), nativeBackground,
    "ordinary content releases local material rules and uses global wallpaper");
  await expectBlend(page, await shot(page), native, "#identity-card", "ordinary content does not double blend");
  await page.$eval("#host", (node, { attribute, host }) => node.setAttribute(attribute, host === "footer" ? "true" : "dialog"), { attribute, host });
  await settle(page);
  await expectBlend(page, await shot(page), native, "#identity-card", "host identity restores local material");
});

check("transparent option row gains and releases its native hover background inside a request card", async page => {
  // 原生选项/列表行通常只有 hover 或 focus-within 时出现背景；透明布局阶段不得重复叠图。
  await mount(page, "footer", card("request", "", "", '<div id="option" role="radio" tabindex="0"><span>Option</span></div>'), `
    #request{height:160px;padding:20px}#option{height:90px;background:transparent;border-radius:12px}
    #option:hover{background:#505050}#option:focus{background:#707070}
  `);
  const states = [
    async () => { await page.$eval("#option", node => node.blur()); await page.mouse.move(790, 580); },
    async () => { await page.locator("#option").hover({ position: { x: 450, y: 65 } }); },
    async () => { await page.locator("#option").focus(); },
    async () => { await page.$eval("#option", node => node.blur()); await page.mouse.move(790, 580); },
  ];
  const native = [];
  for (const state of states) { await state(); native.push(await shot(page)); }
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  for (const [index, state] of states.entries()) {
    await state(); await settle(page);
    await expectBlend(page, await shot(page), native[index], "#option", `native option state ${index}`);
  }
});

for (const host of ["footer", "dialog"]) check(`${host} nested opaque and transparent cards each receive one wallpaper blend`, async page => {
  await mount(page, host, '<div id="outer" class="card"><div id="transparent"><div id="inner"></div></div></div>', `
    #outer{height:220px;padding:20px}#transparent{height:140px;background:transparent;padding:20px}
    #inner{height:70px;border-radius:12px;background:#3c3c3c}
  `);
  const native = await shot(page);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const frame = await shot(page);
  await expectBlend(page, frame, native, "#outer", "outer background", .9);
  await expectBlend(page, frame, native, "#transparent", "transparent layout reveals its parent once", .9);
  await expectBlend(page, frame, native, "#inner", "inner opaque background", .5);
  await page.$eval("#inner", node => { node.style.background = "transparent"; });
  await settle(page);
  const transparent = await shot(page);
  for (const [x, y] of await samplePoints(page, "#inner", .5))
    near(pixel(transparent, x, y), blend([45, 45, 45], wallAt(page, x)), "solid to transparent removes stale local paint");
  await page.$eval("#inner", node => { node.style.background = "#505050"; });
  await settle(page);
  const solid = await shot(page);
  for (const [x, y] of await samplePoints(page, "#inner", .5))
    near(pixel(solid, x, y), blend([80, 80, 80], wallAt(page, x)), "transparent to solid registers its own background");
});

check("unlabelled card background follows host moves, native theme changes and disable", async page => {
  await html(page, style + `
    #stage{height:560px}#timeline,#footer,#dialog{position:absolute;left:0;right:0;height:170px}
    #timeline{top:0}#footer{top:180px}#dialog{top:360px;background:#222}#composer{position:absolute;bottom:0;left:0;right:0}
    :root{--card-color:#2d2d2d}.light{--card-color:#dcdcdc}
  `, `<div id="stage"><div id="timeline" class="thread-scroll-container">${card("moving", "", "", `<img id="moving-photo" src="${assets.photo}">`)}</div><div id="footer" data-thread-scroll-footer>${composer}</div><div id="dialog" role="dialog"></div></div>`);
  await readyImages(page);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const aligned = async (label, base) => {
    const frame = await shot(page);
    for (const [x, y] of await samplePoints(page, "#moving"))
      near(pixel(frame, x, y), blend([base, base, base], wallAt(page, x)), label);
    await expectNativeMedia(page, frame, "#moving-photo", [220, 40, 50]);
  };
  await aligned("ordinary timeline uses global wallpaper", 45);
  for (const host of ["footer", "dialog", "timeline", "footer"]) {
    await page.evaluate(host => document.getElementById(host).prepend(document.getElementById("moving")), host);
    await settle(page);
    await aligned(`moved to ${host}`, 45);
  }
  await page.evaluate(() => document.documentElement.classList.add("light"));
  await settle(page);
  await aligned("ancestor theme changes existing background", 220);
  await page.$eval("#moving", node => { node.style.background = "#505050"; });
  await settle(page);
  await aligned("inline native background changes", 80);
  const idle = await delta(page, () => page.waitForTimeout(180));
  for (const key of ["rectReads", "styleReads", "layerWrites", "markerWrites", "styleCommits"])
    assert.equal(idle[key], 0, `settled host surfaces do not poll: ${key}`);
  await disable(page);
  const restored = await shot(page);
  for (const [x, y] of await samplePoints(page, "#moving")) near(pixel(restored, x, y), [80, 80, 80], "disable retains the latest native background");
  assert.equal(await page.evaluate(() => [...document.querySelectorAll("*")].flatMap(node => [...node.attributes])
    .filter(attribute => attribute.name.startsWith("data-codex-plus-image")).length), 0, "disable removes all overlay annotations");
});

