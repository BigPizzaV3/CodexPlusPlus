const { assert, assets, check, configure, css, delta, html, near, pixel, rect, settle, shot } = require("./image-overlay-fixture.cjs");
const native = "data-codex-plus-image-native";
const material = "data-codex-plus-image-popup-material";
const control = "data-codex-plus-image-control";
const zero = (work, keys) => keys.forEach(key => assert.equal(work[key], 0, key));
const image = (id = "photo") => `<img id="${id}" src="${assets.photo}">`;
const click = async (page, selector) => {
  await page.$eval(selector, node => { window.clicked = false; node.onclick = () => { window.clicked = true; }; });
  await page.locator(selector).click();
  assert.ok(await page.evaluate(() => window.clicked));
};

check("editable mention nodes remain owned by the native editor", async page => {
  await html(page, '#composer{position:relative;margin:30px;background:#333;width:500px;height:160px}' +
    '#editor{padding:20px}#editor img{width:24px;height:24px}',
    '<div id="composer" class="_ComposerLayoutRoot_test"><div id="editor" class="ProseMirror" contenteditable="true">Draft</div></div>');
  await configure(page);
  await page.evaluate(photo => {
    const editor = document.getElementById("editor");
    window.editorMutations = 0;
    window.nodeViewRebuilds = 0;
    const mention = document.createElement("span");
    const icon = document.createElement("span");
    icon.contentEditable = "false";
    icon.innerHTML = `<img src="${photo}">`;
    const label = document.createElement("span");
    label.textContent = "Selected plugin";
    mention.append(icon, label);
    // The native inlineMention view ignores its icon/text interiors and root style,
    // but root data attributes invalidate that view and rebuild its DOM.
    window.editorObserver = new MutationObserver(records => {
      window.editorMutations += records.filter(record => record.type === "attributes").length;
      window.nodeViewRebuilds += records.filter(record => record.type === "attributes" &&
        !icon.contains(record.target) && !label.contains(record.target) &&
        !(record.target === mention && record.attributeName === "style")).length;
    });
    window.editorObserver.observe(editor, { attributes: true, subtree: true });
    editor.append(mention);
  }, assets.photo);
  await settle(page);
  assert.equal(await page.evaluate(() => window.nodeViewRebuilds), 0, "no overlay-triggered native mention rebuilds");
  assert.equal(await page.evaluate(() => window.editorMutations), 0,
    "overlay attribute writes must not retrigger ProseMirror node views");
  const box = await rect(page, "#editor img");
  near(pixel(await shot(page), box[0] + 12, box[1] + 12), [220, 40, 50]);
  await page.evaluate(() => window.editorObserver.disconnect());
});

check("image badges retain native sibling stacking", async page => {
  await html(page, '#icon{position:relative;margin:30px;width:80px;height:80px;isolation:isolate}' +
    '#icon img{width:80px;height:80px}#badge{position:absolute;right:0;bottom:0;width:26px;height:26px;z-index:1;background:#00ff00}',
    `<div id="icon">${image()}<span id="badge"></span></div>`);
  await configure(page);
  near(pixel(await shot(page), 100, 100), [0, 255, 0], "badge retains native color and sibling order");
  near(pixel(await shot(page), 50, 50), [220, 40, 50], "image retains native color");
});

check("overlapping logo stack keeps later SVG and background above earlier images", async page => {
  await html(page, '#logos{display:inline-flex;margin:30px;height:40px}' +
    '.logo{display:flex;align-items:center;justify-content:center;width:40px;height:40px;overflow:hidden;background:white;border-radius:6px}' +
    '.logo+.logo{margin-left:-12px}.logo img{width:40px;height:40px}',
    `<span id="logos"><span class="logo">${image()}</span><span class="logo"><svg width="20" height="20"><rect width="20" height="20" fill="#00ff00"/></svg></span></span>`);
  const original = await shot(page);
  await configure(page);
  const painted = await shot(page);
  for (const [x, y] of [[63, 36], [74, 48], [40, 45]]) near(pixel(painted, x, y), pixel(original, x, y));
});

for (const headerKind of ["catalog", "collapsing", "titlebar"]) check(`native ${headerKind} occludes scrolling icons without control filters`, async page => {
  const catalog = headerKind === "catalog";
  const collapsing = headerKind === "collapsing";
  await html(page, '#page{height:450px;overflow:auto;position:relative}' +
    '#header{position:sticky;top:0;z-index:30;height:80px}' +
    (collapsing ? '#header::before{content:"";position:absolute;inset:0;z-index:-1;background:#303030;pointer-events:none}' :
      '#header{background:#303030}#header::after{content:"";position:absolute;top:100%;left:0;right:0;height:12px;background:linear-gradient(#303030,transparent);pointer-events:none}') +
    '#header button{position:absolute;left:140px;top:20px;height:40px}' +
    '#items{height:1500px}.entry{height:120px;padding:10px}.entry img{width:80px;height:80px}',
    `<div id="page"><div id="header" ${catalog ? 'class="sticky z-30 bg-surface"' : collapsing ? 'data-sticky class="_shell_test"' : 'data-app-shell-header-layout="thread-edge-scroll"'}>` +
    '<button id="header-action">Page title</button></div><div id="items">' +
    Array.from({ length: 10 }, () => `<div class="entry">${image()}</div>`).join("") + '</div></div>');
  const pseudo = collapsing ? "::before" : "::after";
  const fade = await page.$eval("#header", (node, pseudo) => {
    const paint = getComputedStyle(node, pseudo); return [paint.content, paint.backgroundColor, paint.top, paint.bottom, paint.height];
  }, pseudo);
  await configure(page);
  await page.$eval("#page", node => { node.scrollTop = 70; }); await settle(page);
  near(pixel(await shot(page), 40, 35), [46, 54, 78], "native opaque header clips the promoted icon");
  near(pixel(await shot(page), 40, 96), [220, 40, 50], "icon below header remains opaque");
  assert.deepEqual(await page.$eval("#header", (node, pseudo) => {
    const paint = getComputedStyle(node, pseudo); return [paint.content, paint.backgroundColor, paint.top, paint.bottom, paint.height];
  }, pseudo), fade);
  assert.equal(await page.locator("#header [" + control + "]").count(), 0);
  await click(page, "#header-action");
});

check("transparent application headers do not apply wallpaper twice", async page => {
  await html(page, '#header{height:80px;position:relative}#header.solid{background:#303030}#header button{margin:20px}',
    '<div id="header" data-app-shell-header-layout="home"><button>Action</button></div>');
  await configure(page);
  const frame = await shot(page);
  near(pixel(frame, 400, 40), pixel(frame, 400, 100), "transparent header matches surrounding wallpaper");
  assert.equal(await page.locator("#header filter, #header [" + control + "]").count(), 0);
  await page.$eval('#header', node => { node.className = 'solid'; node.dataset.appShellHeaderLayout = 'thread'; });
  await settle(page);
  near(pixel(await shot(page), 400, 40), [46, 54, 78], "route change refreshes the native backing");
  await page.$eval('#header', node => { node.className = ''; node.dataset.appShellHeaderLayout = 'home'; });
  await settle(page);
  near(pixel(await shot(page), 400, 40), pixel(frame, 400, 100), "returning home removes stale backing paint");
});

check("current native titlebar preserves nested chrome and native foreground order", async page => {
  await html(page, `
    #header{position:fixed;inset:0 0 auto;height:80px;z-index:30;pointer-events:none}
    #main-title{position:absolute;inset:0 200px 0 0;isolation:isolate;contain:layout style;background:#303030}
    #page-header{height:100%;background:#303030} #title-action{pointer-events:auto;margin:20px}
    #photo{position:absolute;top:0;left:0;width:600px;height:150px}
  `, `<img id="photo" src="${assets.photo}"><header id="header" data-app-shell-titlebar>
    <div id="main-title" data-app-shell-main-titlebar>
      <div id="page-header" data-app-shell-page-header><button id="title-action">Thread</button></div>
    </div>
  </header>`);
  const geometry = await rect(page, "#main-title");
  await configure(page, { dataUrl: assets.pattern });
  const frame = await shot(page);
  near(pixel(frame, 360, 74), [78, 56, 46], "header covers scrolling media and paints the left wall once");
  near(pixel(frame, 440, 74), [46, 54, 78], "page header paints the right wall once");
  assert.deepEqual(await rect(page, "#main-title"), geometry);
  assert.deepEqual(await css(page, "#main-title", ["zIndex", "isolation", "contain"]), ["auto", "isolate", "layout style"]);
  assert.equal(await page.locator("#header").getAttribute(native), "header");
  assert.equal(await page.locator(`#header[${control}], #header [${control}], filter`).count(), 0);
  await click(page, "#title-action");
});

check("opaque clipped native main shell keeps images and controls in native paint order", async page => {
  await html(page, `
    #app-frame{position:relative}
    #edge-backing{position:absolute;left:100px;top:0;width:600px;height:100%;background:#303030}
    #native-content-fade{position:absolute;top:8px;left:0;right:0;height:24px;
      background-image:var(--app-shell-content-fade,linear-gradient(to bottom,currentColor,transparent));color:#303030}
    #sidebar-icon{position:absolute;left:30px;top:20px;width:28px}
    main{position:relative;margin:0 0 0 100px;top:6px;width:600px;height:400px;isolation:isolate;clip-path:border-box;background:#303030}
    #shell-photo{position:absolute;left:80px;top:40px;width:100px}
    #badge{position:absolute;left:150px;top:60px;width:40px;height:30px;background:#00ff00;border:0;z-index:2}
  `, `<div id="app-frame" data-app-shell-frame><div id="edge-backing" data-app-shell-surface-backing></div><img id="sidebar-icon" src="${assets.photo}">
    <main id="main" class="bg-surface" data-app-shell-main-surface="default">
      <div id="native-content-fade" class="text-surface bg-[image:var(--app-shell-content-fade,linear-gradient(to_bottom,currentColor,transparent))]"></div>
      <img id="shell-photo" src="${assets.photo}"><button id="badge" aria-label="Badge"></button>
    </main></div>`);
  const geometry = await rect(page, "#main");
  await configure(page, { dataUrl: assets.pattern });
  const frame = await shot(page);
  near(pixel(frame, 360, 260), [78, 56, 46], "left shell wallpaper is viewport aligned");
  near(pixel(frame, 440, 260), [46, 54, 78], "right shell wallpaper is viewport aligned");
  near(pixel(frame, 440, 2), pixel(frame, 440, 260), "titlebar top inset shares the main wallpaper");
  near(pixel(frame, 440, 15), pixel(frame, 440, 260), "shell fade does not draw a black band over the wallpaper");
  near(pixel(frame, 220, 90), [220, 40, 50], "image remains untinted inside a native clip");
  near(pixel(frame, 265, 72), [0, 255, 0], "native badge stays above the image");
  near(pixel(frame, 40, 35), [220, 40, 50], "sidebar icons also stay native inside the shell");
  assert.deepEqual(await css(page, "#main", ["isolation", "clipPath"]), ["isolate", "border-box"]);
  assert.deepEqual(await rect(page, "#main"), geometry);
  assert.equal(await page.locator(`#shell-photo[${native}], #badge[${control}], filter`).count(), 0);
  await click(page, "#badge");
});

const currentFrame = content => `<div id="current-frame" data-app-shell-frame data-app-shell-page-surface="true" data-app-shell-sidebar-open="true">${content}</div>`;
const currentMain = content => `<main id="current-main" data-app-shell-main-surface="default">${content}</main>`;
const currentShellCss = `
  html{--color-surface:#303030;--color-surface-elevated-secondary:#363636;--composer-layout-surface-background:#363636}
  body{background:transparent}
  #current-frame{position:relative;height:100vh;background-image:linear-gradient(#1118,#1118)}
  #current-main{position:relative;margin-left:180px;height:100vh;background:#303030;clip-path:border-box;isolation:isolate}
  #current-backing{position:absolute;inset:40px 0 0 180px;background:#303030;border-top-left-radius:16px}
`;

check("current app opens a shell card at its final height and leaves other disclosures alone", async page => {
  const disclosure = "overflow-hidden motion-reduce:h-auto!";
  await html(page, currentShellCss, currentFrame(currentMain(`
    <div class="thread-scroll-container"><div data-turn-key="one">
      <div id="shell-height" class="${disclosure}" style="height:0px"><div data-turn-activity-window><div data-testid="exec-shell-body" style="height:180px">Output</div></div></div>
      <div id="other-height" class="${disclosure}" style="height:0px"><div style="height:180px">Other</div></div>
    </div></div>`)));
  await configure(page);
  assert.deepEqual(await css(page, "#shell-height", ["height"]), ["180px"]);
  assert.deepEqual(await css(page, "#other-height", ["height"]), ["0px"]);
  await page.$eval("#shell-height", node => node.setAttribute("aria-hidden", "true"));
  assert.deepEqual(await css(page, "#shell-height", ["height"]), ["0px"]);
});

