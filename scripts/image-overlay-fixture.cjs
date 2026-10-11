const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { before, after, test } = require("node:test");
const { execFileSync } = require("node:child_process");
const { chromium } = require("playwright");
const sharp = require("sharp");
const { PNG } = require("pngjs");
const repo = path.resolve(__dirname, "..");
const source = process.env.CODEX_OVERLAY_CANDIDATE_FUNCTION
  ? fs.readFileSync(process.env.CODEX_OVERLAY_CANDIDATE_FUNCTION, "utf8")
  : process.env.CODEX_OVERLAY_BASELINE
  ? execFileSync("git", ["show", process.env.CODEX_OVERLAY_BASELINE + ":assets/inject/renderer-inject.js"],
    { cwd: repo, encoding: "utf8", maxBuffer: 10 * 1024 * 1024 })
  : fs.readFileSync(path.join(repo, "assets/inject/renderer-inject.js"), "utf8");
const install = source.match(/^  function installCodexPlusImageOverlay\([^]*?^  \}/m)[0];
const injection = 'const codexPlusImageOverlayId="codex-plus-image-overlay";const sendCodexPlusDiagnostic=()=>{};' + install;
const assets = {};
let browser, started = false;
const bitmap = async color => "data:image/png;base64," + (await sharp({
  create: { width: 80, height: 80, channels: 4, background: color },
}).png().toBuffer()).toString("base64");
async function start() {
  assets.wall = await bitmap("#2850c8");
  assets.photo = await bitmap("#dc2832");
  assets.pattern = "data:image/png;base64," + (await sharp(Buffer.from(
    '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="120" height="60" fill="#2850c8"/><path d="M0 0H60V60H0Z" fill="#c85a28"/></svg>'
  )).png().toBuffer()).toString("base64");
  browser = await chromium.launch({ channel: process.env.CODEX_TEST_BROWSER || "msedge",
    executablePath: process.env.CODEX_TEST_BROWSER_PATH,
    headless: true, ignoreDefaultArgs: ["--hide-scrollbars"] });
}
const settle = page => page.waitForTimeout(100);
const shot = async page => PNG.sync.read(await page.screenshot());
const pixel = (image, x, y) => {
  x = Math.floor(x); y = Math.floor(y);
  assert.ok(x >= 0 && y >= 0 && x < image.width && y < image.height, "pixel inside viewport");
  return [...image.data.subarray((y * image.width + x) * 4, (y * image.width + x) * 4 + 3)];
};
const near = (actual, expected, label = "pixel") => assert.ok(actual.every((v, i) => Math.abs(v - expected[i]) <= 1),
  label + ": " + actual + " != " + expected);
const rect = (page, selector) => page.$eval(selector, node => {
  const r = node.getBoundingClientRect(); return [r.x, r.y, r.width, r.height];
});
const css = (page, selector, properties) => page.$eval(selector, (node, keys) =>
  keys.map(key => getComputedStyle(node)[key]), properties);
const html = (page, style, body) => page.setContent(
  '<style>body{margin:0;background:#181818;color:white;font:16px Arial}' + style + '</style>' + body);
