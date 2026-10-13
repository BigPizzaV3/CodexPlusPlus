const fs = require("node:fs");
const path = require("node:path");
const { assert, check, configure: applyOverlay, injection, near, pixel, sharp, shot } = require("./image-overlay-fixture.cjs");
const out = process.env.CODEX_OVERLAY_TEST_OUTPUT;
const configure = (page, dataUrl) => applyOverlay(page, { dataUrl });
const rect = (page, id) => page.$eval(id, node => {
  const r = node.getBoundingClientRect();
  return { x: r.x, y: r.y, width: r.width, height: r.height };
});
async function main() {
  const wall = `data:image/png;base64,${(await sharp({ create: { width: 80, height: 80, channels: 4, background: "#2850c8" } }).png().toBuffer()).toString("base64")}`;
  const opaque = `data:image/png;base64,${(await sharp({ create: { width: 80, height: 80, channels: 4, background: "#dc2832" } }).png().toBuffer()).toString("base64")}`;
  const iconSvg = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80"><rect x="24" y="24" width="32" height="32" fill="#dc2832"/><rect x="8" y="24" width="8" height="32" fill="#dc2832" opacity=".5"/></svg>');
  const assets = {};
  for (const format of ["png", "gif"])
    assets[format] = `data:image/${format};base64,${(await sharp(iconSvg).toFormat(format).toBuffer()).toString("base64")}`;
  assets.svg = `data:image/svg+xml;base64,${iconSvg.toString("base64")}`;

  check("actual-app-media-containment-and-current-viewer", async page => {
    await page.setContent(`<style>
      body{margin:0;background:#181818}
      #frame{position:relative;container-type:inline-size;width:650px;height:580px}
      #thread{container-type:inline-size;width:600px;height:250px;overflow:auto}
      #content{height:700px;padding:30px;box-sizing:border-box}
      #composer{position:relative;width:600px;height:200px;container-type:inline-size}
      #attachments{transform:translateX(0px);width:180px;height:120px;margin:20px}
      img{width:80px;height:80px;border-radius:12px;object-fit:cover}
      #preview-backdrop{display:none;position:fixed;inset:0;z-index:50;background:#0009;backdrop-filter:blur(6px)}
      #preview{display:none;position:fixed;inset:0;z-index:50}
      #preview img{position:absolute;left:260px;top:180px;width:240px;height:240px}
      #close{position:absolute;right:30px;top:30px;width:40px;height:40px;background:white}
      </style><div id="frame"><div id="thread"><div id="content">
      <img id="transcript" src="${opaque}"></div></div>
      <div id="composer"><div id="attachments"><img id="attachment" src="${opaque}"></div></div></div>
      <div id="preview-backdrop" class="codex-dialog-overlay _imageOverlay_zt97a_2"></div>
      <div id="preview" class="codex-dialog _chrome_abc_2" role="dialog" data-state="open">
      <div data-image-preview-toolbar><button id="close">X</button></div><img id="viewed" src="${opaque}"></div>`);
    const nativeBoxes = await page.evaluate(() => ["transcript", "attachment"].map(id => {
      const r = document.getElementById(id).getBoundingClientRect();
      return [r.x, r.y, r.width, r.height];
    }));
    await configure(page, wall);
    for (const id of ["transcript", "attachment"]) {
      const r = await rect(page, `#${id}`);
      near(pixel(await shot(page), r.x + r.width / 2, r.y + r.height / 2), [220, 40, 50], `${id} native ink through container ancestors`);
    }
    assert.deepEqual(await page.evaluate(() => ["transcript", "attachment"].map(id => {
      const r = document.getElementById(id).getBoundingClientRect();
      return [r.x, r.y, r.width, r.height];
    })), nativeBoxes, "container repair preserves native layout");
    await page.evaluate(() => {
      window.nativeRAF = requestAnimationFrame;
      window.requestAnimationFrame = () => 999999;
      document.querySelector("#preview").style.display = "block";
      document.querySelector("#preview-backdrop").style.display = "block";
      document.querySelector("#close").onclick = () => { window.closeClicked = true; };
    });
    await page.waitForTimeout(50);
    assert.equal(await page.$eval("#codex-plus-image-overlay", node => getComputedStyle(node).visibility), "visible", "wallpaper remains below the actual native viewer");
    assert.ok(await page.evaluate(() => Number(getComputedStyle(document.getElementById("preview")).zIndex) >
      Number(getComputedStyle(document.getElementById("transcript")).zIndex)), "complete viewer is above thread media");
    assert.equal(await page.$eval("#viewed", node => getComputedStyle(node).zIndex), "auto", "viewer media not lifted over its controls");
    const reference = await page.evaluate(() => {
      const style = [...document.querySelectorAll('style[data-codex-plus-ext="image-overlay"]')][0];
      return style.textContent;
    });
    assert.ok(reference.length, "ordinary page layering remains active beneath viewer");
    await page.locator("#close").click();
    assert.ok(await page.evaluate(() => window.closeClicked), "native viewer button receives click");
    const shown = await shot(page);
    near(pixel(shown, 380, 300), [220, 40, 50], "viewer stays opaque");
    const beforeMotion = await page.evaluate(() => ({ ...window.__codexPlusImageOverlayStats }));
    await page.evaluate(async () => {
      for (let i = 0; i < 10; i++) {
        document.querySelector("#viewed").style.transform = `scale(${1 + i / 100})`;
        await new Promise(resolve => window.nativeRAF(resolve));
      }
    });
    const afterMotion = await page.evaluate(() => ({ ...window.__codexPlusImageOverlayStats }));
    assert.equal(afterMotion.rectReads, beforeMotion.rectReads, "native viewer motion needs no injected layout reads");
    assert.equal(afterMotion.styleReads, beforeMotion.styleReads, "native viewer motion needs no injected style reads");
    await page.evaluate(() => {
      document.querySelector("#preview").style.display = "none";
      document.querySelector("#preview-backdrop").style.display = "none";
    });
    await page.waitForTimeout(30);
    near(pixel(await shot(page), 70, 70), [220, 40, 50], "closing viewer restores opaque native thumbnail");
    await page.evaluate(() => {
      window.requestAnimationFrame = window.nativeRAF;
      window.__codexPlusImageOverlayCleanup?.();
    });
  }, { width: 700, height: 600 });
  for (const zoom of [.8, 1, 1.25]) {
    check(`reversed-thread-scroll-${zoom}`, async page => {
      await page.setContent(`<style>body{margin:0;background:#181818}#shell{zoom:${zoom}}
        #thread{width:400px;height:300px;overflow:auto;display:flex;flex-direction:column-reverse;border-radius:24px}
        #content{height:1000px;flex-shrink:0;position:relative}img{position:absolute;left:10px;bottom:160px;width:80px;height:80px;border-radius:16px}
        </style><div id="shell"><div id="thread"><div id="content"><img id="photo" src="${opaque}"></div></div></div>`);
      await configure(page, wall);
      await page.evaluate(() => {
        window.nativeRAF = requestAnimationFrame;
        window.requestAnimationFrame = () => 999999;
        document.querySelector("#thread").scrollTop = -100;
      });
      await page.waitForTimeout(120);
      const r = await rect(page, "#photo"), image = await shot(page);
      near(pixel(image, r.x + r.width / 2, r.y + r.height / 2), [220, 40, 50], "native image after reversed scroll");
      near(pixel(image, 40 * zoom, 30 * zoom), [27, 35, 59], "no abandoned black rectangle");
      await page.mouse.move(r.x + r.width / 2, r.y + r.height / 2);
      await page.mouse.wheel(0, -30);
      await page.waitForTimeout(120);
      const moved = await rect(page, "#photo"), movedShot = await shot(page);
      near(pixel(movedShot, moved.x + moved.width / 2, moved.y + moved.height / 2), [220, 40, 50], "wheel keeps image opaque");
      await page.evaluate(() => {
        window.requestAnimationFrame = window.nativeRAF;
        window.__codexPlusImageOverlayCleanup?.();
      });
    }, { width: 700, height: 600 });
  }
  for (const [format, asset] of Object.entries(assets)) {
    check(`transparent-${format}-icons`, async page => {
      await page.setContent(`<style>body{margin:0;background:#181818}#host{margin:40px;display:flex;gap:20px}
        img{width:80px;height:80px;display:block}#clip{width:80px;height:80px;border-radius:24px;overflow:hidden}
        #contain{width:100px;height:160px;object-fit:contain}button{background:transparent;border:0;padding:0}
        </style><div id="host"><img id="icon" src="${asset}"><button><img id="button-icon" src="${asset}"></button>
        <div id="clip"><img id="rounded" src="${opaque}"></div><img id="contain" src="${asset}"></div>`);
      const before = await page.$eval("#icon", node => ({ opacity: getComputedStyle(node).opacity, style: node.getAttribute("style"), parent: node.parentElement.id }));
      const native = await shot(page);
      await configure(page, wall);
      const image = await shot(page), r = await rect(page, "#icon"), b = await rect(page, "#button-icon"), c = await rect(page, "#contain");
      near(pixel(image, r.x + 40, r.y + 40), [220, 40, 50], "native icon ink");
      const rgba = await sharp(Buffer.from(asset.slice(asset.indexOf(",") + 1), "base64"))
        .ensureAlpha().raw().toBuffer();
      const alpha = rgba[(40 * 80 + 12) * 4 + 3] / 255;
      const edge = [220, 40, 50].map((v, i) => Math.round(v * alpha + [27, 35, 59][i] * (1 - alpha)));
      near(pixel(image, r.x + 12, r.y + 40), edge, "native alpha blends over the wallpaper, not an exposed black hole");
      near(pixel(image, r.x + 8, r.y + 8), [27, 35, 59], "transparent icon corner has no black square");
      near(pixel(image, b.x + 8, b.y + 8), [27, 35, 59], "ordinary button backdrop remains tinted");
      near(pixel(image, b.x + 40, b.y + 40), [220, 40, 50], "native button image");
      near(pixel(image, 240, 40), [27, 35, 59], "ancestor rounded corner stays ordinary");
      near(pixel(image, c.x + 50, c.y + 8), [27, 35, 59], "object-fit letterbox is not excluded");
      assert.deepEqual(await page.$eval("#icon", node => ({ opacity: getComputedStyle(node).opacity, style: node.getAttribute("style"), parent: node.parentElement.id })), before);
      assert.equal(await page.locator("body img").count(), 4, "no visible media copy");
      await page.evaluate(() => window.__codexPlusImageOverlayCleanup?.());
    }, { width: 500, height: 320 });
  }
  for (const zoom of [.8, 1, 1.25]) {
    check(`inline-favicons-composer-and-growing-thread-${zoom}`, async page => {
      await page.setContent(`<style>
        body{margin:0;background:#181818;color:#eee;font:16px Arial}
        #shell{zoom:${zoom};isolation:isolate}#workspace{isolation:isolate}
        #thread{height:280px;width:500px;display:flex;flex-direction:column-reverse;overflow:auto}
        #content{height:1000px;flex-shrink:0;position:relative;overflow-x:clip}
        #links{position:absolute;bottom:180px;left:24px;white-space:nowrap}
        a{color:#8bc5f2;background:#38383880;border-radius:16px}
        a img{width:24px;height:24px;vertical-align:middle}
        #transcript{width:80px;height:80px;border-radius:12px;display:block}
        #composer{position:relative;height:180px;isolation:isolate;background:#55555570}
        #attachments{transform:translate(0);padding:12px;margin:18px}
        #attachment{display:block;width:100px;height:100px;object-fit:cover;border-radius:12px}
        #action{position:absolute;left:80px;top:12px;width:20px;height:20px}
        </style><div id="shell"><div id="workspace"><div id="thread"><div id="content">
        <div id="links"><a><img id="favicon" src="${assets.png}"> example.test</a>
        <img id="transcript" src="${opaque}"></div></div></div>
        <div id="composer"><div id="attachments"><img id="attachment" src="${opaque}"><button id="action">X</button></div>
        <span>Composer</span></div></div></div>`);
      const before = await page.evaluate(() => [...document.images].map(node => ({
        src: node.src, parent: node.parentElement.id, style: node.getAttribute("style"),
      })));
      const originalTransition = await page.evaluate(() => {
        window.originalTransition = document.startViewTransition;
        window.originalAnimate = Element.prototype.animate;
        return true;
      });
      assert.ok(originalTransition);
      const actionBefore = await rect(page, "#action");
      await configure(page, wall);
      assert.deepEqual(await rect(page, "#action"), actionBefore, "removing an identity transform retains the original absolute positioning container");
      await page.evaluate(() => {
        document.querySelector("#attachments").style.padding = "14px";
      });
      await page.waitForTimeout(50);
      for (const id of ["#transcript", "#attachment"]) {
        const r = await rect(page, id), image = await shot(page);
        near(pixel(image, (r.x + r.width / 2) * 2, (r.y + r.height / 2) * 2),
          [220, 40, 50], `${id} is the opaque original media`);
      }
      assert.equal(await page.locator("#codex-plus-image-overlay mask, #codex-plus-image-overlay path").count(), 0);
      assert.equal(await page.locator("body img").count(), 3, "no visible media copies or layout-image placeholders");
      await page.evaluate(() => {
        window.savedRAF = requestAnimationFrame;
        window.requestAnimationFrame = () => 999999;
        document.querySelector("#thread").scrollTop = -50;
        document.querySelector("#content").style.height = "1200px";
      });
      await page.waitForTimeout(100);
      const moved = await rect(page, "#transcript"), movedShot = await shot(page);
      near(pixel(movedShot, (moved.x + moved.width / 2) * 2, (moved.y + moved.height / 2) * 2),
        [220, 40, 50], "native image follows its actual changing layout without an independent region");
      await page.evaluate(() => {
        window.requestAnimationFrame = window.savedRAF;
        window.__codexPlusImageOverlayCleanup?.();
      });
      assert.deepEqual(await page.evaluate(() => [...document.images].map(node => ({
        src: node.src, parent: node.parentElement.id, style: node.getAttribute("style"),
      }))), before);
      assert.ok(await page.evaluate(() =>
        document.startViewTransition === window.originalTransition && Element.prototype.animate === window.originalAnimate));
      assert.equal(await page.locator("[data-codex-plus-image-native], [data-codex-plus-image-scope], [data-codex-plus-image-control]").count(), 0);
    }, { width: 700, height: 600 }, { deviceScaleFactor: 2 });
  }
  check("ordinary-image-controls-retain-patterned-wallpaper-fits", async page => {
    const pattern = `[image omitted]${(await sharp(Buffer.from(
      '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="120" height="60" fill="#2850c8"/><path d="M0 0H60V60H0Z" fill="#c85a28"/></svg>'
    )).png().toBuffer()).toString("base64")}`;
    await page.setContent(`<style>body{margin:0;background:#181818}
      #photo{position:absolute;left:30px;top:30px;width:100px;height:100px}
      #remove{position:absolute;left:72px;top:36px;width:32px;height:32px;background:#fafafa;border:0}
      </style><img id="photo" src="${opaque}"><button id="remove">X</button>`);
    for (const fitMode of ["fill", "fit", "stretch", "tile", "center"]) {
      for (const opacity of [.01, .2, .75, 1]) {
        await page.evaluate(({ pattern, fitMode, opacity }) => {
          window.__codexPlusImageOverlayCleanup?.(true);
          const fit = {
            fill: ["cover", "center center", "no-repeat"], fit: ["contain", "center center", "no-repeat"],
            stretch: ["100% 100%", "center center", "no-repeat"], tile: ["auto", "left top", "repeat"],
            center: ["auto", "center center", "no-repeat"],
          }[fitMode];
          const node = document.createElement("div");
          node.id = "reference";
          Object.assign(node.style, { position: "fixed", inset: "0", zIndex: "2147483646",
            backgroundImage: `url("${pattern}")`, backgroundSize: fit[0], backgroundPosition: fit[1],
            backgroundRepeat: fit[2], opacity: String(opacity), pointerEvents: "none" });
          document.documentElement.appendChild(node);
        }, { pattern, fitMode, opacity });
        await page.waitForTimeout(40);
        const expected = pixel(await shot(page), 76, 40);
        await page.evaluate(() => document.querySelector("#reference").remove());
        await page.evaluate(injection);
        await page.evaluate(async ({ pattern, fitMode, opacity }) => {
          window.__CODEX_PLUS_IMAGE_OVERLAY__ = { enabled: true, dataUrl: pattern, opacity, fitMode };
          installCodexPlusImageOverlay();
          await window.__codexPlusImageOverlayReady;
        }, { pattern, fitMode, opacity });
        await page.waitForTimeout(80);
        near(pixel(await shot(page), 76, 40), expected, `ordinary image control ${fitMode}/${opacity}`);
      }
    }
    await page.evaluate(() => window.__codexPlusImageOverlayCleanup?.());
    return { comparisons: 20 };
  }, { width: 700, height: 500 });
  for (const backdrop of ["none", "blur(6px)"]) check(`ordinary-popup-native-alpha-shadow-${backdrop}`, async page => {
    await page.setContent(`<style>
      body{margin:0;background:#181818;color:white;font:16px Arial}
      img{position:absolute;left:40px;top:40px;width:100px;height:100px}
      #popup{position:fixed;left:50px;top:60px;width:180px;height:90px;z-index:20;
        background:rgba(53,53,53,.85);box-shadow:0 4px 8px rgba(0,0,0,.4);backdrop-filter:${backdrop}}
      </style><img src="${opaque}"><div id="popup" role="menu">Native menu</div>`);
    await page.evaluate(wall => {
      const css = document.createElement("style");
      css.id = "reference-css";
      css.textContent = "img{z-index:2147483647}#popup{z-index:2147483647;background:rgba(53,53,53,.68);backdrop-filter:none}";
      const node = document.createElement("div");
      node.id = "reference";
      Object.assign(node.style, { position: "fixed", inset: "0", zIndex: "2147483646",
        backgroundImage: `url("${wall}")`, backgroundSize: "100% 100%", opacity: ".2" });
      document.documentElement.append(css, node);
    }, wall);
    const expected = await shot(page);
    await page.evaluate(() => {
      document.querySelector("#reference").remove();
      document.querySelector("#reference-css").remove();
    });
    await configure(page, wall);
    const actual = await shot(page);
    let maxError = 0;
    for (let y = 0; y < actual.height; y++) for (let x = 0; x < actual.width; x++) {
      const offset = (y * actual.width + x) * 4;
      for (let c = 0; c < 3; c++)
        maxError = Math.max(maxError, Math.abs(actual.data[offset + c] - expected.data[offset + c]));
    }
    assert.ok(maxError <= 1, `popup native alpha/shadow reference error: ${maxError}`);
    assert.equal(await page.locator('filter[id*="-control-"]').count(), 0, "popup produces no SVG filters");
    assert.equal(await page.$eval("#popup", n => getComputedStyle(n).backdropFilter), "none");
    await page.evaluate(() => window.__codexPlusImageOverlayCleanup?.());
    return { maxError };
  }, { width: 700, height: 500 });
  check("popup-real-underlay-preserves-media-alpha-fit-rounding-and-zoom", async page => {
    const cases = [
      { zoom: 1, fit: "fill", radius: "16px", media: assets.png, opacity: .2 },
      { zoom: .8, fit: "contain", radius: "24px", media: assets.svg, opacity: .2 },
      { zoom: 1.25, fit: "cover", radius: "24px", media: assets.png, opacity: .2 },
      { zoom: 1, fit: "none", radius: "50%", media: assets.png, opacity: .75 },
    ];
let maximumError = 0, maximumMediaBackdropError = 0;
    for (const { zoom, fit, radius, media, opacity } of cases) {
      await page.setContent(`<style>
        body{margin:0;background:#181818;color:white;font:16px Arial}
        #scene{zoom:${zoom}}
        img{position:absolute;left:40px;top:40px;width:120px;height:100px;object-fit:${fit};border-radius:${radius}}
        #popup{position:fixed;left:50px;top:60px;width:180px;height:100px;z-index:20;
          background:rgba(53,53,53,.85);box-shadow:0 4px 8px rgba(0,0,0,.4);backdrop-filter:blur(6px)}
        </style><div id="scene"><img id="photo" src="${media}"><div id="popup" role="menu"></div></div>`);
      await page.evaluate(async () => document.querySelector("img").decode());
      const sourceState = await page.$eval("#photo", node => ({
        src: node.src, parent: node.parentElement.id, style: node.getAttribute("style"),
      }));
      await page.evaluate(({ wall, opacity }) => {
        const css = document.createElement("style");
        css.id = "reference-css";
        css.textContent = `img{z-index:2147483647}#popup{z-index:2147483647;
          background:rgba(53,53,53,${.85 * (1 - opacity)});backdrop-filter:none}`;
        const node = document.createElement("div");
        node.id = "reference";
        Object.assign(node.style, { position: "fixed", inset: "0", zIndex: "2147483646",
          backgroundImage: `url("${wall}")`, backgroundSize: "100% 100%", opacity: String(opacity) });
        document.documentElement.append(css, node);
      }, { wall, opacity });
      const expected = await shot(page);
      await page.evaluate(() => {
        document.querySelector("#reference").remove();
        document.querySelector("#reference-css").remove();
      });
      await page.evaluate(injection);
      await page.evaluate(async ({ wall, opacity }) => {
        window.__CODEX_PLUS_IMAGE_OVERLAY__ = { enabled: true, dataUrl: wall, opacity, fitMode: "stretch" };
        installCodexPlusImageOverlay();
        await window.__codexPlusImageOverlayReady;
      }, { wall, opacity });
      await page.waitForTimeout(100);
      const actual = await shot(page);
      const photo = await rect(page, "#photo"), popup = await rect(page, "#popup");
      let maxError = 0, mediaBackdropError = 0, worstPixel;
      for (let y = 0; y < actual.height; y++) for (let x = 0; x < actual.width; x++) {
        const inPhoto = x >= photo.x - 1 && x <= photo.x + photo.width + 1 &&
          y >= photo.y - 1 && y <= photo.y + photo.height + 1;
        const inPopup = x > popup.x + 1 && x < popup.x + popup.width - 1 &&
          y > popup.y + 1 && y < popup.y + popup.height - 1;
        const offset = (y * actual.width + x) * 4;
        for (let c = 0; c < 3; c++) {
          const error = Math.abs(actual.data[offset + c] - expected.data[offset + c]);
          if (inPhoto) {
            mediaBackdropError = Math.max(mediaBackdropError, error);
          } else if (error > maxError) {
            maxError = error;
            worstPixel = { x, y, actual: pixel(actual, x, y), expected: pixel(expected, x, y) };
          }
        }
      }
      assert.ok(maxError <= 1, `popup alpha ${zoom}/${fit}/${opacity} reference error: ${maxError} ${JSON.stringify(worstPixel)}`);
      assert.ok(mediaBackdropError <= 1, `original media alpha ${zoom}/${fit}/${opacity} error: ${mediaBackdropError}`);
      maximumError = Math.max(maximumError, maxError);
      maximumMediaBackdropError = Math.max(maximumMediaBackdropError, mediaBackdropError);
      assert.deepEqual(await page.$eval("#photo", node => ({
        src: node.src, parent: node.parentElement.id, style: node.getAttribute("style"),
      })), sourceState);
      assert.equal(await page.locator("img").count(), 1, "no native media display copy");
      await page.evaluate(() => window.__codexPlusImageOverlayCleanup?.());
    }
    return { comparisons: cases.length, maxError: maximumError, menuOverMediaMaxError: maximumMediaBackdropError };
  }, { width: 700, height: 500 });
  check("native-preview-snapshots-before-animation", async page => {
    await page.setContent(`<style>
      body{margin:0;background:#181818}#thumbnail{width:80px;height:80px;margin:40px;border-radius:16px}
      .native-preview-active,.native-preview-active *{view-transition-name:none!important}
      .native-preview-active .shared-image{view-transition-name:image-preview-editor!important}
      .native-preview-active::view-transition-old(root){display:none}
      .native-preview-active::view-transition-new(root){animation:none}
      .native-preview-active::view-transition-old(image-preview-editor){display:none}
      .native-preview-active::view-transition-new(image-preview-editor){animation:none;object-fit:cover}
      .native-preview-active::view-transition-group(image-preview-editor){animation-duration:800ms}
      #preview{position:fixed;inset:0;background:#242424ee;display:none}
      #preview img{position:absolute;left:230px;top:150px;width:240px;height:240px}
      </style><img id="thumbnail" src="${opaque}"><div id="preview" role="dialog">
      <div data-testid="image-preview-dismiss-area"><img id="destination" src="${opaque}"></div></div>`);
    await page.evaluate(() => {
      window.nativeTransition = document.startViewTransition;
      window.nativeAnimate = Element.prototype.animate;
    });
    await configure(page, wall);
    await page.evaluate(() => {
      window.nativeRAF = requestAnimationFrame;
      window.requestAnimationFrame = () => 999999;
      document.documentElement.classList.add("native-preview-active");
      document.querySelector("#thumbnail").classList.add("shared-image");
      window.transition = document.startViewTransition(async () => {
        document.querySelector("#thumbnail").classList.remove("shared-image");
        document.querySelector("#preview").style.display = "block";
        await document.querySelector("#destination").decode();
        document.querySelector("#destination").classList.add("shared-image");
      });
    });
    await page.evaluate(() => window.transition.ready);
    const image = await shot(page);
    near(pixel(image, 650, 450), [36, 36, 38], "native preview dims the wallpaper without removing it");
    assert.equal(await page.locator("body img").count(), 2, "native source and destination only");
    if (out) {
      fs.mkdirSync(out, { recursive: true });
      await sharp(await page.screenshot()).resize({ width: 700 }).png().toFile(path.join(out, "native-preview-review.png"));
    }
    await page.evaluate(() => window.transition.finished);
    await page.evaluate(() => {
      document.querySelector("#destination").classList.add("shared-image");
      window.transition = document.startViewTransition({
        types: ["native-image-close"],
        update: async () => {
          document.querySelector("#destination").classList.remove("shared-image");
          document.querySelector("#preview").style.display = "none";
          await document.querySelector("#thumbnail").decode();
          document.querySelector("#thumbnail").classList.add("shared-image");
        },
      });
    });
    await page.evaluate(() => window.transition.ready);
    const closing = await shot(page);
    near(pixel(closing, 650, 450), [27, 35, 59], "closing snapshot restores ordinary upstream tint");
    near(pixel(closing, 60, 60), [27, 35, 59], "closing destination has no stale black rectangle");
    assert.ok(await page.evaluate(() => window.transition.types.has("native-image-close")));
    await page.evaluate(() => window.transition.finished);
    await page.evaluate(() => {
      window.__codexPlusImageOverlayCleanup?.();
      window.requestAnimationFrame = window.nativeRAF;
      document.documentElement.classList.remove("native-preview-active");
    });
    assert.ok(await page.evaluate(() => document.startViewTransition === window.nativeTransition && Element.prototype.animate === window.nativeAnimate));
  }, { width: 700, height: 500 });
  check("visible-media-do-not-measure-hidden-controls", async page => {
    await page.setContent(`<style>body{background:#181818}button{display:none}</style><img width="80" height="80" src="${opaque}">
      <div id="text"></div>${Array.from({ length: 1000 }, () => "<button>Native</button>").join("")}`);
    await configure(page, wall);
    const before = await page.evaluate(() => ({ ...window.__codexPlusImageOverlayStats }));
    await page.evaluate(async () => {
      for (let i = 0; i < 10; i++) {
        document.querySelector("#text").textContent = String(i);
        await new Promise(requestAnimationFrame);
      }
    });
    const after = await page.evaluate(() => ({ ...window.__codexPlusImageOverlayStats }));
    assert.ok(after.rectReads - before.rectReads < 20, `hidden controls caused ${after.rectReads - before.rectReads} rect reads`);
    assert.equal(after.queries, before.queries);
    await page.evaluate(() => window.__codexPlusImageOverlayCleanup?.());
    return { rectReads: after.rectReads - before.rectReads, queries: after.queries - before.queries };
  }, { width: 700, height: 500 });
}
main().catch(error => { console.error(error); process.exitCode = 1; });
