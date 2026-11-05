const { assert, assets, check, configure, css, delta, html, near, pixel, rect, settle, shot } = require("./image-overlay-fixture.cjs");

// 真实 Codex DOM / 材质来自 2026-11-05 的现场诊断；不依赖扩展内部的登记属性。
const composerAttributes = 'data-composer-surface-variant="default" data-composer-radius-variant="default"';
const materialClass = "capsule-uG_Zor controlSurface-VUAmap";
const materialCss = `
  .native-material{background-color:transparent;background-image:linear-gradient(rgb(45,45,45),rgb(45,45,45));
    backdrop-filter:blur(24px);border:0;border-radius:9999px;color:white;
    box-shadow:0 0 0 .5px #ffffff15,0 8px 16px -4px #0000001f}
  #toolbar{display:flex;align-items:center;gap:12px;padding:16px;box-sizing:border-box;height:76px;background:#222}
  #navigation{display:flex;width:100px;height:44px;flex-shrink:0}
  #navigation button{width:44px;background:transparent;border:0;color:inherit}
  #address{position:relative;isolation:isolate;display:flex;align-items:center;gap:10px;flex:1;min-width:0;height:44px;overflow:hidden}
  #address input{background:transparent;border:0;color:inherit;width:160px;outline:none}
  #address img{width:16px;height:16px;margin-left:14px}
  #control{position:relative;isolation:isolate;display:flex;align-items:center;justify-content:center;width:48px;height:44px;flex-shrink:0}
  #control:hover{background-image:linear-gradient(rgb(65,65,65),rgb(65,65,65))}
  #address::after,#control::after{content:"";position:absolute;inset:0;border-radius:inherit;pointer-events:none;background:transparent}
  #address:hover::after,#address:focus-within::after,#control:hover::after{background-color:rgba(255,255,255,.06)}
  #address:focus-within{outline:2px solid #55cc88;outline-offset:-2px}
`;
const toolbarMarkup = contents => `<div id="toolbar" data-browser-sidebar-toolbar>
  <div id="navigation" class="${materialClass} native-material" data-menu-overlay-surface="true">${contents ? '<button id="back" aria-label="Back">←</button>' : ""}</div>
  <div id="address" class="${materialClass} inputSurface-DKfVc_ native-material" data-menu-overlay-surface="true">${contents ? `<img id="favicon" src="${assets.photo}"><input id="url" aria-label="Address" value="example.test">` : ""}</div>
  <button id="control" class="button-p4QQIY controlSurface-VUAmap native-material" data-browser-sidebar-skip-address-commit="true" aria-label="Browser menu">${contents ? '<svg id="toolbar-icon" width="12" height="12"><rect width="12" height="12" fill="#00ff00"/></svg>' : ""}</button>
</div>`;
const sample = async (page, frame, selector, dx = .5, dy = .5) => {
  const [x, y, width, height] = await rect(page, selector);
  return pixel(frame, x + width * dx, y + height * dy);
};

// 旧版固定 DIV 只作为空白材质的像素参考，媒体仍与未开启壁纸的原生截图比较。
async function reference(page, dataUrl, opacity = .3) {
  await page.evaluate(({ dataUrl, opacity }) => {
    const layer = document.createElement("div");
    layer.id = "native-surface-reference";
    Object.assign(layer.style, { position: "fixed", inset: "0", width: "100vw", height: "100vh",
      backgroundImage: `url("${dataUrl}")`, backgroundSize: "100% 100%", opacity: String(opacity),
      zIndex: "2147483646", pointerEvents: "none" });
    document.documentElement.append(layer);
  }, { dataUrl, opacity });
  const frame = await shot(page);
  await page.$eval("#native-surface-reference", node => node.remove());
  return frame;
}

