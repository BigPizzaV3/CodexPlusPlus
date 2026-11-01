import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";

const source = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
const install = source.match(/^  function installCodexPlusImageOverlay\([^]*?^  \}/m)?.[0];
assert.ok(install);

// Browser suites verify pixels, positioning, lifetimes and bounded work.
// Static checks guard side effects outside those synthetic fixtures.
it("never captures or reparents native media", () => {
  assert.doesNotMatch(install, /cloneNode|captureStream|drawImage|toDataURL/);
  assert.doesNotMatch(install, /node\.style\s*=|node\.style\.(?:setProperty|removeProperty)|appendChild\(node\)|showPopover/);
});

it("leaves native animation APIs and guest documents alone", () => {
  assert.doesNotMatch(install, /document\.startViewTransition|Element\.prototype\.animate|skipTransition/);
  assert.doesNotMatch(install, /styleSheets|cssRules|adoptedStyleSheets|executeJavaScript|setInterval/);
});

// #2813：祖先（含自身）带 transform/translate/scale/rotate 时 background-attachment:fixed 退化为元素局部坐标，
// 侧栏打开时原生 footer 带 translate:-154px，局部重绘的壁纸就会错位。受影响表面改按视口换算尺寸与偏移。
const helperStart = install.indexOf("    const transformedStyle =");
const helperEnd = install.indexOf("    const scopeKey =");
type Rect = [number, number, number, number];
type Helpers = {
  transformedStyle: (paint: Record<string, string>) => boolean;
  wallpaperRect: (mode: string, width: number, height: number, imageWidth: number, imageHeight: number) => Rect | null;
  localWallpaper: (wall: Rect, originX: number, originY: number, sx: number, sy: number) => { size: string; position: string };
  layoutScale: (size: number, layout: number) => number;
};
const helpers = (): Helpers => {
  assert.ok(helperStart > 0 && helperEnd > helperStart, "transform-aware wallpaper helpers are embedded");
  return new Function(`${install.slice(helperStart, helperEnd)}return { transformedStyle, wallpaperRect, localWallpaper, layoutScale };`)() as Helpers;
};
const untransformed = { transform: "none", translate: "none", scale: "none", rotate: "none", willChange: "auto" };

it("treats translate:-154px, translate:0 and transform:translateX(0) ancestors as breaking fixed wallpaper", () => {
  const { transformedStyle } = helpers();
  assert.equal(transformedStyle(untransformed), false);
  assert.equal(transformedStyle({ ...untransformed, filter: "blur(0px)", perspective: "100px", contain: "paint" }), false);
  for (const paint of [
    { translate: "-154px" }, { translate: "0px" }, { transform: "matrix(1, 0, 0, 1, 0, 0)" },
    { scale: "1" }, { rotate: "0deg" }, { willChange: "transform" },
  ]) assert.equal(transformedStyle({ ...untransformed, ...paint }), true, JSON.stringify(paint));
});

it("maps the global wallpaper into a translated footer's local background coordinates", () => {
  const { wallpaperRect, localWallpaper, layoutScale } = helpers();
  // 1200x800 视口、800x600 壁纸：各适配模式下与 overlay 的 background-size/position 一致。
  assert.deepEqual(wallpaperRect("stretch", 1200, 800, 800, 600), [0, 0, 1200, 800]);
  assert.deepEqual(wallpaperRect("tile", 1200, 800, 800, 600), [0, 0, 800, 600]);
  assert.deepEqual(wallpaperRect("center", 1200, 800, 800, 600), [200, 100, 800, 600]);
  assert.deepEqual(wallpaperRect("fill", 1200, 800, 800, 600), [0, -50, 1200, 900]);
  const fit = wallpaperRect("fit", 1200, 800, 800, 600)!;
  assert.deepEqual(fit.map((value) => Math.round(value * 1000) / 1000), [66.667, 0, 1066.667, 800]);
  assert.equal(wallpaperRect("fit", 1200, 800, 0, 0), null, "unknown intrinsic size keeps fixed attachment");
  // 侧栏打开：footer 布局在 x=200，translate:-154px 后视口 x=46；背景定位原点即其 padding box 左上角。
  assert.deepEqual(localWallpaper(fit, 46, 620, 1, 1), { size: "1066.667px 800px", position: "20.667px -620px" });
  // translate:0 / translateX(0) 不移动元素，但同样要按视口换算，不能退回元素局部的 contain。
  assert.deepEqual(localWallpaper(fit, 200, 620, 1, 1), { size: "1066.667px 800px", position: "-133.333px -620px" });
  // 祖先缩放时把视口长度换回元素局部 CSS 像素。
  assert.deepEqual(localWallpaper([0, 0, 800, 600], 234, 398, 0.8, 0.8), { size: "1000px 750px", position: "-292.5px -497.5px" });
  assert.equal(layoutScale(300.5, 301), 1, "subpixel layout rounding is not a transform scale");
  assert.equal(layoutScale(480, 600), 0.8);
});

it("repaints transformed surfaces with scroll attachment and refreshes on native motion", () => {
  assert.match(install, /background-attachment:scroll!important;background-size:\$\{geometry\.size\}!important;background-position:\$\{geometry\.position\}!important;/);
  assert.match(install, /background-attachment:\$\{geometry \? "scroll" : "fixed"\}!important;/);
  // 侧栏开合的 translate 过渡结束后重算；祖先 class/style 突变、尺寸变化、滚动也会触发，但不逐帧轮询。
  for (const event of ["transitionend", "transitioncancel", "animationend"])
    assert.ok(install.includes(`[document, "${event}", onPaintMotionEnd, true]`), event);
  assert.match(install, /if \(invalidatePaintChains\(node\)\) changed = true;/);
  assert.match(install, /resizeObserver\.observe\(paint\.surface\)/);
  assert.doesNotMatch(install, /setInterval|requestAnimationFrame\(refreshPaints\)/);
});

it("preserves overlay opt-in and integration contracts", () => {
  for (const name of ["__codexPlusImageOverlayReady", "__codexPlusImageOverlayCleanup",
    "__codexPlusImageOverlayResource", "__codexPlusImageOverlayStats"]) assert.ok(install.includes(name));
  assert.match(install, /if \(!config\.enabled \|\| !source \|\| !root\) return/);
  assert.match(install, /sendCodexPlusDiagnostic\("image_overlay_installed"/);
  assert.doesNotMatch(source, /function installCodexPlusImageOverlayForeground|function codexPlusSettingsGuestWallpaper/);
});
