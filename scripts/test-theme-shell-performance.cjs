// 真实 Chromium 回归：NODE_PATH 中须可解析 playwright；可用 CODEX_TEST_BROWSER_PATH 指定浏览器。
// node scripts/test-theme-shell-performance.cjs
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { chromium } = require("playwright");

const root = path.resolve(__dirname, "..");
const asset = name => fs.readFileSync(path.join(root, "assets/inject", name), "utf8");
const extract = (source, name) => {
  const match = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"));
  assert.ok(match, name);
  return match[0];
};
const homeMarker = "data-dream-has-home-utility-above";
const timelineMarker = "data-dream-has-timeline-composer";
const layoutFixtures = process.env.CODEX_THEME_LAYOUT_CSS_OUTPUT
  ? JSON.parse(fs.readFileSync(process.env.CODEX_THEME_LAYOUT_CSS_OUTPUT, "utf8")) : null;

async function run() {
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CODEX_TEST_BROWSER_PATH });
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    // 只使用仓库自带资源，不访问个人会话或网络。
    const artwork = "data:image/png;base64," + fs.readFileSync(path.join(root, "assets/inject/dream-skin-default.png")).toString("base64");
    for (const platform of ["windows"]) {
      const css = [asset(`upstream/dream-skin/${platform}/dream-skin.css`),
        asset("dream-skin-native-shell-compat.css"), asset("dream-skin-community-compat.css")].join("\n");
      let runtime = asset(`upstream/dream-skin/${platform}/renderer-inject.js`);
      const values = {
        __DREAM_SKIN_VERSION_JSON__: "shell-regression-test",
        __DREAM_SKIN_STYLE_REVISION_JSON__: platform,
        __DREAM_SKIN_PAYLOAD_REVISION_JSON__: platform,
        __DREAM_SKIN_CSS_JSON__: css,
        __DREAM_SKIN_ART_JSON__: artwork,
        __DREAM_SKIN_THEME_JSON__: { id: "shell-test", appearance: "light", art: { taskMode: "full" } },
      };
      for (const [key, value] of Object.entries(values)) runtime = runtime.replaceAll(key, () => JSON.stringify(value));
      await page.setContent(`
        <style>#home-chrome {border-radius: 3px} #backdrop {background:rgb(12,34,56)}</style>
        <aside class="app-shell-left-panel"></aside><header class="app-header-tint"></header>
        <main data-app-shell-main-surface>
          <section id="home" data-codex-composer-root data-composer-placement="home">
            <div id="utility" data-composer-home-utility-bar-position="above"></div>
            <div id="home-chrome" class="composer-surface-chrome"></div>
          </section>
          <section id="timeline" data-app-action-timeline-scroll>
            <div id="footer" class="pointer-events-none absolute">
              <div id="backdrop" class="pointer-events-none absolute bg-surface"></div>
              <div id="thread-chrome" class="composer-surface-chrome"></div>
            </div>
            <div id="decoy"><div class="composer-surface-chrome"></div></div>
          </section>
        </main>`);
      await page.evaluate(runtime);
      await page.evaluate(() => {
        Object.assign(document.documentElement.dataset, {codexOs: "win32", dreamArtWide: "true", dreamTaskMode: "full"});
      });
      const marked = (id, marker, expected) => page.waitForFunction(
        ({id, marker, expected}) => document.getElementById(id).hasAttribute(marker) === expected,
        {id, marker, expected});
      await marked("home", homeMarker, true);
      await marked("footer", timelineMarker, true);
      assert.equal(await page.$eval("#decoy", n => n.hasAttribute("data-dream-has-timeline-composer")), false);
      assert.equal(await page.$eval("#home-chrome", n => getComputedStyle(n).borderTopLeftRadius), "22px");
      assert.equal(await page.$eval("#backdrop", n => getComputedStyle(n).backgroundColor), "rgba(0, 0, 0, 0)");
      const nativeHas = await page.evaluate(() => {
        const selectors = [];
        const walk = rules => { for (const rule of rules) {
          if (rule.selectorText?.includes(":has(")) selectors.push(rule.selectorText);
          if (rule.cssRules) walk(rule.cssRules);
        }};
        walk(window.__CODEX_DREAM_SKIN_STATE__.styleSheet.cssRules);
        return selectors;
      });
      assert.deepEqual(nativeHas, [], `${platform}: no persistent stylesheet :has()`);

      // 条件属性、归属属性、动态类名变化均须由真实 observer 刷新。
      await page.$eval("#utility", n => n.setAttribute("data-composer-home-utility-bar-position", "below"));
      await marked("home", homeMarker, false);
      await page.$eval("#utility", n => n.setAttribute("data-composer-home-utility-bar-position", "above"));
      await marked("home", homeMarker, true);
      await page.$eval("#home", n => n.setAttribute("data-composer-placement", "thread"));
      await marked("home", homeMarker, false);
      await page.$eval("#home", n => n.setAttribute("data-composer-placement", "home"));
      await marked("home", homeMarker, true);
      await page.$eval("#thread-chrome", n => n.className = "unrelated");
      await marked("footer", timelineMarker, false);
      assert.equal(await page.$eval("#backdrop", n => getComputedStyle(n).backgroundColor), "rgb(12, 34, 56)");
      await page.$eval("#thread-chrome", n => n.className = "test_ComposerLayoutRoot_123");
      await marked("footer", timelineMarker, true);
      await page.$eval("#footer", n => n.classList.remove("absolute"));
      await marked("footer", timelineMarker, false);
      await page.$eval("#footer", n => n.classList.add("absolute"));
      await marked("footer", timelineMarker, true);
      await page.$eval("#timeline", n => n.removeAttribute("data-app-action-timeline-scroll"));
      await marked("footer", timelineMarker, false);
      await page.$eval("#timeline", n => n.setAttribute("data-app-action-timeline-scroll", ""));
      await marked("footer", timelineMarker, true);

      // detached 路由也应清理；重新注入后不得留下旧属性/定时器。
      await page.$eval("#footer", n => { window.detachedFooter = n; n.remove(); });
      await page.waitForFunction(() => !window.detachedFooter.hasAttribute("data-dream-has-timeline-composer"));
      await page.evaluate(runtime);
      await page.evaluate(() => window.__CODEX_DREAM_SKIN_STATE__.cleanup());
      assert.equal(await page.locator(`[${homeMarker}], [${timelineMarker}]`).count(), 0);
      assert.equal(await page.evaluate(() => document.adoptedStyleSheets.length), 0);
      console.log(`${platform}: stylesheet, mutation and cleanup regressions passed`);
    }

    // 新版 header 在 main 外，aside 在 workspace 内；同时保留旧版结构。
    for (const structure of ["legacy", "workspace"]) {
      await page.setContent(`<style>@layer theme,base,utilities;
        @layer utilities { #sidebar {margin-top:8px!important} #header {margin-inline-end:4px!important;height:52px!important} }
        #header {background:rgb(11,22,33);backdrop-filter:blur(12px)}
        </style>${structure === "workspace"
          ? '<div data-app-shell-workspace><aside id="sidebar" data-ds-part="sidebar"></aside><main></main></div><header id="header" data-ds-part="header"></header>'
          : '<aside id="sidebar" data-ds-part="sidebar"></aside><main><header id="header" data-ds-part="header"></header></main>'}`);
      await page.addStyleTag({content: asset("dream-skin-native-shell-compat.css")});
      const margins = () => page.evaluate(() => [getComputedStyle(document.getElementById("sidebar")).marginTop,
        getComputedStyle(document.getElementById("header")).marginInlineEnd,
        getComputedStyle(document.getElementById("header")).height]);
      const state = (os, active) => page.evaluate(({os,active}) => {
        Object.assign(document.documentElement.dataset, {codexOs:os, codexWindowChrome:"application-menu"});
        if (active) document.documentElement.dataset.dreamSkin = "active";
        else delete document.documentElement.dataset.dreamSkin;
      }, {os, active});
      await state("win32", true);
      assert.deepEqual(await margins(), ["8px", "4px", "52px"], "no theme layout declaration preserves native geometry");
      assert.deepEqual(await page.$eval("#header", n => [getComputedStyle(n).backgroundColor, getComputedStyle(n).backdropFilter]), ["rgb(11, 22, 33)", "blur(12px)"]);
      if (layoutFixtures) {
        const unchanged = await page.addStyleTag({content: layoutFixtures.unchanged});
        assert.deepEqual(await margins(), ["8px", "4px", "52px"]);
        await unchanged.evaluate(n => n.remove());
        for (const [name, expected] of [["custom", ["6px", "12px", "64px"]], ["aligned", ["0px", "0px", "52px"]]]) {
          const style = await page.addStyleTag({content: layoutFixtures[name]});
          await state("win32", true);
          assert.deepEqual(await margins(), expected, `${structure}/${name}: authored values override native important utilities`);
          await state("darwin", true);
          assert.deepEqual(await margins(), ["8px", "4px", "52px"], "Windows layout declarations are isolated");
          await state("win32", false);
          assert.deepEqual(await margins(), ["8px", "4px", "52px"], "disabled theme restores native geometry");
          await state("win32", true);
          await style.evaluate(n => n.remove());
          assert.deepEqual(await margins(), ["8px", "4px", "52px"], "switching to an unconfigured theme restores defaults");
        }
      }
    }
    console.log(layoutFixtures
      ? "Windows theme layout: actual Rust-compiled CSS, custom values, theme switch, disable and platform isolation passed"
      : "Windows theme layout: native defaults passed; compiled CSS checks require CODEX_THEME_LAYOUT_CSS_OUTPUT");

    const styles = asset("renderer-inject/10-style.js");
    const pages = asset("renderer-inject/40-backend-settings.js");
    await page.setContent('<div id="outside"><div role="menu"></div></div><div class="codex-plus-page-overlay"><div id="inside"><div role="menu"></div></div></div>');
    await page.evaluate(`(() => {
      const styleId = "codex-delete-style";
      const codexPlusIsWindowsPlatform = true;
      const codexPlusPageClass = "codex-plus-page-overlay";
      const codexPlusRailSelector = "nav[data-app-navigation-rail]";
      const codexPlusRailDestinationSelector = "[data-sidebar-destination]";
      ${extract(styles, "syncCodexPlusPagePopupStyle")}
      ${extract(pages, "syncCodexPlusRailNativeSelection")}
      window.syncPage = syncCodexPlusRailNativeSelection;
      window.syncPage();
    })()`);
    assert.equal(await page.$eval("#outside", n => getComputedStyle(n).zIndex), "2147483645");
    assert.equal(await page.$eval("#inside", n => getComputedStyle(n).zIndex), "auto");
    await page.evaluate(() => { window.syncPage(); window.syncPage(); });
    assert.equal(await page.locator("#codex-delete-style-page-popups").count(), 1);
    await page.locator(".codex-plus-page-overlay").evaluate(n => n.remove());
    await page.evaluate(() => window.syncPage());
    assert.equal(await page.locator("#codex-delete-style-page-popups").count(), 0);
    assert.equal(await page.$eval("#outside", n => getComputedStyle(n).zIndex), "auto");
    console.log("Manager popups: parent stacking, internal exclusion and close cleanup passed");
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