for (const variant of ["current", "legacy"]) check(`${variant} composer blends wallpaper while preserving editor ownership and attachments`, async page => {
  await html(page, `
    #thread,#footer{position:relative}
    #composer{position:relative;display:flex;flex-direction:column;margin:40px;width:650px;height:180px;
      background:#363636;border-radius:22px;overflow:auto;box-shadow:inset 0 0 1px #ffffff33}
    #editor{padding:20px;min-height:30px}#attachment{position:relative;width:48px;height:48px;margin-left:20px}
    #attachment img{width:48px;height:48px}#remove{position:absolute;right:0;top:0;width:16px;height:16px;padding:0}
  `, `<div id="thread" class="thread-scroll-container"><div id="footer" data-thread-scroll-footer><div id="composer" class="${variant === "current" ? "ComposerLayoutRoot-XCKS7O" : "_ComposerLayoutRoot_test"}" ${variant === "current" ? composerAttributes : ""}>
    <div id="editor" class="ProseMirror" contenteditable="true">Draft <span contenteditable="false">Plugin</span></div>
    <div id="attachment"><img id="attachment-photo" src="${assets.photo}"><button id="remove" aria-label="Remove attachment">×</button></div>
  </div></div></div>`);
  const native = await shot(page), expected = await reference(page, assets.pattern);
  const geometry = await rect(page, "#composer");
  await page.evaluate(() => {
    window.editorNode = document.getElementById("editor");
    window.editorContent = window.editorNode.innerHTML;
    window.editorMutations = 0;
    window.editorObserver = new MutationObserver(records => { window.editorMutations += records.length; });
    window.editorObserver.observe(window.editorNode, { attributes: true, childList: true, subtree: true });
    document.getElementById("remove").onclick = () => document.getElementById("attachment").remove();
  });
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  assert.deepEqual(await rect(page, "#composer"), geometry, "native composer geometry is unchanged");
  assert.ok(await page.evaluate(() => window.editorNode === document.getElementById("editor") &&
    window.editorContent === window.editorNode.innerHTML && window.editorMutations === 0), "editor owns its DOM");
  const actual = await shot(page);
  near(await sample(page, actual, "#attachment-photo"), await sample(page, native, "#attachment-photo"), "attachment stays native");
  await page.locator("#remove").click();
  assert.equal(await page.locator("#attachment").count(), 0, "native attachment removal stays clickable");
  await page.evaluate(() => window.editorObserver.disconnect());
  for (const x of [380, 420]) near(pixel(actual, x, 190), pixel(expected, x, 190), `${variant} composer wallpaper at ${x}`);
});

check("current home composer keeps utility layout and its above/below material behavior", async page => {
  await html(page, `
    #composer{position:relative;margin:30px;width:650px}
    #body{position:relative;height:140px;background:#363636;border-radius:22px}
    #actions{position:relative;height:60px;background:#282828;border-radius:0 0 22px 22px}
    #project{margin:12px}#body img{width:24px;height:24px;margin:18px}
  `, `<div id="composer" class="ComposerLayoutRoot-XCKS7O" ${composerAttributes} data-composer-utility-bar-variant="home">
    <div id="body" class="ComposerLayoutBody-XCKS7O" data-composer-body><img id="home-photo" src="${assets.photo}"><div contenteditable="true">Draft</div></div>
    <div id="placement" class="top-full" style="transform:translateY(0)"><div id="actions" data-composer-placement="home" class="bg-background-composer-action-bar"><button id="project">Project</button></div></div>
  </div>`);
  const geometry = await rect(page, "#actions");
  await page.$eval("#project", node => { node.onclick = () => { window.projectClicked = true; }; });
  await configure(page, { opacity: .3 });
  const frame = await shot(page);
  assert.deepEqual(await rect(page, "#actions"), geometry, "home utility bar retains native geometry");
  await page.locator("#project").click();
  assert.ok(await page.evaluate(() => window.projectClicked), "home project action remains clickable");
  near(await sample(page, frame, "#home-photo"), [220, 40, 50], "home attachment remains native");
  near(pixel(frame, 450, 210), pixel(frame, 450, 260), "below-composer utility reveals viewport wallpaper");
  await page.$eval("#placement", node => { node.className = "bottom-full"; });
  await settle(page);
  near(pixel(await shot(page), 450, 210), [40, 40, 40], "above-composer project material stays native");
  assert.deepEqual(await rect(page, "#actions"), geometry, "placement changes do not rewrite native layout");
});