check("current app blends the wallpaper once without dimming navigation, media or native chrome", async page => {
  await html(page, currentShellCss + `
    #current-main{top:40px;height:calc(100vh - 40px)}
    #media{position:absolute;top:90px;left:55px;width:55px;height:65px}
    #sidebar{position:absolute;inset:0 auto 0 0;width:180px;background:#242424}
    #navigation-rail{position:absolute;top:60px;left:0;width:16px;height:30px;background:#728b98}
    #header{position:sticky;top:0;height:42px;z-index:30;background:#303030}
    #browser{position:fixed;right:15px;bottom:20px;width:60px;height:60px;background:#dc2832;z-index:40}
    #browser-toolbar{background:#303030;backdrop-filter:blur(16px);height:32px}
    #viewer{position:fixed;right:15px;top:70px;width:60px;height:60px;background:#dc2832;z-index:50}
  `, currentFrame(`<div id="current-backing" data-app-shell-surface-backing></div>
    <aside id="sidebar" class="app-shell-left-panel"><button id="thread-link">Sidebar</button></aside>
    <nav id="navigation-rail" data-app-navigation-rail></nav>
    ${currentMain(`<header id="header" data-app-shell-main-titlebar>Title</header>${image("media")}
      <div id="browser-toolbar" data-browser-sidebar-toolbar><button id="back">Back</button></div>
      <div id="browser" data-browser-sidebar-webview></div>
      <div id="viewer" role="dialog" class="codex-dialog"><div data-image-preview-toolbar></div></div>`)}`));
  const ordering = await css(page, "#header", ["position", "zIndex"]);
  await configure(page, { dataUrl: assets.pattern, opacity: .35 });
  const frame = await shot(page);
  near(pixel(frame, 130, 250), [101, 63, 45], "sidebar reveals the same wallpaper material as the main area");
  near(pixel(frame, 380, 20), [101, 63, 45], "top strip does not add a second tint");
  near(pixel(frame, 380, 250), [101, 63, 45], "main matches the old single mix on the orange half");
  near(pixel(frame, 440, 250), [45, 59, 101], "main matches the old single mix on the blue half");
  for (const point of [[260, 150], [748, 545], [748, 100]]) near(pixel(frame, ...point), [220, 40, 50], "native foreground");
  assert.deepEqual(await css(page, "#header", ["position", "zIndex"]), ordering);
  assert.deepEqual(await css(page, "#navigation-rail", ["backgroundColor"]), ["rgb(114, 139, 152)"]);
  assert.deepEqual(await css(page, "#current-main", ["clipPath", "isolation"]), ["border-box", "isolate"]);
  assert.deepEqual(await css(page, "#browser-toolbar", ["backgroundColor", "filter", "backdropFilter"]), ["rgba(0, 0, 0, 0)", "none", "none"]);
  assert.equal(await page.locator(`[${native}], [${control}], filter`).count(), 0);
  await click(page, "#thread-link");
  await click(page, "#back");
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
  assert.deepEqual(await css(page, "#current-main", ["backgroundColor"]), ["rgb(48, 48, 48)"]);
  assert.deepEqual(await css(page, "#sidebar", ["backgroundColor"]), ["rgb(36, 36, 36)"]);
});

for (const [theme, width, railPixel, bodyPixel] of [
  ["dark", 960, [70, 49, 40], [80, 61, 52]],
  ["light", 580, [210, 217, 240], [211, 218, 239]],
]) {
  check(`current home project rail shares the composer material without changing its layout (${theme})`, async page => {
    const rail = `<div data-composer-rail data-composer-rail-placement="responsive"><div>
      <div id="project-rail" data-composer-utility-rail data-composer-rail-item class="bg-background-composer-action-bar">
        <button id="home-project">Project</button>
      </div>
    </div></div>`;
    await html(page, currentShellCss + `
      html{color-scheme:${theme};--color-surface:${theme === "dark" ? "#181818" : "#fafafa"};--color-text:#dfdfdf;
        --color-surface-elevated-secondary:${theme === "dark" ? "#363636" : "#ebebeb"}}
      .bg-background-composer-action-bar{background:#282828}
      [data-composer-body]{background:var(--composer-layout-surface-fill,#363636);position:relative;z-index:10;height:70px}
      #home-composer{position:absolute;top:170px;left:20px;width:240px}
      [data-composer-rail]{position:absolute;inset:auto 4px calc(100% - 4px)}
      [data-composer-rail-item]{height:32px}
      @media(max-width:640px){[data-composer-rail]{inset:calc(100% - 4px) 4px auto}}
    `, currentFrame(currentMain(`
      <div id="home-composer" ${theme === "dark" ? 'data-codex-composer-root data-composer-placement="home"' : ""}>
        ${theme === "dark" ? rail : ""}
        <div data-composer-surface-variant="default" data-composer-utility-bar-variant="home">
          ${theme === "light" ? rail : ""}
          <div id="project-composer" data-composer-body></div>
        </div>
        <div data-codex-plus-ext="fixture">
          <div id="extension-rail" data-composer-utility-rail data-composer-rail-item class="bg-background-composer-action-bar"></div>
        </div>
      </div>`)));
    const railRect = await rect(page, "#project-rail");
    const body = await rect(page, "#project-composer");
    assert.equal(railRect[1] < body[1], width > 640, "the responsive rail keeps its native side");
    await configure(page, { dataUrl: assets.pattern, opacity: .2 });
    const frame = await shot(page);
    near(pixel(frame, railRect[0] + 96, railRect[1] + 16), railPixel, "rail is translucent, not a dark plate");
    near(pixel(frame, body[0] + 96, body[1] + 28), bodyPixel, "composer keeps its existing tint");
    assert.deepEqual(await rect(page, "#project-rail"), railRect);
    assert.deepEqual(await css(page, "#project-rail", ["backgroundImage", "backdropFilter", "opacity"]), ["none", "none", "1"]);
    assert.deepEqual(await css(page, "#extension-rail", ["backgroundColor"]), ["rgb(40, 40, 40)"]);
    await click(page, "#home-project");
    await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
    assert.deepEqual(await css(page, "#project-rail", ["backgroundColor"]), ["rgb(40, 40, 40)"]);
  }, { width, height: 600 });
}

check("current app gives native menu, settings and image viewer controls translucent paint without filtering media", async page => {
  await html(page, currentShellCss + `
    #menu,.settings-dialog{background:#303030}
    .settings-dialog-backdrop{position:fixed;inset:0;background:#0003;backdrop-filter:blur(3px)}
    .settings-dialog{position:fixed;inset:120px 40px 220px}
    .settings-dialog-sidebar,[data-settings-panel],.settings-dialog-content{width:40%;height:60px;background:#303030}
    .codex-dialog-overlay{position:fixed;inset:0;background:#0006;backdrop-filter:blur(4px)}
    .codex-dialog{position:fixed;inset:40px 160px;background:transparent}
    .controlSurface-fixture{background:linear-gradient(#333,#333);backdrop-filter:blur(24px)}
    [role="toolbar"],#inline-bg,.ComposerLayoutRoot-fixture{background:#333}
    #toolbar,#inline-bg,.ComposerLayoutRoot-fixture{height:33px}
    #viewer-img{width:90px;height:75px}
  `, currentFrame(currentMain("Thread")) + `
    <div id="menu" data-radix-menubar-content role="menu" class="bg-codex-application-menu">Edit</div>
    <div class="settings-dialog-backdrop"></div>
    <div id="settings" class="settings-dialog" role="dialog">
      <aside class="settings-dialog-sidebar">Settings navigation</aside>
      <div class="bg-surface" data-settings-panel><div class="electron:bg-surface settings-dialog-content">Account</div></div>
    </div>
    <div class="codex-dialog-overlay imageOverlay-fixture"></div>
    <div class="codex-dialog" role="dialog">
      <button id="zoom" class="controlSurface-fixture">Zoom</button>
      <div data-testid="image-preview-dismiss-area">${image("viewer-img")}</div>
      <div data-image-preview-toolbar><div id="toolbar" role="toolbar" class="annotationTheme-fixture">Edit image</div></div>
      <div data-image-preview-composer>
        <div id="viewer-composer" data-composer-surface-variant="opaque" class="ComposerLayoutRoot-fixture">Compose</div>
        <div data-inline-composer><div id="inline-bg" class="bg-surface-elevated-secondary">Inline compose</div></div>
      </div>
      <div data-codex-plus-ext="fixture"><button id="other" class="controlSurface-fixture">Third-party</button></div>
    </div>`);
  const photoGeometry = await rect(page, "#viewer-img");
  const untouched = await css(page, "#other", ["backgroundImage", "backdropFilter"]);
  await configure(page, { opacity: .35 });
  for (const selector of ["#menu", "#settings", "#zoom", "#toolbar", "#inline-bg"]) {
    const [fill, picture, opacity] = await css(page, selector, ["backgroundColor", "backgroundImage", "opacity"]);
    assert.match(fill, /\/ 0\.65\)/, `${selector} uses configured popup transparency`);
    assert.equal(picture, "none");
    assert.equal(opacity, "1", "foreground is not faded");
  }
  for (const selector of [".codex-dialog-overlay", "#zoom"]) {
    assert.deepEqual(await css(page, selector, ["backdropFilter"]), ["none"]);
  }
  for (const selector of [".settings-dialog-sidebar", "[data-settings-panel]", ".settings-dialog-content"]) {
    assert.deepEqual(await css(page, selector, ["backgroundColor"]), ["rgba(0, 0, 0, 0)"]);
  }
  assert.deepEqual(await css(page, ".settings-dialog-backdrop", ["backdropFilter"]), ["blur(3px)"]);
  assert.match(await page.$eval("#viewer-composer", node =>
    getComputedStyle(node).getPropertyValue("--composer-layout-surface-fill")), /transparent/);
  assert.deepEqual(await css(page, "#other", ["backgroundImage", "backdropFilter"]), untouched);
  assert.deepEqual(await rect(page, "#viewer-img"), photoGeometry);
  const [x, y] = photoGeometry;
  near(pixel(await shot(page), x + 8, y + 8), [220, 40, 50], "viewer photo pixels stay native");
  await click(page, "#zoom");
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
  assert.deepEqual(await css(page, "#zoom", ["backgroundImage", "backdropFilter"]), untouched);
});

check("current app covers the reported native materials through CSS alone", async page => {
  const panelFill = "bg-[var(--app-shell-panel-background,var(--color-surface))]";
  const trailingFill = "bg-(--composer-layout-surface-background)";
  await html(page, currentShellCss + `
    #agents,#settings,#settings-nav,#rail,#indicator,#trailing,#changes,#file,#extension,#suggestions{background:#303030;width:150px;height:28px}
    #main-pane{position:absolute;inset:110px 0 0 auto;width:175px}
    #settings{position:fixed;left:40px;top:370px;z-index:50}
    #agent-pane{background:#303030}
    #agents{background-color:var(--app-shell-panel-background,var(--color-surface))}
    #current-footer::before{content:"";background:linear-gradient(transparent,#303030)}
    #current-footer::after{content:"";background:#303030}
    #media{width:16px;height:16px}
  `, currentFrame(currentMain(`<aside id="main-pane" data-app-shell-focus-area="right-panel">
      <div id="agent-pane" data-app-shell-tabs="true"><div id="agents" class="${panelFill}">
        <div role="tabpanel" data-app-shell-tab-panel-controller="right" data-tab-id="subagents:local">Agents</div>
      </div></div>
    </aside>
    <form data-codex-composer-root data-composer-placement="home">
      <div id="rail" data-composer-utility-rail class="bg-background-composer-action-bar">Computer / Project</div>
    </form>
    <div role="group"><span class="bg-background-mode-toggle-track"></span><span id="indicator" aria-hidden="true"></span>
      <button aria-pressed="true">Chat</button><button aria-pressed="false">Work</button>
    </div>
    <form data-chatgpt-composer data-composer-placement="thread"><div id="trailing" data-composer-trailing-controls class="rounded-full ${trailingFill}">
      <span data-model-selection-label>Model</span><button data-composer-voice-start>Voice</button>
    </div></form>
    <div id="changes" class="bg-surface-elevated-secondary/50 [--turn-diff-row-padding-y:0.25rem]">
      <div class="group/turn-diff-header">Changes</div>
      <div class="group/turn-diff-file-row"><button id="file" class="bg-surface/70">File</button></div>
    </div>
    <div id="current-footer" class="absolute inset-x-0"><div data-thread-scroll-footer><div data-thread-composer-backdrop></div></div></div>
    <div data-composer-overlay-floating-ui><div><div id="suggestions" class="bg-surface-elevated-secondary">Suggestions</div></div></div>
    <div data-codex-plus-ext="fixture"><div id="extension" role="menu" class="bg-surface-elevated-secondary">Extension</div></div>
    ${image("media")}`)) + `<div id="settings" class="settings-dialog" role="dialog">
    <div id="settings-nav" class="settings-dialog-sidebar"></div><div data-settings-panel>Settings</div>
  </div>`);
  await configure(page, { dataUrl: assets.pattern, opacity: .35 });
  for (const selector of ["#settings", "#settings-nav", "#rail", "#indicator", "#trailing", "#changes", "#file", "#agents", "#suggestions"]) {
    const [fill, picture] = await css(page, selector, ["backgroundColor", "backgroundImage"]);
    assert.notEqual(fill, "rgb(48, 48, 48)", `${selector} receives the wallpaper material`);
    assert.equal(picture, "none", `${selector} does not paint its own copy of the wallpaper`);
  }
  assert.deepEqual(await page.$eval("#current-footer", node => [
    getComputedStyle(node, "::before").backgroundImage, getComputedStyle(node, "::after").backgroundColor,
  ]), ["none", "rgba(0, 0, 0, 0)"], "footer does not add a dark band");
  assert.deepEqual(await css(page, "#extension", ["backgroundColor"]), ["rgb(48, 48, 48)"]);
  assert.equal(await page.locator("filter").count(), 0);
  assert.equal(await page.locator("#rail").innerText(), "Computer / Project");
});

