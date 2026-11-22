const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const { before, after, test } = require("node:test");
const { chromium } = require("playwright");
const { PNG } = require("pngjs");
const asset = (file) => readFileSync(path.join(__dirname, "../assets/inject", file), "utf8");
let browser;
before(async () => { browser = await chromium.launch({ executablePath: process.env.CODEX_TEST_BROWSER_PATH }); });
after(async () => { await browser?.close(); });

test("图标导航与项目侧栏共享显示偏好，保留设置入口", async () => {
  const page = await browser.newPage();
  try {
    await page.setContent(`<nav data-app-navigation-rail><button data-sidebar-destination="builtin:projects" aria-label="Projects"></button><button data-sidebar-destination="builtin:automations" aria-label="Scheduled"></button><button data-sidebar-destination="builtin:settings">Settings</button></nav>
      <div data-app-action-sidebar-scroll><button data-sidebar-destination="builtin:automations">Scheduled</button><section data-app-action-sidebar-section-heading="Recents">Chats</section></div><main></main>`);
    await page.addScriptTag({content: asset("renderer-inject/96-daily-ui.js")});
    await page.evaluate(() => {
      const preferences = new Map();
      window.readCodexAppSetting = async () => null;
      codexPlusDailyUiExtension({storage:{get:(key, fallback) => preferences.get(key) ?? fallback, set:(key, value) => preferences.set(key, value), remove:key=>preferences.delete(key), keys:()=>[...preferences.keys()]},
        registerSettingsSection: section => { window.renderDaily = () => section.render({container:document.querySelector("main")}); },
        onScan() {}, listen() {}, effect: callback => callback()});
      window.renderDaily();
    });
    await page.getByRole("switch", {name:"Scheduled", exact:true}).click();
    assert.equal(await page.locator('[data-sidebar-destination="builtin:automations"]:visible').count(), 0);
    assert.equal(await page.locator('[data-sidebar-destination="builtin:settings"]:visible').count(), 1);
    await page.getByRole("button", {name:"恢复全部侧栏项目"}).click();
    assert.equal(await page.locator('[data-sidebar-destination="builtin:automations"]:visible').count(), 2);
  } finally { await page.close(); }
});

test("书签按会话和精确锚点定位，远端会话沿用其原生入口", async () => {
  const page = await browser.newPage();
  try {
    await page.setContent('<main><div class="thread-scroll-container" style="overflow:auto;height:160px"><div style="height:400px"></div><div data-content-search-unit-key="turn:question">same text</div><div style="height:500px"></div><div data-above-composer-conversation-id></div></div></main>');
    await page.addScriptTag({content: asset("renderer-inject/96-daily-ui.js")});
    const result = await page.evaluate(async () => {
      const first = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
      const other = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
      const scroller = document.querySelector(".thread-scroll-container");
      const marker = scroller.querySelector("[data-above-composer-conversation-id]");
      const element = scroller.querySelector("[data-content-search-unit-key]");
      window.currentThreadScroller = () => scroller;
      window.currentSessionRef = () => ({session_id: first, host_id: "remote:test"});
      window.normalizedCodexThreadUuid = value => /^[a-f\d-]{36}$/.test(value) ? value : "";
      window.cancelThreadScrollRestoreForUserIntent = () => {};
      window.threadScrollIsReversed = () => false;
      let route = null;
      window.addEventListener("message", event => { route = event.data.path; });
      marker.setAttribute("data-above-composer-conversation-id", first);
      const mark = dailyUiBookmark({element, id: "turn:question", role: "user", text: "same text"});
      marker.setAttribute("data-above-composer-conversation-id", other);
      const another = dailyUiBookmark({element, id: "turn:question", role: "user", text: "same text"});
      const row = document.createElement("button");
      let clicked = false;
      row.addEventListener("click", () => { clicked = true; marker.setAttribute("data-above-composer-conversation-id", first); });
      window.sessionRows = () => [row];
      window.sessionRefFromRow = () => ({session_id:first, host_id:"remote:test"});
      dailyUiNavigateToMark(mark);
      await new Promise(resolve => dailyUiScrollToMark(mark, resolve));
      const target = element.getBoundingClientRect(), frame = scroller.getBoundingClientRect();
      return {distinct: dailyUiBookmarkKey(mark) !== dailyUiBookmarkKey(another), clicked, route,
        centered: Math.abs((target.top + target.bottom - frame.top - frame.bottom) / 2) < 2};
    });
    assert.deepEqual(result, { distinct:true, clicked:true, route:null, centered:true });
  } finally { await page.close(); }
});

test("视频在主窗口背层实际绘制，系统减少动态效果、换页和清理生效", async () => {
  const page = await browser.newPage({viewport:{width:320,height:240}});
  try {
    await page.setContent('<style>body{margin:0}main{height:240px;background:white;position:relative}button{position:relative}</style><main data-app-shell-main-surface><button>Content</button></main>');
    await page.addStyleTag({content:asset("dream-skin-native-shell-compat.css")});
    await page.evaluate(async () => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 32;
      const brush = canvas.getContext("2d");
      const stream = canvas.captureStream(12);
      const recorder = new MediaRecorder(stream, {mimeType:"video/webm;codecs=vp8"});
      const chunks = [];
      recorder.ondataavailable = event => chunks.push(event.data);
      const stopped = new Promise(resolve => { recorder.onstop = resolve; });
      recorder.start();
      for (let frame = 0; frame < 8; frame++) {
        brush.fillStyle = frame % 2 === 0 ? "#e71018" : "#ef1522";
        brush.fillRect(0, 0, 32, 32);
        await new Promise(resolve => setTimeout(resolve, 90));
      }
      recorder.stop();
      await stopped;
      stream.getTracks().forEach(track => track.stop());
      window.__CODEX_PLUS_DREAM_SKIN_ART__ = await new Promise(resolve => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result);
        reader.readAsDataURL(new Blob(chunks, {type:"video/webm"}));
      });
      window.state = {cleanup() { window.cleaned = true; }};
    });
    await page.evaluate(asset("dream-skin-local-video.js"));
    const video = page.locator("main > video");
    await page.waitForFunction(() => document.querySelector("video")?.currentTime > .05);
    const capture = PNG.sync.read(await page.screenshot());
    const offset = (capture.width * 120 + 160) * 4;
    assert.ok(capture.data[offset] - capture.data[offset + 1] > 60, "video frame is visible on the main surface");
    assert.equal(await page.evaluate(() => document.elementFromPoint(8, 8).tagName), "BUTTON");
    await page.emulateMedia({reducedMotion:"reduce"});
    await page.waitForFunction(() => document.querySelector("video").paused);
    await page.evaluate(() => document.querySelector("main").replaceWith(Object.assign(document.createElement("main"), {className:"main-surface"})));
    await video.waitFor({state:"attached"});
    assert.equal(await video.count(), 1);
    await page.evaluate(() => window.state.cleanup());
    assert.equal(await video.count(), 0);
    assert.equal(await page.evaluate(() => window.cleaned), true);
  } finally { await page.close(); }
});