check("browser address, navigation and button capsules blend their native gray material", async page => {
  await html(page, materialCss, toolbarMarkup(false));
  const native = await shot(page), expected = await reference(page, assets.wall);
  const geometry = await Promise.all(["#navigation", "#address", "#control"].map(selector => rect(page, selector)));
  await configure(page, { opacity: .3 });
  const actual = await shot(page);
  // 三类控件均应保留 45 灰底的强度；仅清空背景会得到错误的、更暗的像素。
  const comparisons = [];
  for (const selector of ["#navigation", "#address", "#control"]) {
    near(await sample(page, native, selector), [45, 45, 45], `${selector} fixture contains opaque native gradient`);
    comparisons.push({ selector, actual: await sample(page, actual, selector), expected: await sample(page, expected, selector) });
  }
  assert.deepEqual(await Promise.all(["#navigation", "#address", "#control"].map(selector => rect(page, selector))), geometry);
  assert.ok(comparisons.every(({ actual, expected }) => actual.every((value, i) => Math.abs(value - expected[i]) <= 1)),
    "native toolbar materials must match the old wallpaper blend: " + JSON.stringify(comparisons));
});

check("toolbar keeps native media, scoped surfaces, hover, focus and clicks, then disables cleanly", async page => {
  await html(page, materialCss + `
    #web{height:120px;background:#dc2832;padding:20px}#outside{margin:24px}
    .foreign-control{display:block;width:100px;height:44px}
  `, toolbarMarkup(true) + `<div id="web" data-browser-sidebar-webview>
    <button id="guest-control" class="${materialClass} native-material foreign-control" data-menu-overlay-surface="true"></button>
    <img id="guest-photo" src="${assets.photo}" width="24" height="24">
  </div><div id="outside"><button id="outside-control" class="${materialClass} native-material foreign-control" data-menu-overlay-surface="true"></button></div>`);
  const native = await shot(page);
  const properties = ["backgroundColor", "backgroundImage", "backdropFilter", "borderRadius", "boxShadow", "position", "isolation"];
  const selectors = ["#toolbar", "#navigation", "#address", "#control", "#guest-control", "#outside-control"];
  const originalStyles = await Promise.all(selectors.map(selector => css(page, selector, properties)));
  const nativePseudo = () => page.$eval("#address", node => {
    const style = getComputedStyle(node, "::after");
    return [style.content, style.backgroundImage, style.position, style.pointerEvents];
  });
  const originalPseudo = await nativePseudo();
  await page.evaluate(() => {
    window.clicks = [];
    for (const id of ["back", "control"]) document.getElementById(id).onclick = () => window.clicks.push(id);
  });
  await configure(page, { opacity: .3 });
  const actual = await shot(page);
  for (const selector of ["#favicon", "#toolbar-icon", "#guest-control", "#guest-photo"])
    near(await sample(page, actual, selector), await sample(page, native, selector), `${selector} retains native pixels`);
  assert.deepEqual(await css(page, "#outside-control", properties), originalStyles[5], "arbitrary controls are outside toolbar material handling");
  assert.deepEqual(await css(page, "#guest-control", properties), originalStyles[4], "embedded web content is outside toolbar material handling");
  assert.deepEqual(await nativePseudo(), originalPseudo, "occupied native state pseudo-element is preserved");
  await page.locator("#back").click();
  await page.locator("#control").click();
  assert.deepEqual(await page.evaluate(() => window.clicks), ["back", "control"]);
  await page.locator("#url").fill("changed.test");
  assert.equal(await page.locator("#url").inputValue(), "changed.test");
  assert.equal(await page.evaluate(() => document.activeElement.id), "url");
  assert.deepEqual(await css(page, "#address", ["outlineColor", "outlineWidth"]), ["rgb(85, 204, 136)", "2px"], "native focus ring remains visible");
  assert.equal(await page.$eval("#address", node => getComputedStyle(node, "::after").backgroundColor), "rgba(255, 255, 255, 0.06)", "native focus material remains active");
  await page.locator("#control").hover();
  const hovered = await shot(page);
  const restingColor = await sample(page, actual, "#control", .2), hoverColor = await sample(page, hovered, "#control", .2);
  assert.ok(hoverColor.every((value, i) => value > restingColor[i] + 5), "native hover material still changes the rendered surface");
  assert.equal(await page.$eval("#control", node => getComputedStyle(node, "::after").backgroundColor), "rgba(255, 255, 255, 0.06)", "native hover pseudo-element is preserved");
  await page.$eval("#control", node => { node.disabled = true; });
  const [x, y, width, height] = await rect(page, "#control");
  await page.mouse.click(x + width / 2, y + height / 2);
  assert.deepEqual(await page.evaluate(() => window.clicks), ["back", "control"], "disabled native button does not activate");
  await page.mouse.move(900, 450);
  await page.$eval("#control", node => { node.disabled = false; });
  await page.$eval("#url", node => node.blur());
  await page.evaluate(() => {
    window.__CODEX_PLUS_IMAGE_OVERLAY__.enabled = false;
    installCodexPlusImageOverlay();
  });
  await settle(page);
  assert.deepEqual(await Promise.all(selectors.map(selector => css(page, selector, properties))), originalStyles, "disabling restores native material and layout styles");
  assert.equal(await page.evaluate(() => [...document.querySelectorAll("*")].flatMap(node => [...node.attributes])
    .filter(attribute => attribute.name.startsWith("data-codex-plus-image")).length), 0, "disabling removes every image-overlay marker");
  const disabled = await shot(page);
  for (const selector of ["#navigation", "#control", "#favicon", "#toolbar-icon", "#guest-control"])
    near(await sample(page, disabled, selector), await sample(page, native, selector), `${selector} restores native pixels`);
}, { width: 1000, height: 600 });