check("routed settings and embedded Dot reveal the wallpaper without changing media or floating windows", async page => {
  await html(page, currentShellCss + `
    [class~="electron:bg-surface"],.messaging-root,.thread-pane,.bg-surface {background:#181818}
    .composer-wrap {border-bottom:12px solid #181818}
    #dot-avatar {width:18px;height:18px;background:#7755aa}
  `, currentFrame(`<section data-settings-panel><div id="account" class="flex electron:bg-surface">Account</div></section>
    <div class="messaging-root messaging-embedded"><div class="thread-pane"><button id="dot-avatar">A</button>
      <span class="composer-wrap"></span></div></div>
    <div class="messaging-root messaging-embedded" data-floating><div id="float" class="thread-pane"></div></div>
    <section aria-labelledby="orbit-introduction-title" class="bg-surface"><i class="bg-surface"></i><footer class="bg-surface"></footer></section>`));
  await configure(page);
  for (const selector of ["#account",".messaging-root:not([data-floating])",".messaging-root:not([data-floating]) .thread-pane","section.bg-surface","footer"]) {
    assert.deepEqual(await css(page, selector, ["backgroundColor","opacity"]), ["rgba(0, 0, 0, 0)","1"]);
  }
  assert.deepEqual(await css(page, ".composer-wrap", ["borderBottomWidth","borderBottomColor"]), ["12px","rgba(0, 0, 0, 0)"]);
  for (const selector of ["#float","i.bg-surface"]) assert.deepEqual(await css(page, selector, ["backgroundColor"]), ["rgb(24, 24, 24)"]);
  assert.deepEqual(await css(page, "#dot-avatar", ["backgroundColor"]), ["rgb(119, 85, 170)"]);
  await click(page, "#dot-avatar");
});

for (const theme of ["dark", "light"]) {
  check(`current app adjusts only the Dot footer, Work control capsule and Chat/Work indicator (${theme})`, async page => {
    const footer = id => `<div class="conversation-footer"><div data-codex-composer-root>
      <div class="transcriptBackdrop-native"><div id="${id}" aria-hidden="true" class="combined-native from-surface via-surface"></div></div>
      <div data-composer-surface-variant="default" id="${id}-input">Draft</div>
    </div></div>`;
    const trailing = id => `<div id="${id}" data-composer-trailing-controls class="bg-(--composer-layout-surface-background)">
      <button type="button" id="${id}-button" aria-label="Send">Send</button></div>`;
    await html(page, currentShellCss + `
      html{color-scheme:${theme};--color-text:#dfdfdf;--color-background-mode-toggle-selected:#282828}
      .combined-native{height:16px;background-image:linear-gradient(to top,#181818,transparent)}
      [data-composer-trailing-controls]{background-color:var(--composer-layout-surface-background)}
      [data-composer-surface-variant]{background-color:var(--composer-layout-surface-fill,#303030)}
      .bg-background-mode-toggle-track{background-color:rgb(255 255 255 / .08)}
      .indicator-native{background-color:var(--color-background-mode-toggle-selected);box-shadow:0 1px 2px #333;border:1px solid #333}
      button[aria-label="Send"]{background-color:#637c99}
      #capsule{height:28px}
    `, currentFrame(currentMain(`<div class="messaging-root messaging-embedded"><main class="thread-pane">
      ${footer("dot")}<div id="other-fade" class="combined-native from-surface via-surface"></div>
      </main></div>
      <div class="messaging-root messaging-embedded" data-floating>${footer("floating")}</div>
      <div data-thread-find-composer="true">${trailing("capsule")}</div>
      <form data-composer-placement="home">${trailing("home")}</form>
      <form data-thread-find-composer="true" data-composer-placement="thread" data-chatgpt-composer>${trailing("chat")}</form>
      <div role="group"><span id="track" class="bg-background-mode-toggle-track"></span>
        <span id="pill" class="indicator-native" aria-hidden="true"></span>
        <button type="button" id="chat-mode" aria-pressed="true">Chat</button><button type="button" id="work-mode" aria-pressed="false">Work</button>
      </div>
      <div data-codex-plus-ext="fixture"><div role="group"><span class="bg-background-mode-toggle-track"></span>
        <span id="extension-pill" class="indicator-native" aria-hidden="true"></span></div></div>`)));
    const geometry = await rect(page, "#capsule");
    const decorations = await css(page, "#pill", ["border", "boxShadow"]);
    await configure(page, { opacity: .5 });
    assert.deepEqual(await css(page, "#dot", ["backgroundImage"]), ["none"]);
    for (const selector of ["#floating", "#other-fade"]) assert.match((await css(page, selector, ["backgroundImage"]))[0], /linear-gradient/);
    const [capsule] = await css(page, "#capsule", ["backgroundColor"]);
    assert.match(capsule, /\/ 0\.08\)/);
    assert.deepEqual(await css(page, "#dot-input", ["backgroundColor"]), [capsule]);
    assert.deepEqual(await css(page, "#home", ["backgroundColor"]), ["rgb(54, 54, 54)"]);
    assert.match((await css(page, "#chat", ["backgroundColor"]))[0], /\/ 0\.5\)/);
    assert.deepEqual(await rect(page, "#capsule"), geometry);
    assert.deepEqual(await css(page, "#capsule-button", ["backgroundColor", "opacity"]), ["rgb(99, 124, 153)", "1"]);
    assert.match((await css(page, "#pill", ["backgroundColor"]))[0], /\/ 0\.25\)/);
    assert.deepEqual(await css(page, "#pill", ["border", "boxShadow"]), decorations);
    assert.deepEqual(await css(page, "#track", ["backgroundColor"]), ["rgba(255, 255, 255, 0.08)"]);
    assert.deepEqual(await css(page, "#extension-pill", ["backgroundColor"]), ["rgb(40, 40, 40)"]);
    await click(page, "#capsule-button");
    await click(page, "#work-mode");
    await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
    assert.match((await css(page, "#dot", ["backgroundImage"]))[0], /linear-gradient/);
    assert.deepEqual(await css(page, "#pill", ["backgroundColor"]), ["rgb(40, 40, 40)"]);
    assert.deepEqual(await css(page, "#capsule", ["backgroundColor"]), ["rgb(54, 54, 54)"]);
  });
}

check("file pane clears native tree rows but preserves selection, file media and browser guest", async page => {
  await html(page, currentShellCss + `
    .bg-surface {background:#181818}
    #guest,iframe {background:#dc2832} #paper {width:24px;height:24px}
  `, currentFrame(currentMain(`<aside data-app-shell-focus-area="right-panel">
    <div role="tabpanel" data-app-shell-tab-panel-controller="right" data-tab-id="file:local:">
      <div id="file-pane" class="flex h-full min-h-0 flex-col bg-surface">
        ${image("paper")}<svg width="20" height="20"><path id="glyph" fill="#123456" d="M0 0h20v20z"/></svg>
        <div id="file-preview" class="flex h-full min-h-0 flex-col bg-surface"></div>
        <file-tree-container data-file-tree-virtualized="true" style="background:#181818;--trees-bg-override:#181818"></file-tree-container>
      </div>
    </div><div class="border-b bg-surface min-h-12"><div data-browser-sidebar-toolbar></div></div>
    <div data-browser-host-root><div id="guest" data-browser-sidebar-webview><iframe srcdoc="<body style='background:#dc2832'>Document"></iframe></div></div>
    <div data-codex-plus-ext="other"><div role="tabpanel" data-app-shell-tab-panel-controller="right" data-tab-id="file:other:">
      <div id="external" class="flex h-full min-h-0 flex-col bg-surface"></div></div></div>
    </aside>`)));
  await page.$eval("file-tree-container", e => {
    e.attachShadow({mode:"open"}).innerHTML = '<style>.row{background:var(--trees-bg-override)}.selected{background:#123456}</style><div class="row">File</div><div class="row selected">Selected file</div>';
  });
  await configure(page);
  for (const selector of ["#file-pane","file-tree-container",".border-b.bg-surface"]) {
    assert.deepEqual(await css(page, selector, ["backgroundColor"]), ["rgba(0, 0, 0, 0)"]);
  }
  const treeColors = () => page.locator("file-tree-container .row").evaluateAll(nodes => nodes.map(n => getComputedStyle(n).backgroundColor));
  assert.deepEqual(await treeColors(), ["rgba(0, 0, 0, 0)", "rgb(18, 52, 86)"]);
  for (const selector of ["#external", "#file-preview"]) assert.deepEqual(await css(page, selector, ["backgroundColor"]), ["rgb(24, 24, 24)"]);
  assert.deepEqual(await css(page, "#glyph", ["fill","opacity"]), ["rgb(18, 52, 86)","1"]);
  assert.deepEqual(await css(page, "#guest", ["backgroundColor","opacity"]), ["rgb(220, 40, 50)","1"]);
  assert.equal(await page.frameLocator("iframe").locator("body").evaluate(n => getComputedStyle(n).backgroundColor), "rgb(220, 40, 50)");
  const [x,y] = await rect(page, "#paper");
  near(pixel(await shot(page),x+4,y+4),[220,40,50],"native file media pixels");
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
  assert.deepEqual(await treeColors(), ["rgb(24, 24, 24)", "rgb(18, 52, 86)"]);
});

check("native hover popover and copyable source use alpha without affecting copy, selection or SVG preview", async page => {
  await html(page, currentShellCss + `
    [popover]{background:#363636}
    [data-code-block],.sticky{background:#181818 linear-gradient(#222,#222)}
    [data-code-block-source]{user-select:text}
  `, currentFrame(currentMain(`<div id="source" data-code-block data-markdown-copy="code-block" class="CodeSurface-native">
    <div class="sticky" data-markdown-copy="exclude"><button id="copy"><svg width="10" height="10"><rect width="10" height="10" fill="#123456"/></svg>Copy</button></div>
    <div data-code-block-source><pre>value = 1</pre></div></div>
    <div id="preview" data-code-block data-markdown-copy="code-block" class="WithPreview-native"><div data-code-block-preview>${image("diagram")}</div></div>
    `)) + `<div popover="manual" role="dialog" id="thread-hover-card" class="bg-surface-elevated-secondary">Project</div>`);
  await configure(page, {opacity:.35});
  await click(page, "#copy");
  await page.$eval("#thread-hover-card", n => n.showPopover());
  for (const [selector, alpha] of [["#thread-hover-card","0.65"],["#source","0.325"],["#source > .sticky","0.325"]]) {
    const [fill, picture, opacity] = await css(page,selector,["backgroundColor","backgroundImage","opacity"]);
    assert.ok(fill.endsWith(`/ ${alpha})`), `${selector}: ${fill}`);
    assert.equal(picture,"none"); assert.equal(opacity,"1");
  }
  assert.deepEqual(await css(page,"[data-code-block-source]",["userSelect"]),["text"]);
  assert.match((await css(page,"#preview",["backgroundImage"]))[0],/linear-gradient/);
});

