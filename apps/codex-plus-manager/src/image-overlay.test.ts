import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";

const source = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

function functionSource(name: string) {
  const match = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"));
  assert.ok(match, name);
  return match[0];
}

it("keeps the complete upstream tint layer and its configured opacity", () => {
  const install = functionSource("installCodexPlusImageOverlay");
  assert.match(install, /width: "100vw"/);
  assert.match(install, /height: "100vh"/);
  assert.match(install, /opacity: String\(opacity\)/);
  assert.match(install, /zIndex: "2147483646"/);
  assert.match(install, /pointerEvents: "none"/);
  assert.doesNotMatch(install, /clipPath|clip-path|maskImage|mask-image/);
});

it("does not rewrite native palettes, foreground paint, or the ordinary app shell", () => {
  const foreground = functionSource("installCodexPlusImageOverlayForeground");
  assert.doesNotMatch(foreground, /styleSheets|cssRules/);
  assert.doesNotMatch(foreground, /--composer-[\w-]+\s*:/);
  assert.doesNotMatch(foreground, /document\.querySelectorAll\([^)]*body/);
  assert.match(foreground, /img, video, canvas/);
  assert.match(foreground, /data-browser-sidebar-webview/);
  assert.match(foreground, /image-preview-dismiss-area/);
});

it("uses bounded media geometry updates and restores every owned resource", () => {
  const foreground = functionSource("installCodexPlusImageOverlayForeground");
  assert.match(foreground, /getBoundingClientRect/);
  assert.match(foreground, /requestAnimationFrame/);
  assert.match(foreground, /addEventListener\("scroll"/);
  assert.match(foreground, /new ResizeObserver/);
  assert.match(foreground, /record\.plane\.remove\(\)/);
  assert.match(foreground, /resizeObserver\.disconnect\(\)/);
  assert.match(foreground, /target\.removeEventListener/);
  assert.doesNotMatch(foreground, /setInterval|setTimeout/);
});

it("retains native media parents and uses the original video for playback controls", () => {
  const foreground = functionSource("installCodexPlusImageOverlayForeground");
  assert.doesNotMatch(foreground, /anchor-name|positionAnchor|anchor-size|anchor\(/);
  assert.match(foreground, /document\.documentElement\.appendChild\(plane\)/);
  assert.match(foreground, /pointer-events: none/);
  assert.match(foreground, /drawImage\(source/);
  assert.match(foreground, /native\.host\.showPopover\(\)/);
  assert.doesNotMatch(foreground, /appendChild\(source\)|append\(source\)/);
  assert.doesNotMatch(foreground, /copy\.src\s*=\s*record\.source\.currentSrc/);
});