check("late native surfaces stay viewport aligned after sidebar transforms, resize and DOM moves", async page => {
  await html(page, materialCss + `
    #stage{position:absolute;left:154px;top:30px;width:calc(100vw - 180px)}
    #toolbar,#footer{transform:translateX(-104px) scale(.9);transform-origin:0 0}
    #thread,#footer{position:relative}
    #composer{position:relative;margin-top:30px;height:140px;background:#363636;border-radius:22px}
    #composer img{position:absolute;left:18px;top:18px;width:24px;height:24px}
    #web{position:absolute;left:30px;top:420px;width:600px;height:100px;background:#dc2832;padding:10px}
  `, '<div id="stage"></div><div id="web" data-browser-sidebar-webview></div>');
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const initialGeometry = await page.evaluate(({ toolbar, composerAttributes, photo }) => {
    // 无类名的输入框证明 data 属性可独立识别；原生控件在安装后才挂载。
    // 变换归属原生 footer/toolbar；普通 transformed 祖先的整体层级提升是 v2.3.0 已有的独立限制。
    document.getElementById("stage").innerHTML = toolbar + `<div id="thread" class="thread-scroll-container"><div id="footer" data-thread-scroll-footer><div id="composer" ${composerAttributes}><img id="moving-photo" src="${photo}"></div></div></div>`;
    return ["composer", "toolbar", "address"].map(id => {
      const r = document.getElementById(id).getBoundingClientRect(); return [r.x, r.y, r.width, r.height];
    });
  }, { toolbar: toolbarMarkup(false), composerAttributes, photo: assets.photo });
  await settle(page);
  assert.deepEqual(await Promise.all(["#composer", "#toolbar", "#address"].map(selector => rect(page, selector))), initialGeometry);
  const aligned = async label => {
    const frame = await shot(page), midpoint = page.viewportSize().width / 2;
    for (const [selector, base] of [["#composer", 54], ["#address", 45]]) {
      const [left, top, width, height] = await rect(page, selector);
      assert.ok(left < midpoint - 20 && left + width > midpoint + 20, `${selector} crosses wallpaper boundary`);
      for (const [x, wall] of [[midpoint - 20, [200, 90, 40]], [midpoint + 20, [40, 80, 200]]])
        near(pixel(frame, x, top + height / 2), wall.map(value => Math.round(base * .7 + value * .3)), `${label} ${selector} at ${x}`);
    }
    near(await sample(page, frame, "#moving-photo"), [220, 40, 50], `${label} native attachment`);
  };
  await aligned("late insertion");
  await page.$eval("#stage", node => { node.style.left = "230px"; });
  await page.evaluate(() => {
    for (const id of ["footer", "toolbar"]) document.getElementById(id).style.transform = "translateX(-100px) scale(.85)";
  });
  await page.$eval("#composer", node => { node.style.height = "210px"; });
  await page.setViewportSize({ width: 1000, height: 650 });
  await settle(page);
  await aligned("sidebar placement and viewport resize");
  await page.evaluate(() => document.getElementById("web").append(document.getElementById("address")));
  await settle(page);
  near(await sample(page, await shot(page), "#address"), [45, 45, 45], "material moved into web content becomes native again");
  await page.evaluate(() => document.getElementById("toolbar").insertBefore(document.getElementById("address"), document.getElementById("control")));
  await settle(page);
  await aligned("material moved back into toolbar");
  const work = await delta(page, () => page.waitForTimeout(180));
  for (const key of ["rectReads", "styleReads", "layerWrites", "markerWrites", "styleCommits"])
    assert.equal(work[key], 0, `settled native surfaces do not poll: ${key}`);
});