check("plain-text Add to chat writing blocks tint the native backing plate, not the text or controls", async page => {
  const content = `<div aria-hidden="true" class="plate"></div><button id="add-text">Add to chat</button>
    <button id="copy-text" aria-label="Copy"><svg><rect width="9" height="9" fill="#abcdef"/></svg></button>
    <div data-horizontal-scroll>Plain text</div><div class="inset-border"></div>`;
  await html(page, currentShellCss + `
    [data-oai-writing-block-surface]{--wb-surface-primary:#282828;--oai-wb-surface-primary:var(--wb-surface-primary)}
    .plate{height:28px;background:var(--oai-wb-surface-primary)} .inset-border{box-shadow:inset 0 0 0 1px #888}
  `, currentFrame(currentMain(`<div id="writing" data-oai-writing-block-surface data-markdown-copy="code-block">${content}</div>
    <div id="other-writing" data-oai-writing-block-surface><div class="plate"></div></div>`)));
  const border = await css(page, ".inset-border", ["boxShadow"]);
  await configure(page, {opacity:.65});
  assert.match((await css(page,"#writing > .plate",["backgroundColor"]))[0], /\/ 0\.175\)/);
  for (const selector of ["#writing", "[data-horizontal-scroll]", "#copy-text"]) assert.deepEqual(await css(page,selector,["opacity"]),["1"]);
  assert.deepEqual(await css(page, ".inset-border", ["boxShadow"]), border);
  assert.deepEqual(await css(page, "#other-writing > .plate", ["backgroundColor"]), ["rgb(40, 40, 40)"]);
  await click(page, "#add-text");
  await click(page, "#copy-text");
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
  assert.deepEqual(await css(page,"#writing > .plate",["backgroundColor"]),["rgb(40, 40, 40)"]);
});

check("current app reflows both scrollers and sidebars with no overlay observer or paint work", async page => {
  const rows = Array.from({ length: 120 }, (_, i) =>
    `<div data-app-action-sidebar-thread-row><button>Thread ${i}</button></div>`).join("");
  await html(page, currentShellCss + `
    #current-main{margin-left:var(--side,180px)}
    #current-frame[data-app-shell-sidebar-open="false"]{--side:120px}
    #side-scroll,#thread-scroll{height:360px;overflow:auto}
    #side-scroll{position:absolute;width:110px;inset:0 auto auto 0}
    #thread-scroll > div{height:30px}
    [data-composer-surface-variant]{background:#363636;backdrop-filter:blur(12px)}
  `, currentFrame(`<aside class="app-shell-left-panel"><div id="side-scroll" data-app-action-sidebar-scroll>${rows}</div></aside>
    ${currentMain(`<div id="thread-scroll" class="thread-scroll-container">${rows}
      <div class="absolute inset-x-0"><div data-thread-scroll-footer><div data-thread-composer-backdrop>
        <div data-composer-surface-variant="default" data-composer-radius-variant="default"><div data-composer-body>Draft</div></div>
      </div></div></div>
    </div>`)}`));
  await page.evaluate(() => { window.installOverlayTurnRetention = () => { throw new Error("wallpaper must not control native turn lifetime"); }; });
  await configure(page);
  const work = await delta(page, () => page.evaluate(async () => {
    const frame = document.getElementById("current-frame");
    const sidebar = document.getElementById("side-scroll");
    const thread = document.getElementById("thread-scroll");
    const phase = sidebar.lastElementChild;
    for (let i = 1; i <= 24; i++) {
      sidebar.scrollTop = i * 16;
      thread.scrollTop = i * 55;
      phase.className = i % 2 ? "visible" : "";
      frame.dataset.appShellSidebarOpen = String(i % 2 === 0);
      await new Promise(requestAnimationFrame);
    }
  }));
  zero(work, ["frames", "rectReads", "styleReads", "queries", "layerWrites", "markerWrites", "styleCommits", "filterNodes"]);
  assert.equal(await page.evaluate(() => window.intersectionTargets.size + window.resizeTargets.size), 0);
  assert.deepEqual(await css(page, "[data-composer-surface-variant]", ["backdropFilter"]), ["none"]);
});

check("current app keeps project labels and short change lists paintable without altering wheel targeting", async page => {
  const rows = Array.from({ length: 28 }, (_, i) =>
    `<button data-app-action-sidebar-project-row data-app-action-sidebar-project-id="${i}" class="deferred [content-visibility:auto]">Project ${i}</button>`).join("");
  const threadRows = Array.from({ length: 22 }, (_, i) =>
    `<button id="thread-${i}" data-app-action-sidebar-thread-row class="deferred">Thread ${i}</button>`).join("");
  await html(page, currentShellCss + `
    #projects{position:absolute;inset:40px auto auto 0;width:170px;height:240px;overflow:auto}
    #projects button{display:block;height:32px}
    .deferred{content-visibility:auto;contain-intrinsic-block-size:auto 32px}
    #seam{height:16px}
  `, currentFrame(`<aside class="app-shell-left-panel"><div id="projects" data-app-action-sidebar-scroll>${rows}
    <div id="seam"></div><div data-app-action-sidebar-section-heading="Recents">Recents</div>${threadRows}</div></aside>
    ${currentMain(`<div data-virtualized-turn-content class="deferred" id="transcript">
      <div class="bg-surface-elevated-secondary/50 [--turn-diff-row-padding-y:0.25rem]">
        <div class="group/turn-diff-header">Changes</div>
        <div>${Array.from({ length: 4 }, (_, i) =>
          `<div id="file-${i}" class="group/turn-diff-file-row deferred [content-visibility:auto]">Changed file ${i}</div>`).join("")}</div>
      </div>
    </div>`)}`));
  await configure(page);
  assert.deepEqual(await css(page, "[data-app-action-sidebar-project-id='27']", ["contentVisibility", "contain"]), ["visible", "content"]);
  assert.deepEqual(await css(page, "#file-2", ["contentVisibility"]), ["visible"]);
  for (const selector of ["#file-3", "#thread-5", "#transcript"])
    assert.deepEqual(await css(page, selector, ["contentVisibility"]), ["auto"]);
  await page.$eval("#projects", node => { node.scrollTop = document.querySelector("#seam").offsetTop - 120; });
  const [x, y, , height] = await rect(page, "#seam");
  const initial = await page.$eval("#projects", node => node.scrollTop);
  await page.mouse.move(x + 100, y + height / 2);
  await page.mouse.wheel(0, 70);
  await page.waitForFunction(initial => document.querySelector("#projects").scrollTop === initial + 70, initial);
  await page.mouse.wheel(0, -70);
  await page.waitForFunction(initial => document.querySelector("#projects").scrollTop === initial, initial);
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
  assert.deepEqual(await css(page, "[data-app-action-sidebar-project-id='27']", ["contentVisibility"]), ["auto"]);
});

check("current app switches from the startup shell once and releases all legacy work", async page => {
  await html(page, currentShellCss, "<div id='mount'>Loading</div>");
  await configure(page, { dataUrl: assets.pattern, opacity: .35 });
  const resource = await page.evaluate(() => window.__codexPlusImageOverlayResource.blobUrl);
  await page.$eval("#mount", (node, markup) => { node.innerHTML = markup; }, currentFrame(currentMain("Ready")));
  await page.waitForFunction(() =>
    getComputedStyle(document.getElementById("codex-plus-image-overlay")).zIndex === "-1");
  assert.equal(await page.evaluate(() => window.__codexPlusImageOverlayResource.blobUrl), resource);
  assert.equal(await page.locator('[data-codex-plus-ext="image-overlay"]').count(), 2);
  assert.equal(await page.evaluate(() => window.intersectionTargets.size + window.resizeTargets.size), 0);
  near(pixel(await shot(page), 440, 250), [45, 59, 101], "late mount uses the same wallpaper coordinates");
  await page.evaluate(async () => {
    window.__CODEX_PLUS_IMAGE_OVERLAY__.opacity = .7;
    installCodexPlusImageOverlay();
    await window.__codexPlusImageOverlayReady;
  });
  assert.equal(await page.evaluate(() => window.__codexPlusImageOverlayResource.blobUrl), resource);
  near(pixel(await shot(page), 440, 250), [42, 70, 154], "hot update reuses the resource and applies opacity once");
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
  assert.equal(await page.evaluate(() => window.__codexPlusImageOverlayResource), null);
});

check("sticky header top extensions keep continuous wallpaper color", async page => {
  await html(page, '#page{position:relative;padding-top:16px;background:#303030;height:450px;overflow:auto}' +
    '#header{position:sticky;top:16px;z-index:30;height:80px;background:#303030}' +
    '#header::before{content:"";position:absolute;inset:-8px 0 auto;height:8px;background:#303030}' +
    '#header::after{content:"";position:absolute;inset:100% 0 auto;height:12px;background:linear-gradient(#303030,transparent)}',
    '<div id="page"><div id="header" class="sticky z-30 bg-surface">Plugins</div></div>');
  const geometry = await rect(page, "#header");
  await configure(page);
  const frame = await shot(page);
  for (const y of [4, 12, 40, 140]) near(pixel(frame, 400, y), [46, 54, 78], "header top edge matches page at " + y);
  assert.deepEqual(await rect(page, "#header"), geometry);
  assert.equal(await page.locator("filter").count(), 0);
});

check("Codex home project bar retains its existing composer paint", async page => {
  await html(page, '#composer{position:relative;margin:30px;width:500px;background:#303030;border-radius:20px}' +
    '#body{position:relative;height:140px}#actions{position:relative;height:60px;background:#282828;border-radius:20px 20px 0 0}' +
    '#actions button{margin:10px}',
    '<div id="composer" class="_ComposerLayoutRoot_test" data-composer-utility-bar-variant="home">' +
    '<div class="bottom-full" style="transform:translateY(0)"><div id="actions" data-composer-placement="home" class="bg-background-composer-action-bar"><button>Project</button></div></div>' +
    '<div id="body" class="_ComposerLayoutBody_test" contenteditable="true">Draft</div></div>');
  await configure(page);
  near(pixel(await shot(page), 450, 65), [40, 40, 40], "Codex project bar retains the original native appearance");
  assert.equal(await page.locator('#actions[data-codex-plus-image-paint]').count(), 0);
});

for (const shell of ["_shell_test", "shell-yWQKe6"]) check(`native header backing follows its top inset and live theme without seams ${shell}`, async page => {
  await html(page, ':root{--surface:#303030}.light{--surface:#e0e0e0}body{background:var(--surface)}' +
    '#header{position:sticky;top:-16px;z-index:30;height:80px;padding-top:16px;container-type:inline-size}' +
    '#header::before{content:"";position:absolute;z-index:-1;inset:var(--top-inset) 0 0;background:var(--surface)}',
    `<div id="header" class="${shell}" data-sticky style="--top-inset:8px">Plugins</div>`);
  await configure(page, { dataUrl: assets.pattern });
  for (const [width, base] of [[800, 48], [900, 224]]) {
    if (base === 224) {
      await page.evaluate(() => document.documentElement.classList.add("light"));
      await page.setViewportSize({ width, height:600 });
      await settle(page);
    }
    const frame = await shot(page);
    for (const [x, wall] of [[100, [200, 90, 40]], [700, [40, 80, 200]]]) {
      const expected = wall.map(value => Math.round(base * .8 + value * .2));
      for (const y of [4, 12, 40, 110]) near(pixel(frame, x, y), expected, "native top inset and header match the page");
    }
    assert.equal(await page.$eval("#header", node => getComputedStyle(node, "::after").content), "none");
  }
  assert.equal(await page.locator("filter").count(), 0);
});

check("Work home action bar shares the global wallpaper through its native transform", async page => {
  await html(page, '#composer{position:relative;margin:30px;width:500px}' +
    '#body{position:relative;height:140px;background:#303030;border-radius:20px}' +
    '#actions{position:relative;height:60px;background:#282828;border-radius:0 0 20px 20px}' +
    '#actions button{margin:10px}#body img{width:40px;height:40px;margin:20px}',
    '<div id="composer" class="_ComposerLayoutRoot_test" data-composer-utility-bar-variant="home">' +
    `<div id="body" class="_ComposerLayoutBody_test" data-composer-body>${image()}<div contenteditable="true">Draft</div></div>` +
    '<div class="top-full" style="transform:translateY(0)"><div id="actions" data-composer-placement="home" class="bg-background-composer-action-bar"><button id="project">Project</button><button id="plugin">Plugins</button></div></div></div>');
  const geometry = await rect(page, "#actions");
  await configure(page, { dataUrl: assets.pattern });
  const frame = await shot(page);
  for (const x of [50, 250, 450, 510]) near(pixel(frame, x, 220), pixel(frame, x, 260), "Work action bar reveals the same viewport wallpaper");
  near(pixel(await shot(page), 65, 65), [220, 40, 50]);
  assert.deepEqual(await rect(page, "#actions"), geometry);
  await click(page, "#project"); await click(page, "#plugin");
  assert.equal(await page.locator("filter").count(), 0);
  assert.equal(await page.locator('#actions[data-codex-plus-image-paint]').count(), 0);
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) {
      document.querySelector('.top-full').style.transform = `translateY(${-i}px)`;
      await new Promise(requestAnimationFrame);
    }
  }));
  zero(work, ["rectReads", "styleReads", "filterNodes", "styleCommits"]);
  for (const placement of ["bottom-full", "top-full"]) {
    await page.$eval('#actions', (node, placement) => { node.parentElement.className = placement; }, placement);
    await settle(page);
    assert.deepEqual(await css(page, '#actions', ['backgroundColor']),
      [placement === 'top-full' ? 'rgba(0, 0, 0, 0)' : 'rgb(40, 40, 40)']);
  }
});

