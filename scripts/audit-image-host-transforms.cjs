// 独立审计：额外共享祖先变换下的完整背景、媒体、裁切和交互契约。
// 这些原始断言完整保留；当前仍有已知失败，运行失败必须报告，不能作为正式套件全绿的一部分。
// 运行：node scripts/audit-image-host-transforms.cjs

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

check("shared ancestor changes none to translate and scale then back without stale composition", async page => {
  await mount(page, "footer", card("dynamic", "", "", `<img id="dynamic-photo" src="${assets.photo}">`),
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
    await expectBlend(page, actual, native[index].frame, "#composer", `composer ${transform}`);
    await expectNativeMedia(page, actual, "#dynamic-photo", [220, 40, 50]);
  }
  await disable(page);
  assert.equal(await page.$eval("#stage", node => getComputedStyle(node).zIndex), "auto", "disable releases ancestor promotion");
  assert.equal(await page.$eval("#stage", node => getComputedStyle(node).transform), "none", "disable preserves latest native transform");
});

check("transformed content keeps external header and popup ordering plus native clipping", async page => {
  await html(page, style + `
    #stage{margin-top:50px;transform:translateX(10px) scale(.9);transform-origin:0 0}
    #host{height:360px}#large{height:330px}#header{position:fixed;left:0;top:0;width:800px;height:120px;z-index:30;background:#303030}
    #header-action{position:absolute;left:200px;top:65px;width:100px;height:30px}
    #popup{position:fixed;left:330px;top:170px;width:160px;height:90px;background:#505050}
    #popup-action{position:absolute;inset:20px;width:100px;height:30px}
    #clip{position:absolute;left:100px;top:160px;width:100px;height:80px;overflow:hidden;border:0;background:#222}
    #clipped-photo{position:absolute;left:80px;top:5px;width:60px;height:60px}
  `, `<div id="stage"><div id="host" data-thread-scroll-footer>${card("large", "", "", '<button id="inside">Inside</button>')}
    <div id="clip"><img id="clipped-photo" src="${assets.photo}"></div>${composer}</div></div>
    <div id="header" data-app-shell-header-layout="thread"><button id="header-action">Header</button></div>
    <div class="fixed inset-0 pointer-events-none zoom-adjusted-viewport contain-layout" style="position:fixed;inset:0;z-index:60;pointer-events:none">
      <div id="popup" role="menu" style="pointer-events:auto"><button id="popup-action">Popup</button></div></div>`);
  await readyImages(page);
  const original = await Promise.all(["#stage", "#large", "#clip", "#clipped-photo", "#header", "#popup"].map(selector => rect(page, selector)));
  const hit = point => page.evaluate(([x, y]) => document.elementFromPoint(x, y)?.id, point);
  const expectedHeader = await hit([250, 80]), expectedPopup = await hit([380, 205]);
  const clippedBox = await rect(page, "#clip");
  const clipPoint = [clippedBox[0] + clippedBox[2] + 10, clippedBox[1] + 20];
  const nativeClippedPixel = pixel(await shot(page), ...clipPoint);
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  assert.deepEqual(await Promise.all(["#stage", "#large", "#clip", "#clipped-photo", "#header", "#popup"].map(selector => rect(page, selector))), original);
  assert.equal(await hit([250, 80]), expectedHeader, "native header remains above transformed content");
  assert.equal(await hit([380, 205]), expectedPopup, "native external popup remains above transformed content");
  near(pixel(await shot(page), ...clipPoint), blend(nativeClippedPixel, wallAt(page, clipPoint[0])), "promoted media does not escape native clipping");
  await page.evaluate(() => {
    window.clicks = [];
    for (const id of ["header-action", "popup-action", "inside"]) document.getElementById(id).onclick = () => window.clicks.push(id);
  });
  await page.locator("#header-action").click();
  await page.locator("#popup-action").click();
  await page.$eval("#inside", node => { node.style.top = "260px"; });
  await page.locator("#inside").click();
  assert.deepEqual(await page.evaluate(() => window.clicks), ["header-action", "popup-action", "inside"]);
});

// 独立的既有层级风险：此用例失败不能以其他卡片通过来掩盖，也不是业务标记遗漏。
check("transformed ancestor composition keeps shared surfaces single-blended and native media unmodified", async page => {
  await mount(page, "footer", family(), '#stage{transform:translateX(10px) scale(.9);transform-origin:0 0}');
  const native = await shot(page);
  const geometry = await rect(page, "#stage");
  await configure(page, { dataUrl: assets.pattern, opacity: .3 });
  assert.deepEqual(await rect(page, "#stage"), geometry, "native ancestor transform is preserved");
  const frame = await shot(page);
  for (const id of ["unknown", "gradient", "composer"]) await expectBlend(page, frame, native, `#${id}`);
  await expectNativeMedia(page, frame, "#native-photo", [220, 40, 50]);
}, { width: 800, height: 700 });
