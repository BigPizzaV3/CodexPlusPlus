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

it("preserves overlay opt-in and integration contracts", () => {
  for (const name of ["__codexPlusImageOverlayReady", "__codexPlusImageOverlayCleanup",
    "__codexPlusImageOverlayResource", "__codexPlusImageOverlayStats"]) assert.ok(install.includes(name));
  assert.match(install, /if \(!config\.enabled \|\| !source \|\| !root\) return/);
  assert.match(install, /sendCodexPlusDiagnostic\("image_overlay_installed"/);
  assert.doesNotMatch(source, /function installCodexPlusImageOverlayForeground|function codexPlusSettingsGuestWallpaper/);
});