check("native headers retain both occupied pseudo-elements", async page => {
  await html(page, '#header{position:relative;height:80px;background:#303030}' +
    '#header::before{content:"before";position:absolute;left:0;background:red;pointer-events:none}' +
    '#header::after{content:"after";position:absolute;right:0;background:blue;pointer-events:none}',
    '<div id="header" data-app-shell-header-layout="thread"><button id="native-action">Title</button></div>');
  const pseudoPaint = () => page.$eval("#header", node => ["::before", "::after"].map(pseudo => {
    const paint = getComputedStyle(node, pseudo); return [paint.content, paint.backgroundColor, paint.position, paint.top, paint.left, paint.width, paint.height];
  }));
  const nativePaint = await pseudoPaint();
  await click(page, "#native-action");
  await configure(page);
  assert.deepEqual(await pseudoPaint(), nativePaint);
  await click(page, "#native-action");
});

check("browser portal retains native paint and animation containment", async page => {
  await html(page, '#portal{position:fixed;inset:0;pointer-events:none;contain:layout paint;isolation:isolate}' +
    '#pane{position:absolute;inset:0 0 0 auto;width:300px;pointer-events:auto;will-change:transform;isolation:isolate}' +
    '#web{height:300px;background:#dc2832}#chrome{height:50px;background:#333}',
    '<div id="portal"><div id="pane" data-app-shell-focus-area="right-panel"><div id="chrome" data-browser-sidebar-toolbar></div><div id="web" data-browser-sidebar-webview></div></div></div>');
  const portal = await css(page, "#portal", ["contain", "isolation"]);
  const pane = await css(page, "#pane", ["isolation", "willChange"]);
  await configure(page);
  assert.deepEqual(await css(page, "#portal", ["contain", "isolation"]), portal);
  assert.deepEqual(await css(page, "#pane", ["isolation", "willChange"]), pane);
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) {
      document.getElementById("pane").style.transform = `translateX(${i}px)`;
      await new Promise(requestAnimationFrame);
    }
  }));
  zero(work, ["rectReads", "styleReads", "layerWrites", "filterNodes"]);
  const nativeBounds = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) {
      document.getElementById("web").style.cssText = `height:${300 + i}px;clip-path:inset(0 ${i}px 0 0);opacity:${1 - i / 100}`;
      await new Promise(requestAnimationFrame);
    }
  }));
  zero(nativeBounds, ["rectReads", "styleReads", "layerWrites", "filterNodes"]);
});

check("browser navigation preserves the outer native host and application chrome", async page => {
  await html(page, '#titlebar{height:50px;background:#333}#titlebar button{height:40px}' +
    '#frame-host,#browser-host{position:fixed;inset:0;pointer-events:none}' +
    '#browser-host{contain:paint;clip-path:inset(50px 0 0 400px)}' +
    '#web{position:fixed;left:400px;top:100px;width:400px;height:400px;overflow:hidden;pointer-events:auto}' +
    '#web iframe{border:0;width:100%;height:100%}' +
    '#tabs{position:absolute;right:0;top:0;width:380px;height:50px;overflow:hidden;z-index:10}' +
    '#toolbar{position:absolute;left:400px;top:50px;width:400px;height:50px;background:#333}',
    '<div id="titlebar" data-app-shell-header-layout="thread-edge-scroll"><div data-app-shell-focus-area="main" role="toolbar"><button id="title">Thread title</button></div>' +
    '<div id="tabs" data-app-shell-focus-area="right-panel" data-app-shell-titlebar-slot="content"><button id="tab">Browser</button></div></div>' +
    '<div id="toolbar" data-browser-sidebar-toolbar><button id="back">Back</button></div>' +
    '<div id="frame-host" data-embedded-frame-host-root><div id="browser-host" data-browser-host-root>' +
    '<div id="web" data-browser-sidebar-webview><iframe srcdoc="<body style=\'margin:0;background:rgb(220,40,50)\'>Page one"></iframe></div></div></div>');
  await configure(page);
  const titlebar = await shot(page), headerBox = await rect(page, "#titlebar");
  for (let i = 0; i < 3; i++) {
    near(pixel(await shot(page), 600, 200), [220, 40, 50], "guest content excludes wallpaper");
    assert.deepEqual(await rect(page, "#titlebar"), headerBox);
    const frame = await shot(page);
    for (const [x, y] of [[15, 25], [455, 25]]) near(pixel(frame, x, y), pixel(titlebar, x, y), "navigation preserves header pixels");
    for (const selector of ["#title", "#tab", "#back"]) await click(page, selector);
    await page.evaluate(() => {
      const web = document.getElementById("web");
      document.getElementById("frame-host").append(web);
      web.style.opacity = ".001";
      web.style.contain = "layout paint size style";
    });
    await settle(page);
    await page.evaluate(() => {
      const web = document.getElementById("web");
      document.getElementById("browser-host").append(web);
      web.style.opacity = "1";
      web.style.contain = "";
      web.querySelector("iframe").srcdoc = "<body style='margin:0;background:rgb(220,40,50)'>Navigated page";
    });
    await settle(page);
  }
  assert.deepEqual(await css(page, "#browser-host", ["contain"]), ["paint"]);
  assert.equal(await page.locator("#titlebar [" + control + "]").count(), 0);
});

check("native footer occludes transcript below the composer", async page => {
  await html(page, '#frame{position:relative;width:600px;height:500px}' +
    '#thread{height:500px;overflow:auto}#transcript{height:1000px;background:#dc2832}' +
    '#footer{position:absolute;bottom:0;left:20px;right:20px;height:150px;z-index:20;pointer-events:none}' +
    '#surface{position:absolute;inset:0;background:#303030;z-index:0}' +
    '#footer-content{position:relative;z-index:10;padding:8px 20px}' +
    '#composer{position:relative;height:100px;background:#404040;border-radius:12px;pointer-events:auto;box-shadow:0 4px 10px #0008}',
    '<div id="frame"><div id="thread" class="thread-scroll-container"><div id="transcript"></div>' +
    '<div id="footer" data-thread-scroll-footer><div id="surface" aria-hidden="true"></div>' +
    '<div id="footer-content"><div id="composer" class="_ComposerLayoutRoot_test"><div contenteditable="true">Editor</div></div></div></div></div></div>');
  const geometry = await rect(page, "#composer");
  await configure(page);
  assert.deepEqual(await rect(page, "#composer"), geometry);
  assert.deepEqual(await css(page, "#footer-content", ["zIndex"]), ["10"]);
  const expected = [46, 54, 78];
  near(pixel(await shot(page), 300, 485), expected, "native footer hides transcript at its lower edge");
  await page.$eval("#thread", n => { n.scrollTop = 100; }); await settle(page);
  near(pixel(await shot(page), 300, 485), expected);
  assert.equal(await page.locator("filter").count(), 0, "composer and footer shadows need no SVG repaint");
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) {
      document.getElementById("frame").style.width = 600 + i + "px";
      await new Promise(requestAnimationFrame);
    }
  }));
  zero(work, ["composerReads", "filterNodes", "layerWrites"]);
});

for (const compact of [false, true]) check(`current ${compact ? "compact" : "thread"} footer paints its native backing without hiding the gutter`, async page => {
  const footer = `<div id="footer" class="absolute inset-x-0"><div data-thread-scroll-footer="true">
    <div data-thread-composer-backdrop><div id="composer" data-composer-surface-variant="default"
      data-composer-radius-variant="default"><div data-composer-body><button id="send">Send</button></div></div></div>
  </div></div>`;
  await html(page, `
    :root{--color-surface:#303030;--composer-radius:16px}
    #frame{position:relative;height:500px}
    #thread{height:500px;overflow:auto;scrollbar-gutter:stable both-edges}
    #thread::-webkit-scrollbar{width:16px}#thread::-webkit-scrollbar-thumb{background:#00ff00}
    #transcript{height:950px;background:#dc2832}
    #footer{position:absolute;inset: auto 0 0;height:150px;z-index:20;pointer-events:none}
    #footer::before,#footer::after{content:"";position:absolute;inset-inline:0;pointer-events:none}
    #footer::before{top:0;bottom:32px;background:linear-gradient(to bottom,transparent,var(--color-surface) calc(100% - var(--composer-radius)))}
    #footer::after{top:118px;bottom:0;background:var(--color-surface)}
    [data-thread-scroll-footer]{position:relative}
    #composer{position:relative;z-index:10;margin:14px 40px;height:95px;background:#404040;border-radius:16px;pointer-events:auto}
    #send{margin:12px}
  `, `<div id="frame" data-request-input-activity-root>
    <div id="thread" class="thread-scroll-container"><div id="transcript"></div>${compact ? "" : footer}</div>
    ${compact ? footer : ""}
  </div>`);
  await configure(page, { dataUrl: assets.pattern });
  const frame = await shot(page);
  near(pixel(frame, 360, 485), [78, 56, 46], "footer solid backing shows left wallpaper");
  near(pixel(frame, 440, 485), [46, 54, 78], "footer solid backing shows right wallpaper");
  near(pixel(frame, 440, 462), [46, 54, 78], "opaque end of native fade matches the solid part");
  near(pixel(frame, 420, 350), pixel(frame, 420, 349), "native fade starts transparent");
  assert.deepEqual(await rect(page, "#footer"), [16, 350, 768, 150]);
  assert.equal(await page.$eval("[data-thread-scroll-footer]", n => n.hasAttribute("data-codex-plus-image-native")), false);
  await click(page, "#send");
  assert.equal(await page.locator("filter").count(), 0);
  await page.$eval("#thread", n => { n.scrollTop = n.scrollHeight; });
  await settle(page);
  await page.mouse.move(792, 445); await page.mouse.down(); await page.mouse.move(792, 365, { steps: 5 }); await page.mouse.up();
  assert.ok(await page.$eval("#thread", n => n.scrollTop < 450), "native scrollbar can be dragged near the footer");
  await page.$eval("[data-thread-composer-backdrop]", node => node.removeAttribute("data-thread-composer-backdrop"));
  await settle(page);
  assert.equal(await page.$eval("#footer", node => getComputedStyle(node, "::before").maskImage), "none");
});

// #2813：侧栏打开时原生 footer 带 translate:-154px；transform 祖先会让 fixed 背景退化为元素局部坐标。
for (const transform of ["translate:-154px", "translate:0", "transform:translateX(0)"])
check(`footer wallpaper stays viewport aligned under ${transform}`, async page => {
  await html(page, '#frame{position:relative;width:600px;height:500px;margin-left:154px}' +
    '#thread{height:500px;overflow:auto}#transcript{height:1000px;background:#dc2832}' +
    '#footer{position:absolute;bottom:0;left:20px;right:20px;height:150px;z-index:20;pointer-events:none}' +
    '#surface{position:absolute;inset:0;background:#303030}',
    '<div id="frame"><div id="thread" class="thread-scroll-container"><div id="transcript"></div>' +
    `<div id="footer" data-thread-scroll-footer style="${transform}"><div id="surface" aria-hidden="true"></div></div></div></div>`);
  await configure(page, { dataUrl: assets.pattern });
  const attachment = () => page.$eval("#surface", node => getComputedStyle(node, "::after").backgroundAttachment);
  // 视口里图案左半在 x<400；按 footer 自身盒子拉伸时，380/420 两点至少一处会翻色。
  const aligned = async label => {
    const frame = await shot(page);
    near(pixel(frame, 380, 430), [78, 56, 46], label + " left half");
    near(pixel(frame, 420, 430), [46, 54, 78], label + " right half");
  };
  assert.equal(await attachment(), "scroll");
  await aligned(transform);
  await page.$eval("#footer", node => { node.style.cssText = ""; }); await settle(page);
  assert.equal(await attachment(), "fixed", "untransformed footer keeps the fixed wallpaper path");
  await aligned("restored");
  await page.$eval("#footer", node => { node.style.cssText = "transition:translate .1s linear;translate:-154px"; });
  await page.waitForTimeout(300);
  assert.equal(await attachment(), "scroll", "sidebar translate transition refreshes after it ends");
  await aligned("after sidebar transition");
});

check("bottom aligned submenu retains its native placement transform", async page => {
  await html(page, '#wrapper{position:fixed;top:530px;left:20px;transform:translateX(10px)}' +
    '#submenu{width:240px;height:240px;background:#303030;transform:translateY(calc(32px + 8px - 100%))}',
    '<div id="wrapper" data-radix-popper-content-wrapper><div id="submenu" role="menu"><button>Help</button></div></div>');
  const geometry = await rect(page, "#submenu");
  await configure(page);
  assert.deepEqual(await rect(page, "#submenu"), geometry);
  assert.ok(geometry[1] >= 0 && geometry[1] + geometry[3] <= 600);
});