check("existing capsules follow toolbar and outer material semantic changes", async page => {
  await html(page, materialCss, toolbarMarkup(false).replace(" data-browser-sidebar-toolbar", ""));
  const materialProperties = ["backgroundImage", "backdropFilter", "isolation", "position"];
  const nativeMaterial = await css(page, "#address", materialProperties);
  await configure(page, { opacity: .3 });
  await page.$eval("#toolbar", node => node.setAttribute("data-browser-sidebar-toolbar", ""));
  await settle(page);
  const blended = async label => {
    const frame = await shot(page);
    for (const selector of ["#navigation", "#address", "#control"])
      near(await sample(page, frame, selector), [44, 56, 92], `${label} ${selector}`);
  };
  await blended("late toolbar designation");
  await page.$eval("#address", node => {
    const wrapper = document.createElement("div");
    wrapper.id = "material-wrapper";
    wrapper.style.cssText = "display:contents";
    wrapper.setAttribute("data-menu-overlay-surface", "true");
    node.replaceWith(wrapper);
    wrapper.append(node);
  });
  await settle(page);
  await page.$eval("#material-wrapper", node => node.removeAttribute("data-menu-overlay-surface"));
  await settle(page);
  await blended("inner capsule promoted after outer material removal");
  await page.$eval("#toolbar", node => node.removeAttribute("data-browser-sidebar-toolbar"));
  await settle(page);
  assert.deepEqual(await css(page, "#address", materialProperties), nativeMaterial, "removing toolbar designation restores native capsule styles");
  assert.equal(await page.$eval("#address", node => node.hasAttribute("data-codex-plus-image-paint")), false, "former toolbar material is no longer locally painted");
  await page.$eval("#toolbar", node => node.setAttribute("data-browser-sidebar-toolbar", ""));
  await settle(page);
  await blended("toolbar designation restored");
});

check("toolbar newly transformed by inline style keeps the viewport wallpaper fixed", async page => {
  await html(page, materialCss + "#toolbar{position:relative;width:700px}", toolbarMarkup(false));
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  const aligned = async label => {
    const frame = await shot(page), [, top, , height] = await rect(page, "#address");
    near(pixel(frame, 380, top + height / 2), [92, 59, 44], `${label} left wallpaper`);
    near(pixel(frame, 420, top + height / 2), [44, 56, 92], `${label} right wallpaper`);
  };
  await aligned("initial fixed paint");
  await page.$eval("#toolbar", node => { node.style.transform = "translateX(100px)"; });
  await settle(page);
  await aligned("transform added without a resize event");
  await page.$eval("#toolbar", node => { node.style.transform = ""; });
  await settle(page);
  await aligned("transform removed without a resize event");
  const work = await delta(page, () => page.waitForTimeout(180));
  for (const key of ["rectReads", "styleReads", "layerWrites", "markerWrites", "styleCommits"])
    assert.equal(work[key], 0, `semantic and geometry updates settle without polling: ${key}`);
});