async function configure(page, options = {}) {
  await page.evaluate(injection);
  const timing = await page.evaluate(async config => {
    window.__CODEX_PLUS_IMAGE_OVERLAY__ = config;
    const start = performance.now();
    installCodexPlusImageOverlay();
    return { ready: await window.__codexPlusImageOverlayReady, milliseconds: performance.now() - start };
  }, { enabled: true, dataUrl: assets.wall, fitMode: "stretch", opacity: .2, ...options });
  assert.ok(timing.ready, "valid overlay becomes ready");
  await settle(page);
  return timing.milliseconds;
}
async function delta(page, run) {
  const before = await page.evaluate(() => window.take());
  await run(); await settle(page);
  const after = await page.evaluate(() => window.take());
  return Object.fromEntries(Object.entries(after).map(([key, value]) => [key, value - before[key]]));
}
function check(name, run, viewport = { width: 800, height: 600 }, pageOptions = {}) {
  if (!started) { started = true; before(start); after(async () => { await browser?.close(); }); }
  test(name, async () => {
    const page = await browser.newPage({ viewport, ...pageOptions });
    page.setDefaultTimeout(5000);
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.evaluate(() => {
      window.work = { queries: 0, contains: 0, composerReads: 0, markerWrites: 0, styleCommits: 0, filterNodes: 0 };
      const textContent = Object.getOwnPropertyDescriptor(Node.prototype, "textContent");
      Object.defineProperty(Node.prototype, "textContent", { ...textContent, set(value) {
        if (this instanceof HTMLStyleElement && this.dataset.codexPlusExt === "image-overlay") window.work.styleCommits++;
        textContent.set.call(this, value);
      } });
      const createElementNS = Document.prototype.createElementNS;
      Document.prototype.createElementNS = function (ns, tag, ...args) {
        if (ns === "http://www.w3.org/2000/svg" && tag.startsWith("fe")) window.work.filterNodes++;
        return createElementNS.call(this, ns, tag, ...args);
      };
      window.resizeTargets = new Set();
      window.intersectionTargets = new Set();
      for (const [prototype, method, key] of [[Element.prototype, "querySelector", "queries"],
        [Element.prototype, "querySelectorAll", "queries"], [Node.prototype, "contains", "contains"]]) {
        const original = prototype[method];
        prototype[method] = function (...args) { window.work[key]++; return original.apply(this, args); };
      }
      const bounds = Element.prototype.getBoundingClientRect;
      Element.prototype.getBoundingClientRect = function (...args) {
        if (this.id === "composer") window.work.composerReads++;
        return bounds.apply(this, args);
      };
      const Resize = ResizeObserver, Intersection = IntersectionObserver;
      window.ResizeObserver = class extends Resize {
        constructor(callback) { super(callback); window.resizeCallback = callback; }
        observe(node, ...args) { window.resizeTargets.add(node); super.observe(node, ...args); }
        unobserve(node) { window.resizeTargets.delete(node); super.unobserve(node); }
        disconnect() { window.resizeTargets.clear(); super.disconnect(); }
      };
      window.IntersectionObserver = class extends Intersection {
        constructor(callback, options) { super(callback, options); window.intersectionCallback = callback; }
        observe(node, ...args) { window.intersectionTargets.add(node); super.observe(node, ...args); }
        unobserve(node) { window.intersectionTargets.delete(node); super.unobserve(node); }
        disconnect() { window.intersectionTargets.clear(); super.disconnect(); }
      };
      new MutationObserver(entries => {
        window.work.markerWrites += entries.filter(e => e.attributeName?.startsWith("data-codex-plus-image")).length;
      }).observe(document.documentElement, { subtree: true, attributes: true });
      window.take = () => ({ ...window.__codexPlusImageOverlayStats, ...window.work });
      window.nativeAnimate = Element.prototype.animate;
      window.nativeTransition = document.startViewTransition;
    });
    try {
      await run(page);
      assert.deepEqual(errors, []);
      await page.evaluate(() => window.__codexPlusImageOverlayCleanup?.());
      assert.equal(await page.locator('[data-codex-plus-ext="image-overlay"],[data-codex-plus-image-native],[data-codex-plus-image-control],[data-codex-plus-image-scope],[data-codex-plus-image-paint],[data-codex-plus-image-popup-material],[data-codex-plus-image-top-control],[data-codex-plus-image-footer-gutter]').count(), 0);
      assert.equal(await page.evaluate(() => window.resizeTargets.size), 0);
      assert.equal(await page.evaluate(() => window.intersectionTargets.size), 0);
      assert.ok(await page.evaluate(() => Element.prototype.animate === window.nativeAnimate &&
        document.startViewTransition === window.nativeTransition));
    } finally { await page.close(); }
  });
}
module.exports = { assert, assets, bitmap, check, chromium, configure, css, delta, html, injection, near, pixel, rect, settle, sharp, shot };