check("extension widgets keep their native layer above the composer", async page => {
  await html(page, '#composer{position:relative;margin:30px;width:400px;height:200px;background:#303030}' +
    '#widget{position:fixed;left:60px;top:60px;width:80px;height:80px;background:#00ff00;z-index:9999}' +
    '#typing{position:fixed;left:160px;top:60px;width:80px;height:80px;z-index:2147483000;pointer-events:none}',
    '<div id="composer" class="_ComposerLayoutRoot_test"><input></div>' +
    '<div id="widget" data-codex-plus-ext="agent-pet"></div>' +
    '<canvas id="typing" data-codex-plus-ext="builtin-typing-effects" width="80" height="80"></canvas>');
  await page.$eval("#typing", canvas => { const context = canvas.getContext("2d"); context.fillStyle = "#dc2832"; context.fillRect(0, 0, 80, 80); });
  await configure(page);
  near(pixel(await shot(page), 100, 100), [0, 255, 0]);
  near(pixel(await shot(page), 200, 100), [220, 40, 50]);
  for (const id of ["widget", "typing"]) {
    assert.equal(await page.$eval("#" + id, node => [...node.attributes].some(attr => attr.name.startsWith("data-codex-plus-image-"))), false);
  }
});
async function thread(page, count = 40) {
  await html(page, `
    #thread{position:relative;width:620px;height:400px;overflow:auto}
    .row{height:110px;position:relative}.row img{width:90px;height:90px}
    .row button{position:absolute;left:60px;top:35px;width:40px;height:40px;background:white}
    #composer{position:fixed;top:450px;left:0;width:600px;height:100px;background:#333;box-shadow:0 2px 5px black}
    #viewer{display:none;position:fixed;inset:0;background:#111;z-index:50}
    #viewer img{position:absolute;left:200px;top:120px;width:300px;height:300px}#menu{position:fixed;left:640px;top:20px;width:140px;height:140px;background:#333}
  `, `<div id="thread" data-app-action-timeline-scroll><div id="content">
    ${Array.from({ length: count }, (_, i) => `<div class="row">${image("img-" + i)}<button>B</button></div>`).join("")}
    <div id="stream"></div></div></div><div id="composer" class="_ComposerLayoutRoot_test"><input></div>
    <div id="menu" role="menu"><button>Option</button></div>
    <div id="viewer" class="codex-dialog" role="dialog"><div data-image-preview-toolbar><button>Close</button></div>${image("viewed")}</div>`);
  await configure(page);
}
async function popup(page, kind, opacity = .2) {
  const role = { menu: 'role="menu"', listbox: 'role="listbox"', tooltip: 'role="tooltip"', dialog: 'role="dialog"' }[kind] || "";
  await html(page, `
    :root{--menu-color:rgb(240,240,240)}#photo{position:absolute;left:30px;top:30px;width:180px;height:180px}
    #boundary{position:fixed;left:20px;top:20px;z-index:50;transform:translate(7px,11px)}
    #surface{width:280px;height:210px;background:var(--menu-color);border-radius:12px;backdrop-filter:blur(10px)}
    #list{height:190px;overflow:auto}#option{margin:25px;width:90px;height:30px;background:#00ff00;color:black}
  `, `${image()}<div id="boundary" data-radix-popper-content-wrapper>
    <div id="shell" ${kind === "roleless" ? 'data-composer-overlay-floating-ui' : ""}>
    <div id="surface" ${role}><div id="list"><button id="option">Choose</button>
    <input id="range" type="range" value="20"><div style="height:600px"></div></div></div></div></div>`);
  const geometry = await rect(page, "#surface");
  await configure(page, { opacity });
  assert.deepEqual(await rect(page, "#surface"), geometry);
}

for (const tooltip of ["_Tooltip_test", "Tooltip-IBN1Wh"])
for (const anchor of ["composer", "header"]) check(`native shared tooltip portal clears ${anchor} ${tooltip}`, async page => {
  await html(page, `
    .fixed{position:fixed}.inset-0{inset:0}.pointer-events-none{pointer-events:none}.contain-layout{contain:layout}
    #anchor{position:fixed;left:0;top:0;width:600px;height:200px;background:#303030;z-index:20}
    #portal{z-index:60}#portal>*{pointer-events:auto}
    #boundary{position:fixed;left:20px;top:20px;z-index:50;transform:translate(7px,11px)}
    #tooltip{width:280px;height:120px;background:rgb(240,240,240)}
    #tooltip button{margin:20px;width:100px;height:30px;background:#00ff00}
    #sr{position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%)}
  `, `<div id="anchor" ${anchor === "composer" ? 'class="_ComposerLayoutRoot_test"' : 'data-app-shell-header-layout="thread"'}>Native anchor</div>
    <div id="portal" class="pointer-events-none fixed inset-0 zoom-adjusted-viewport contain-layout">
      <div id="boundary" data-radix-popper-content-wrapper>
        <div id="tooltip" class="${tooltip}"><button id="tooltip-action">Usage</button><span id="sr" role="tooltip">Usage</span></div>
      </div>
    </div>`);
  const geometry = await rect(page, "#boundary");
  await configure(page);
  assert.deepEqual(await rect(page, "#boundary"), geometry);
  assert.equal(await page.$eval("#tooltip", (node, attr) => node.hasAttribute(attr), material), true,
    "the visible tooltip receives material instead of only its screen-reader description");
  near(pixel(await shot(page), 200, 110), [201, 203, 208], "tooltip is above the painted native anchor");
  await click(page, "#tooltip-action");
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 8; i++) {
      document.getElementById("boundary").style.transform = `translate(${7 + i}px,11px)`;
      await new Promise(requestAnimationFrame);
    }
  }));
  zero(work, ["rectReads", "layerWrites", "filterNodes"]);
  await page.evaluate(() => {
    const wrapper = document.createElement("div");
    wrapper.id = "second"; wrapper.setAttribute("data-radix-popper-content-wrapper", "");
    wrapper.style.cssText = "position:fixed;left:400px;top:250px";
    wrapper.innerHTML = '<div role="menu" style="background:#eee"><button>Second</button></div>';
    document.getElementById("portal").append(wrapper);
  });
  await settle(page);
  await page.$eval("#boundary", node => node.remove()); await settle(page);
  assert.equal(await page.$eval("#portal", node => getComputedStyle(node).zIndex), "1002");
  await page.$eval("#second", node => node.remove()); await settle(page);
  assert.equal(await page.$eval("#portal", node => getComputedStyle(node).zIndex), "60",
    "the shared portal returns to its native layer after its last popup closes");
});

check("native toast area clears the titlebar without repainting or tracking notifications", async page => {
  await html(page, '#header{position:fixed;inset:0 0 auto;height:120px;background:#303030;z-index:20}' +
    '.codex-toast-area{position:fixed;inset:0;z-index:60;pointer-events:none}' +
    '[data-sonner-toaster]{position:fixed;left:30px;top:20px;z-index:999999999;margin:0;padding:0}' +
    '[data-sonner-toast]{position:relative;list-style:none;width:220px;height:80px;background:#60141c;pointer-events:auto}' +
    '[data-sonner-toast] button{margin:10px;width:60px;height:30px;background:#00ff00;border:0}',
    '<div id="header" data-app-shell-header-layout="thread">Native title</div>');
  await configure(page);
  await page.evaluate(() => {
    const area = document.createElement("div");
    area.id = "toasts"; area.className = "codex-toast-area pointer-events-none fixed inset-0";
    area.innerHTML = '<div class="relative w-full"><ol data-sonner-toaster class="codex-sonner-toaster">' +
      '<li data-sonner-toast><button id="dismiss">Close</button></li></ol></div>';
    document.body.append(area);
  });
  await settle(page);
  near(pixel(await shot(page), 200, 80), [96, 20, 28], "the toast, not the titlebar, owns the overlapping pixels");
  await click(page, "#dismiss");
  assert.equal(await page.locator("[data-sonner-toast][" + material + "]").count(), 0,
    "notification surfaces retain native colors");
  assert.equal(await page.locator("filter").count(), 0);
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 8; i++) {
      document.querySelector("[data-sonner-toast]").style.transform = `translateY(${i}px)`;
      await new Promise(requestAnimationFrame);
    }
  }));
  zero(work, ["rectReads", "styleReads", "layerWrites", "filterNodes", "styleCommits"]);
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup()); await settle(page);
  assert.equal(await page.$eval("#toasts", node => getComputedStyle(node).zIndex), "60",
    "disabling the overlay restores the original toast context");
});

check("modern toast keeps its existing layer and material", async page => {
  await html(page, `
    :root{--toast-z-index:2147479999}
    #toast{position:fixed;inset:0;pointer-events:none;z-index:var(--toast-z-index)}
    #toast [data-sonner-toast]{position:absolute;left:30px;top:20px;width:200px;height:80px;background:#60141c}
    #dialog{position:fixed;inset:0;z-index:2147479900;background:#303030}
  `, `<div id="dialog"></div><div id="toast" class="codex-toast-area">
    <div data-sonner-toast></div>
  </div>`);
  await configure(page);
  assert.deepEqual(await css(page, "#toast", ["zIndex", "backgroundColor"]), ["2147479999", "rgba(0, 0, 0, 0)"]);
  assert.equal(await page.locator("#toast[" + material + "]").count(), 0);
  near(pixel(await shot(page), 100, 60), [96, 20, 28], "higher native toasts stay above native overlays");
});

check("current image viewer keeps its backdrop and content above the app in an owned dialog portal", async page => {
  await html(page, `
    .fixed{position:fixed}.inset-0{inset:0}.contain-layout{contain:layout}
    #header{position:fixed;inset:0 0 auto;height:120px;background:#303030;z-index:30}
    #portal{z-index:50}
    #shade{background:#101010;z-index:50}
    #viewer{position:fixed;left:60px;top:40px;z-index:50}
    #viewer img{display:block;width:150px;height:155px}
  `, `<header id="header" data-app-shell-titlebar>Native title</header>
    <div id="portal" class="pointer-events-none fixed inset-0 contain-layout">
      <div style="isolation:isolate">
        <div id="shade" class="codex-dialog-overlay imageOverlay-_cSMvD fixed inset-0" data-transparent="true"></div>
        <div id="viewer" class="codex-dialog" role="dialog"><img src="${assets.photo}"></div>
      </div>
    </div>`);
  await configure(page, { dataUrl: assets.pattern });
  const frame = await shot(page);
  near(pixel(frame, 20, 300), [16, 16, 16], "viewer dimmer is not tinted by the wall");
  near(pixel(frame, 100, 80), [220, 40, 50], "viewer image keeps native pixels");
  assert.equal(await page.locator("#shade").getAttribute(native), "viewer");
  await page.$eval("#portal", n => n.replaceChildren()); await settle(page);
  assert.deepEqual(await css(page, "#portal", ["zIndex"]), ["50"]);
});

check("archive sticky controls do not leave a dark gradient over wallpaper", async page => {
  await html(page, `
    body{background:#303030}
    #header{position:sticky;top:0;height:80px;z-index:30;background:#303030}
    #header::after{content:"";position:absolute;top:100%;left:0;right:0;height:32px;pointer-events:none;background:linear-gradient(#303030,transparent)}
  `, '<div id="header" class="sticky z-30 bg-surface after:bg-linear-to-b after:from-surface after:to-transparent">Archived chats</div>');
  await configure(page);
  const frame = await shot(page);
  for (const y of [81, 90, 105]) near(pixel(frame, 400, y), pixel(frame, 400, 140),
    "the native sticky fade reveals one continuous wallpaper");
  assert.deepEqual(await css(page, "#header", ["filter", "backdropFilter"]), ["none", "none"]);
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup()); await settle(page);
  assert.match(await page.$eval("#header", node => getComputedStyle(node, "::after").backgroundImage), /linear-gradient/,
    "disabling the overlay restores the native gradient");
});

check("popup batches commit one stylesheet before paint", async page => {
  await html(page, '.menu{position:fixed;left:20px;top:20px;width:160px;height:100px;background:rgb(240,240,240)}',
    Array.from({ length: 24 }, () => '<div class="menu" role="menu"><button>Option</button></div>').join(""));
  await configure(page);
  const initial = await page.evaluate(() => window.work.styleCommits);
  assert.ok(initial <= 3, "initial stylesheet commits: " + initial);
  const work = await delta(page, () => page.evaluate(() => {
    document.querySelectorAll('.menu').forEach(node => { node.style.backgroundColor = "rgb(100,150,200)"; });
  }));
  assert.equal(work.styleCommits, 1, "one commit for all popup material updates");
  assert.equal(await page.locator('[' + material + ']').count(), 24);
  near(pixel(await shot(page), 150, 90), [100, 150, 200]);
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
  await settle(page);
  assert.equal(await page.locator('[data-codex-plus-ext="image-overlay"]').count(), 0);
});

