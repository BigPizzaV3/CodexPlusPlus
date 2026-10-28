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

check("native header backing follows its top inset and live theme without seams", async page => {
  await html(page, ':root{--surface:#303030}.light{--surface:#e0e0e0}body{background:var(--surface)}' +
    '#header{position:sticky;top:-16px;z-index:30;height:80px;padding-top:16px;container-type:inline-size}' +
    '#header::before{content:"";position:absolute;z-index:-1;inset:var(--top-inset) 0 0;background:var(--surface)}',
    '<div id="header" class="_shell_test" data-sticky style="--top-inset:8px">Plugins</div>');
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