check("stale cache is replaced and pending rules cannot resurrect retired styles", async page => {
  await thread(page, 1);
  await page.evaluate(() => {
    window.oldOverlay = document.getElementById("codex-plus-image-overlay");
    window.oldResource = window.__codexPlusImageOverlayResource;
    window.oldResource.version = 0;
    document.getElementById("menu").style.backgroundColor = "red";
    queueMicrotask(() => installCodexPlusImageOverlay());
  });
  await settle(page);
  assert.ok(await page.evaluate(() => document.getElementById("codex-plus-image-overlay") !== window.oldOverlay &&
    window.__codexPlusImageOverlayResource !== window.oldResource && !window.oldOverlay.isConnected));
  assert.equal(await page.locator("#codex-plus-image-overlay").count(), 1);
  assert.equal(await page.locator('style[data-codex-plus-ext="image-overlay"]').count(), 1);
  near(pixel(await shot(page), 40, 40), [220, 40, 50]);
});

check("unchanged backdrop keys allocate no SVG primitives", async page => {
  await html(page,
    '#photo{position:absolute;left:20px;top:20px;width:180px;height:180px}' +
    '#button{position:absolute;left:80px;top:80px;width:100px;height:60px;z-index:10;' +
    'background:rgba(240,240,240,.5);backdrop-filter:blur(4px);border:0}',
    image() + '<button id="button">Action</button>');
  await configure(page);
  assert.match((await css(page, "#button", ["backdropFilter"]))[0], /url/);
  const before = await shot(page);
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) {
      window.dispatchEvent(new Event("resize"));
      await new Promise(requestAnimationFrame);
    }
  }));
  assert.equal(work.frames, 12);
  assert.equal(work.filterNodes, 0, "cached backdrop must not allocate discarded primitives");
  assert.equal(work.styleCommits, 0);
  near(pixel(await shot(page), 100, 95), pixel(before, 100, 95));
  const changed = await delta(page, () => page.$eval("#photo", (node, src) => { node.src = src; }, assets.pattern));
  assert.ok(changed.filterNodes > 0, "source change still rebuilds the backdrop");
  await click(page, "#button");
});

// Replaces overlapping layer-order, popup, live-surface and recovery fixtures.
for (const width of [480, 900]) check("native composer/media/delete geometry " + width, async page => {
  await html(page, `
    #frame{position:relative;height:700px;container-type:inline-size}#thread{height:700px;overflow:auto}
    #content{height:1200px;padding:30px}img{width:80px;height:80px}
    #footer{position:absolute;bottom:40px;left:20px;width:calc(100% - 140px);height:180px;z-index:20}
    #composer{position:relative;height:180px;background:#fafafa;border-radius:12px}
    #attachment{position:relative;top:20px;left:20px;width:80px;height:80px;transform:translate(0)}
    #remove{position:absolute;top:-8px;right:-8px;z-index:10;width:28px;height:28px;background:#00ff00;border:0}
    #editor{position:absolute;top:120px;left:20px;width:80%;color:black}
    #side{position:absolute;right:0;top:30px;contain:layout paint;isolation:isolate}
  `, `<div id="frame"><div id="thread" class="thread-scroll-container"><div id="content">${image()}</div>
    <div id="footer" data-thread-scroll-footer><div id="composer" class="_ComposerLayoutRoot_test">
    <div id="attachment">${image("attached")}<button id="remove"></button></div>
    <div id="editor" contenteditable>Editor</div></div></div></div><div id="side">${image("side-photo")}</div></div>`);
  const ids = ["footer", "photo", "attached", "remove", "side-photo"];
  const before = await Promise.all(ids.map(id => rect(page, "#" + id)));
  await configure(page);
  assert.deepEqual(await Promise.all(ids.map(id => rect(page, "#" + id))), before);
  const screen = await shot(page);
  for (const index of [1, 2, 4]) { const [x, y, w, h] = before[index]; near(pixel(screen, x + w / 2, y + h / 2), [220, 40, 50]); }
  const [x, y, w] = before[3]; near(pixel(screen, x + w - 4, y + 4), [0, 255, 0]);
  await click(page, "#remove");
  await page.locator("#editor").pressSequentially(" typing");
  assert.match(await page.locator("#editor").innerText(), /typing/);
  assert.deepEqual(await css(page, "#editor", ["filter"]), ["none"]);
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
  assert.deepEqual(await Promise.all(ids.map(id => rect(page, "#" + id))), before);
}, { width, height: 720 });

for (const kind of ["menu", "listbox", "tooltip", "dialog", "roleless"])
  for (const opacity of [.2, .75]) check(`popup alpha/ordering/motion ${kind}/${opacity}`, async page => {
    await popup(page, kind, opacity);
    assert.ok(await page.$eval("#surface", (node, attr) => node.hasAttribute(attr), material));
    const actual = await shot(page);
    await page.evaluate(opacity => {
      window.__codexPlusImageOverlayCleanup(true);
      const wall = document.createElement("div"); wall.id = "reference";
      wall.style.cssText = `position:fixed;inset:0;pointer-events:none;z-index:40;background:#2850c8;opacity:${opacity}`;
      document.body.append(wall);
      document.getElementById("photo").style.zIndex = "45";
      Object.assign(document.getElementById("surface").style, { backdropFilter: "none", background: `rgba(240,240,240,${1-opacity})` });
    }, opacity);
    const expected = await shot(page);
    for (const [x, y] of [[60, 180], [240, 180], [90, 65]]) near(pixel(actual, x, y), pixel(expected, x, y));
    await page.evaluate(() => {
      document.getElementById("reference").remove();
      document.getElementById("photo").style.removeProperty("z-index");
      document.getElementById("surface").removeAttribute("style");
    });
    await configure(page, { opacity });
    const before = await css(page, "#surface", ["backgroundColor"]);
    const work = await delta(page, () => page.evaluate(async () => {
      for (let i = 0; i < 12; i++) {
        document.getElementById("boundary").style.transform = `translate(${7+i}px,11px)`;
        document.getElementById("option").className = "hover-" + i;
        document.getElementById("range").value = String(i);
        document.getElementById("list").scrollTop = i;
        await new Promise(requestAnimationFrame);
      }
    }));
    zero(work, ["rectReads", "layerWrites"]);
    assert.deepEqual(await css(page, "#surface", ["backgroundColor"]), before);
    for (const selector of ["#surface", "#option", "#range"])
      assert.deepEqual(await css(page, selector, ["filter", "backdropFilter", "backgroundImage"]), ["none", "none", "none"]);
    await click(page, "#option");
    await page.locator("#range").focus(); await page.keyboard.press("ArrowRight");
    assert.equal(await page.locator("#range").inputValue(), "12");
  });

check("settings popup survives overlapping previews, theme changes and reparenting", async page => {
  await popup(page, "menu");
  const before = await css(page, "#surface", ["backgroundColor"]);
  for (let i = 0; i < 12; i++) {
    await page.evaluate(i => {
      document.getElementById("preview")?.remove();
      if (i % 2 === 0) {
        const el = document.createElement("div"); el.id = "preview"; el.setAttribute("role", "tooltip");
        el.style.cssText = "position:fixed;left:400px;top:20px;background:#333"; document.body.append(el);
      }
      document.getElementById("option").className = "hover-" + i;
    }, i);
    await settle(page); assert.deepEqual(await css(page, "#surface", ["backgroundColor"]), before);
  }
  await page.evaluate(() => document.documentElement.style.setProperty("--menu-color", "rgb(60,80,100)"));
  await settle(page); const themed = await css(page, "#surface", ["backgroundColor"]);
  assert.notDeepEqual(themed, before);
  await page.evaluate(() => { document.body.className = "unrelated"; });
  await settle(page); assert.deepEqual(await css(page, "#surface", ["backgroundColor"]), themed);
  await page.evaluate(() => {
    const next = document.createElement("div"); next.setAttribute("data-radix-popper-content-wrapper", "");
    next.style.cssText = "position:fixed;left:50px;top:50px"; document.body.append(next);
    window.oldBoundary = document.getElementById("boundary"); next.append(document.getElementById("shell")); window.oldBoundary.remove();
  });
  await settle(page);
  assert.equal(await page.evaluate(() => window.oldBoundary.hasAttribute("data-codex-plus-image-top-control")), false);
  assert.deepEqual(await css(page, "#surface", ["backgroundColor"]), themed);
});

for (const roleless of [false, true]) check("expanded composer above native review card " + roleless, async page => {
  await html(page, `
    #card{position:absolute;left:20px;top:260px;width:350px;height:90px;background:#3338}
    #header{position:relative;height:90px}#green{position:absolute;left:20px;top:35px;width:12px;height:12px;background:#00ff00}
    #positioner{position:fixed;left:20px;top:420px;width:350px;z-index:50;transform:translateY(-100%)}
    #panel{position:relative;height:200px;background:#2d2d2d}#option{position:absolute;left:180px;top:55px;width:100px;height:45px;background:white}
  `, `<div id="card"><div id="header" class="group/turn-diff-header">Edited files<div id="green"></div></div></div>
    <div id="positioner"><div id="shell" data-composer-overlay-floating-ui><div id="panel" ${roleless ? "" : 'role="listbox"'}><button id="option">MCP</button></div></div></div>`);
  const before = await rect(page, "#panel");
  await configure(page); assert.deepEqual(await rect(page, "#panel"), before);
  assert.deepEqual(await css(page, "#header", ["zIndex"]), ["auto"]);
  await click(page, "#option");
  await page.$eval("#shell", n => { n.hidden = true; }); await settle(page);
  near(pixel(await shot(page), 45, 300), [8, 220, 40]);
  assert.equal(await page.locator("#card [" + control + "]").count(), 0);
  await page.$eval("#shell", n => n.removeAttribute("data-composer-overlay-floating-ui")); await settle(page);
  if (roleless) assert.equal(await page.locator("[" + material + "]").count(), 0);
});

for (const direction of ["ltr", "rtl"]) for (const width of [480, 800])
  check(`native scrollbar/footer qualification ${direction}/${width}`, async page => {
    await html(page, `
      #frame{position:relative;height:500px}#thread{height:500px;overflow:auto;scrollbar-gutter:stable both-edges;direction:${direction}}
      #thread::-webkit-scrollbar{width:16px}#thread::-webkit-scrollbar-thumb{background:#00ff00}
      #content{height:1500px}#footer{position:absolute;left:20px;right:20px;bottom:0;height:100px}
      #footer.full{left:0;right:0}#composer{height:100px;background:white}
    `, '<div id="frame"><div id="thread" class="thread-scroll-container"><div id="content"></div><div id="footer" data-thread-scroll-footer><div id="composer" class="_ComposerLayoutRoot_test"></div></div></div></div>');
    const original = await rect(page, "#footer"); await configure(page);
    assert.deepEqual(await rect(page, "#footer"), original);
    await page.$eval("#footer", n => { n.className = "full"; }); await settle(page);
    assert.deepEqual(await rect(page, "#footer"), [16, 400, width - 32, 100]);
    assert.deepEqual(await css(page, "#footer", ["position"]), ["absolute"]);
    await page.$eval("#thread", n => { n.scrollTop = 1000; }); await settle(page);
    const x = direction === "ltr" ? width - 8 : 8;
    near(pixel(await shot(page), x, 440), [8, 220, 40]);
    await page.mouse.move(x, 440); await page.mouse.down(); await page.mouse.move(x, 350, { steps: 5 }); await page.mouse.up();
    assert.ok(await page.$eval("#thread", n => n.scrollTop < 1000));
    await page.$eval("#footer", n => { n.className = ""; }); await settle(page);
    assert.deepEqual(await rect(page, "#footer"), original);
  }, { width, height: 600 });

check("browser chrome, retained right panes and extension pages", async page => {
  await html(page, `
    #portal{position:fixed;inset:0;pointer-events:none;contain:paint}
    #pane{position:fixed;right:0;top:0;width:300px;pointer-events:auto}
    #chrome{height:60px;background:#333}#web{height:200px;background:#dc2832}
    #page{position:fixed;inset:20px;z-index:2147483644;background:#333;isolation:isolate}
  `, '<div id="portal"><div id="pane" data-app-shell-focus-area="right-panel"><div id="chrome" data-browser-sidebar-toolbar><button id="browser-action">Browser</button></div><div id="web" data-browser-sidebar-webview>Content</div></div></div>');
  await configure(page); near(pixel(await shot(page), 600, 150), [220, 40, 50]);
  await click(page, "#browser-action");
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) { document.getElementById("browser-action").className = "hover" + i; await new Promise(requestAnimationFrame); }
  }));
  zero(work, ["rectReads", "styleReads", "layerWrites"]);
  for (const selector of ["#chrome", "#browser-action"]) assert.deepEqual(await css(page, selector, ["filter"]), ["none"]);
  await page.$eval("#pane", n => { n.inert = true; n.setAttribute("aria-hidden", "true"); }); await settle(page);
  assert.equal(await page.locator("#pane [" + native + "]").count(), 0);
  await page.$eval("#pane", n => { n.inert = false; n.removeAttribute("aria-hidden"); }); await settle(page);
  near(pixel(await shot(page), 600, 150), [220, 40, 50]);
  await page.evaluate(photo => {
    const el = document.createElement("div"); el.id = "page"; el.className = "codex-plus-page-overlay";
    el.innerHTML = `<img src="${photo}"><button id="page-action">Extensions</button>`; document.body.append(el);
  }, assets.photo); await settle(page);
  assert.deepEqual(await css(page, "#page", ["isolation", "zIndex"]), ["isolate", "2147483644"]);
  await click(page, "#page-action"); assert.equal(await page.locator("#page [" + control + "]").count(), 0);
});

check("browser toolbar uses transparent paint without resize work", async page => {
  await html(page, '#portal{position:fixed;inset:0;pointer-events:none;contain:paint}' +
    '#pane{position:fixed;right:0;top:0;width:300px;pointer-events:auto}' +
    '#chrome{height:60px;background:linear-gradient(#333,#555);box-shadow:0 4px 10px black;backdrop-filter:blur(8px)}' +
    '#chrome button{background:#dc2832;color:white;filter:none}#web{height:200px;background:#dc2832}',
    '<div id="portal"><div id="pane"><div id="chrome" data-browser-sidebar-toolbar><button id="action">Browser</button></div><div id="web" data-browser-sidebar-webview></div></div></div>');
  const properties = ["backgroundImage", "boxShadow", "backdropFilter"];
  const original = await css(page, "#chrome", properties), geometry = await rect(page, "#chrome");
  await configure(page);
  assert.deepEqual(await css(page, "#chrome", ["backgroundColor", ...properties, "filter"]),
    ["rgba(0, 0, 0, 0)", "none", "none", "none", "none"]);
  assert.deepEqual(await rect(page, "#chrome"), geometry);
  const rendered = await shot(page); near(pixel(rendered, 700, 40), pixel(rendered, 100, 40));
  near(pixel(rendered, 700, 150), [220, 40, 50]);
  assert.equal(await page.locator('filter,[data-codex-plus-image-paint]').count(), 0);
  assert.deepEqual(await css(page, "#action", ["backgroundColor", "opacity", "filter"]), ["rgb(220, 40, 50)", "1", "none"]);
  await click(page, "#action");
  assert.deepEqual(await page.evaluate(() => [...window.resizeTargets, ...window.intersectionTargets].map(n => n.id)), []);
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) { document.getElementById("pane").style.width = 300 + i + "px"; await new Promise(requestAnimationFrame); }
  })); zero(work, ["frames", "rectReads", "styleReads", "layerWrites"]);
  await page.$eval("#chrome", n => n.removeAttribute("data-browser-sidebar-toolbar")); await settle(page);
  assert.deepEqual(await css(page, "#chrome", properties), original);
  await page.$eval("#chrome", n => n.setAttribute("data-browser-sidebar-toolbar", "")); await settle(page);
  assert.deepEqual(await css(page, "#chrome", ["backgroundColor", "filter", "backdropFilter"]), ["rgba(0, 0, 0, 0)", "none", "none"]);
  await configure(page, { opacity: .6, fitMode: "tile" });
  const changed = await shot(page); near(pixel(changed, 700, 40), pixel(changed, 100, 40));
  await page.evaluate(() => window.__codexPlusImageOverlayCleanup());
  assert.deepEqual(await css(page, "#chrome", properties), original);
});

for (const count of [40, 400]) check("bounded thread scroll and image reentry " + count, async page => {
  await thread(page, count);
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 1; i <= 20; i++) { document.getElementById("thread").scrollTop = i * 3; await new Promise(requestAnimationFrame); }
  }));
  assert.ok(work.rectReads <= 220, JSON.stringify(work)); assert.ok(work.contains < 1600, JSON.stringify(work));
  assert.equal(work.composerReads, 0); near(pixel(await shot(page), 20, 15), [220, 40, 50]);
  for (let i = 0; i < 4; i++) {
    await page.$eval("#thread", n => { n.scrollTop = 600; }); await settle(page);
    assert.equal(await page.locator(".row:first-child button[" + control + "]").count(), 0);
    await page.$eval("#thread", n => { n.scrollTop = 0; }); await settle(page);
    near(pixel(await shot(page), 75, 50), [212, 220, 244]);
  }
  await page.$eval("#thread", n => { n.scrollTop = 600; }); await settle(page);
  await page.$eval(".row button", n => { n.style.background = "#00ff00"; });
  await page.$eval("#thread", n => { n.scrollTop = 0; }); await settle(page);
  near(pixel(await shot(page), 75, 50), [8, 220, 40]); await click(page, ".row:first-child button");
});

check("scrolling back from the true bottom reuses control filter primitives", async page => {
  await thread(page, 400);
  await page.$eval("#thread", node => { node.scrollTop = node.scrollHeight; });
  await settle(page);
  const work = await delta(page, () => page.evaluate(async () => {
    const thread = document.getElementById("thread");
    for (let i = 0; i < 20; i++) {
      thread.scrollTop -= 2;
      await new Promise(requestAnimationFrame);
    }
  }));
  assert.equal(work.filterNodes, 0, "scroll updates coordinates without replacing SVG primitives: " + JSON.stringify(work));
  assert.equal(work.styleCommits, 0, "same active controls need no stylesheet replacement");
  assert.equal(work.composerReads, 0);
  const box = await rect(page, ".row:nth-child(400) button");
  near(pixel(await shot(page), box[0] + 8, box[1] + 8), [212, 220, 244]);
});

check("streaming text and mutation discovery coalesce", async page => {
  await thread(page);
  const text = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 20; i++) { const p = document.createElement("p"); p.textContent = "Streaming"; document.getElementById("stream").append(p); await new Promise(requestAnimationFrame); }
  })); zero(text, ["frames", "queries", "rectReads"]);
  const scope = await delta(page, () => page.evaluate(() => { for (let i = 0; i < 20; i++) document.getElementById("content").className = "state-" + i; }));
  assert.ok(scope.styleReads <= 2);
  const added = await delta(page, () => page.evaluate(() => {
    const root = document.createElement("div"); document.getElementById("stream").append(root);
    for (let i = 0; i < 20; i++) { const row = document.createElement("div"); row.innerHTML = "<span>Text</span>"; root.append(row); }
  })); assert.equal(added.queries, 1);
  const idle = await delta(page, () => page.evaluate(() => installCodexPlusImageOverlay()));
  zero(idle, ["frames", "rectReads", "styleReads", "queries", "layerWrites"]);
});

check("media loads, reparenting, late popup/viewer and stale observers", async page => {
  await thread(page, 400);
  const load = await delta(page, () => page.$eval("#img-399", n => n.dispatchEvent(new Event("load"))));
  assert.equal(load.composerReads, 0);
  await page.evaluate(() => { document.querySelector(".row").style.height = "140px"; document.getElementById("img-0").dispatchEvent(new Event("load")); }); await settle(page);
  near(pixel(await shot(page), 20, 150), [220, 40, 50]); await click(page, ".row:nth-child(2) button");
  await page.$eval(".row", n => { n.setAttribute("role", "menu"); }); await settle(page);
  const stale = await delta(page, () => page.evaluate(() => window.intersectionCallback([{ target: document.getElementById("img-0"), isIntersecting: true }])));
  zero(stale, ["frames", "rectReads"]);
  await page.evaluate(() => {
    const row = document.querySelector(".row"); row.setAttribute("role", "dialog");
    const dismiss = document.createElement("span"); dismiss.setAttribute("data-testid", "image-preview-dismiss-area"); row.append(dismiss);
  }); await settle(page);
  assert.equal(await page.$eval(".row", (n, a) => n.getAttribute(a), native), "viewer");
  assert.equal(await page.locator("#img-0[" + native + "]").count(), 0);
  await page.evaluate(() => { const row = document.querySelector(".row"); row.removeAttribute("role"); row.querySelector("span").remove(); }); await settle(page);
  assert.equal(await page.locator("#img-0[" + native + "]").count(), 1);
  await page.evaluate(() => { document.getElementById("menu").append(document.querySelector(".row button")); }); await settle(page);
  assert.deepEqual(await css(page, "#menu button:last-child", ["filter"]), ["none"]);
  await click(page, "#menu button:last-child");
  await page.evaluate(() => { window.removed = document.getElementById("composer"); window.removed.remove(); }); await settle(page);
  assert.equal(await page.evaluate(() => [...window.resizeTargets].filter(n => !n.isConnected).length), 0);
  const removed = await delta(page, () => page.evaluate(() => window.resizeCallback([{ target: window.removed }])));
  assert.equal(removed.frames, 0);
});

check("native viewer animation and background resume", async page => {
  await thread(page);
  const background = await delta(page, () => page.evaluate(async () => {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
    document.dispatchEvent(new Event("visibilitychange"));
    for (let i = 0; i < 15; i++) { window.dispatchEvent(new Event("resize")); await new Promise(requestAnimationFrame); }
  })); zero(background, ["frames", "composerReads"]);
  const resumed = await delta(page, () => page.evaluate(() => { delete document.hidden; document.dispatchEvent(new Event("visibilitychange")); }));
  assert.equal(resumed.frames, 1);
  await page.$eval("#viewer", n => { n.style.display = "block"; }); await settle(page);
  const motion = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 12; i++) {
      document.documentElement.className = "_opening_test_" + i;
      document.documentElement.style.setProperty("--image-preview-thumbnail-radius", i + "px");
      document.getElementById("viewed").style.transform = `scale(${1+i/100})`;
      await new Promise(requestAnimationFrame);
    }
  })); zero(motion, ["frames", "rectReads", "styleReads", "markerWrites"]);
  assert.deepEqual(await css(page, "#viewed", ["zIndex"]), ["auto"]);
  await click(page, "#viewer button");
  await page.$eval("#viewer", n => { n.style.display = "none"; }); await settle(page);
  near(pixel(await shot(page), 20, 40), [220, 40, 50]);
  const resource = await page.evaluate(() => window.__codexPlusImageOverlayBlobUrl);
  await configure(page, { opacity: .4 });
  assert.equal(await page.evaluate(() => window.__codexPlusImageOverlayBlobUrl), resource);
});

check("sidebar animation, hover during scroll and edge overscroll", async page => {
  await html(page, `
    #sidebar{position:relative;width:180px;height:400px;overflow:auto}#sidebar button{display:block;width:160px;height:40px}
    #inner{width:180px}#inner img{width:40px;height:40px}#thread{position:absolute;left:220px;top:0;width:300px;height:300px;overflow:auto;overscroll-behavior-y:contain}
  `, `<aside id="sidebar" class="app-shell-left-panel" data-app-action-sidebar-scroll><div id="inner">${image()}
    ${Array.from({ length: 30 }, (_, i) => `<button data-app-action-sidebar-thread-row>Thread ${i}</button>`).join("")}
    </div></aside><div id="thread" data-app-action-timeline-scroll><div style="height:1200px"></div></div>`);
  await configure(page);
  assert.deepEqual(await css(page, "#thread", ["overscrollBehaviorY"]), ["none"]);
  const motion = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 10; i++) { document.getElementById("sidebar").style.width = (180-i*5) + "px"; await new Promise(requestAnimationFrame); }
  })); assert.equal(motion.markerWrites, 0); assert.ok(motion.styleReads < 12);
  await page.evaluate(() => {
    window.hovered = 0; const sidebar = document.getElementById("sidebar");
    sidebar.addEventListener("pointerover", () => { window.hovered++; });
    sidebar.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 30 }));
    sidebar.querySelector("button").dispatchEvent(new PointerEvent("pointerover", { bubbles: true }));
  }); assert.equal(await page.evaluate(() => window.hovered), 0);
  await click(page, "#sidebar button:first-of-type");
  await page.waitForTimeout(200);
  await page.$eval("#sidebar button", n => n.dispatchEvent(new PointerEvent("pointerover", { bubbles: true })));
  assert.equal(await page.evaluate(() => window.hovered) > 0, true);
  const work = await delta(page, () => page.evaluate(async () => {
    for (let i = 0; i < 20; i++) {
      document.getElementById("sidebar").scrollTop = i*10;
      document.getElementById("thread").dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 100 }));
      await new Promise(requestAnimationFrame);
    }
  })); zero(work, ["layerWrites", "styleReads"]);
});
