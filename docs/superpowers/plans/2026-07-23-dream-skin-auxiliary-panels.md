# Dream Skin Auxiliary Panels Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the active Windows Dream Skin wallpaper treatment to Codex's right-side work panel and bottom terminal panel without making dialogs or functional content surfaces transparent.

**Architecture:** The existing Dream Skin renderer will classify visible `[data-app-shell-tabs="true"]` dock roots by geometry relative to `main.main-surface`. It will mark each dock root and its same-sized layout ancestors with owned role classes; scoped CSS will clear only those structural layers and tint terminal content, while the existing observer reconciles opening, closing, and resizing.

**Tech Stack:** JavaScript renderer injection, CSS, Node.js built-in test runner, Rust asset bundling and SHA-256 snapshot tests.

## Global Constraints

- Apply only while `codex-dream-skin`, `dream-art-wide`, and a supported task presentation mode are active.
- Preserve editors, terminal content, input controls, menus, dialogs, cards, and other functional content surfaces for readability.
- Add no dependencies and do not refactor unrelated theme code.
- Remove stale auxiliary marker classes when panel nodes change or Dream Skin is disabled.

---

### Task 1: Lock Auxiliary Panel Coverage With a Failing Regression Test

**Files:**
- Modify: `apps/codex-plus-manager/src/dream-skin.test.ts`

**Interfaces:**
- Consumes: existing Windows renderer and stylesheet files as UTF-8 text.
- Produces: a regression contract for `dream-aux-panel-layer`, `dream-aux-panel-right`, and `dream-aux-panel-bottom` markers plus scoped terminal styling.

- [ ] **Step 1: Write the failing test**

```ts
it("extends the Windows wallpaper treatment to right and bottom dock panels", async () => {
  const renderer = await readFile(
    new URL("../../../assets/inject/upstream/dream-skin/windows/renderer-inject.js", import.meta.url),
    "utf8",
  );
  const css = await readFile(
    new URL("../../../assets/inject/upstream/dream-skin/windows/dream-skin.css", import.meta.url),
    "utf8",
  );

  assert.match(renderer, /\[data-app-shell-tabs="true"\]/);
  assert.match(renderer, /dream-aux-panel-layer/);
  assert.match(renderer, /dream-aux-panel-right/);
  assert.match(renderer, /dream-aux-panel-bottom/);
  assert.match(renderer, /clearAuxiliaryPanelClasses/);
  assert.match(css, /\.dream-aux-panel-layer/);
  assert.match(css, /\.dream-aux-panel-right/);
  assert.match(css, /\.dream-aux-panel-bottom/);
  assert.match(css, /\[data-codex-terminal="true"\]/);
});
```

- [ ] **Step 2: Run the focused test and verify RED**

Run from `apps/codex-plus-manager` with the bundled Node 24 executable:

```powershell
& 'C:\Users\it.szos_shaoheng.xu\.cache\codex-runtimes\codex-primary-runtime\dependencies\node\bin\node.exe' --test --test-name-pattern "right and bottom dock panels" src/dream-skin.test.ts
```

Expected: FAIL because the auxiliary marker names are absent from the renderer and CSS.

- [ ] **Step 3: Commit the failing test**

```powershell
git add apps/codex-plus-manager/src/dream-skin.test.ts
git commit -m "test: cover Dream Skin auxiliary panels"
```

### Task 2: Reconcile Right and Bottom Dock Panel Layers

**Files:**
- Modify: `assets/inject/upstream/dream-skin/windows/renderer-inject.js`

**Interfaces:**
- Consumes: visible `[data-app-shell-tabs="true"]` roots and the `DOMRect` of `main.main-surface`.
- Produces: `reconcileAuxiliaryPanels(shellMain: Element): void` and `clearAuxiliaryPanelClasses(): void`; owned classes are `dream-aux-panel-layer`, `dream-aux-panel-right`, and `dream-aux-panel-bottom`.

- [ ] **Step 1: Add marker constants and cleanup**

```js
const AUX_PANEL_LAYER_CLASS = "dream-aux-panel-layer";
const AUX_PANEL_RIGHT_CLASS = "dream-aux-panel-right";
const AUX_PANEL_BOTTOM_CLASS = "dream-aux-panel-bottom";
const AUX_PANEL_CLASSES = [AUX_PANEL_LAYER_CLASS, AUX_PANEL_RIGHT_CLASS, AUX_PANEL_BOTTOM_CLASS];

const clearAuxiliaryPanelClasses = () => {
  for (const candidate of document.querySelectorAll(`.${AUX_PANEL_LAYER_CLASS}`)) {
    candidate.classList.remove(...AUX_PANEL_CLASSES);
  }
};
```

Also call `clearAuxiliaryPanelClasses()` from `clearSkinDom()` so disabling the theme removes stale markers.

- [ ] **Step 2: Implement geometry-based reconciliation**

```js
const reconcileAuxiliaryPanels = (shellMain) => {
  const shellRect = shellMain.getBoundingClientRect();
  const activeLayers = new Set();

  for (const tabs of document.querySelectorAll('[data-app-shell-tabs="true"]')) {
    const rect = tabs.getBoundingClientRect();
    if (rect.width < 1 || rect.height < 1) continue;

    const roleClass = rect.top >= shellRect.top + shellRect.height * .5
      && rect.width >= shellRect.width * .65
      ? AUX_PANEL_BOTTOM_CLASS
      : rect.left >= shellRect.left + shellRect.width * .45
        && rect.height >= shellRect.height * .45
        ? AUX_PANEL_RIGHT_CLASS
        : null;
    if (!roleClass) continue;

    for (let layer = tabs, depth = 0; layer && depth < 3; layer = layer.parentElement, depth += 1) {
      const layerRect = layer.getBoundingClientRect();
      if (Math.abs(layerRect.x - rect.x) > 3 || Math.abs(layerRect.y - rect.y) > 3
        || Math.abs(layerRect.width - rect.width) > 3 || Math.abs(layerRect.height - rect.height) > 3) break;
      layer.classList.add(AUX_PANEL_LAYER_CLASS, roleClass);
      activeLayers.add(layer);
    }
  }

  for (const candidate of document.querySelectorAll(`.${AUX_PANEL_LAYER_CLASS}`)) {
    if (!activeLayers.has(candidate)) candidate.classList.remove(...AUX_PANEL_CLASSES);
  }
};
```

Call `reconcileAuxiliaryPanels(shellMain)` from `ensure()` after `shellMain` is available.

- [ ] **Step 3: Run the focused test**

Run the Task 1 command. Expected: FAIL only on the missing CSS selectors.

### Task 3: Paint Scoped Auxiliary Panel Surfaces

**Files:**
- Modify: `assets/inject/upstream/dream-skin/windows/dream-skin.css`

**Interfaces:**
- Consumes: renderer-owned auxiliary panel classes and existing `--dream-task-immersive-*`, `--dream-immersive-line`, and `--dream-surface-raised` variables.
- Produces: transparent dock structure with a readable terminal tint.

- [ ] **Step 1: Add scoped structural and terminal styles**

```css
html.codex-dream-skin.dream-art-wide:is(.dream-task-ambient, .dream-task-banner)
  .dream-aux-panel-layer {
  background: transparent !important;
}

html.codex-dream-skin.dream-art-wide:is(.dream-task-ambient, .dream-task-banner)
  .dream-aux-panel-right {
  border-left-color: var(--dream-immersive-line) !important;
}

html.codex-dream-skin.dream-art-wide:is(.dream-task-ambient, .dream-task-banner)
  .dream-aux-panel-bottom {
  border-top-color: var(--dream-immersive-line) !important;
}

html.codex-dream-skin.dream-art-wide:is(.dream-task-ambient, .dream-task-banner)
  .dream-aux-panel-bottom [data-codex-terminal="true"] {
  background: color-mix(in oklab, var(--dream-surface-raised) 58%, transparent) !important;
}
```

Do not add selectors for dialogs, dropdowns, code blocks, user message bubbles, or composer surfaces.

- [ ] **Step 2: Run the focused test and verify GREEN**

Run the Task 1 command. Expected: PASS.

- [ ] **Step 3: Run JavaScript syntax checks**

```powershell
node --check assets/inject/upstream/dream-skin/windows/renderer-inject.js
node --check assets/inject/renderer-inject.js
```

Expected: both commands exit 0.

### Task 4: Refresh Bundled Asset Revision and Hashes

**Files:**
- Modify: `crates/codex-plus-core/src/assets.rs`
- Modify: `crates/codex-plus-core/tests/upstream_theme_assets.rs`

**Interfaces:**
- Consumes: final Windows renderer and CSS bytes.
- Produces: Dream Skin renderer revision `17` and Windows-checkout SHA-256 expectations for both changed assets.

- [ ] **Step 1: Bump the renderer revision**

Change:

```rust
const DREAM_SKIN_RENDERER_REVISION: &str = "17";
```

- [ ] **Step 2: Compute Windows checkout hashes**

Use Node to read each Git blob, normalize LF to CRLF, and print uppercase SHA-256 values. Update only the two matching entries in `upstream_theme_assets.rs`.

- [ ] **Step 3: Run formatting and snapshot checks**

```powershell
cargo fmt --all -- --check
git diff --check
```

Expected: both commands exit 0. The focused Rust asset test may require GitHub Actions on machines without MSVC `link.exe`.

- [ ] **Step 4: Commit the implementation**

```powershell
git add assets/inject/upstream/dream-skin/windows/renderer-inject.js assets/inject/upstream/dream-skin/windows/dream-skin.css crates/codex-plus-core/src/assets.rs crates/codex-plus-core/tests/upstream_theme_assets.rs
git commit -m "fix: extend Dream Skin across docked panels"
```

### Task 5: Full Verification and PR Update

**Files:**
- Verify only; no new production files.

**Interfaces:**
- Consumes: all changes from Tasks 1-4.
- Produces: local and GitHub evidence that the regression is fixed.

- [ ] **Step 1: Run manager verification**

```powershell
node --test "src/*.test.ts"
node node_modules/typescript/bin/tsc --noEmit
node node_modules/vite/bin/vite.js build
```

Expected: all tests pass, TypeScript exits 0, and Vite completes a production build.

- [ ] **Step 2: Verify live DOM classification**

With both dock panels open, evaluate the current renderer through CDP and assert:

```js
document.querySelectorAll(".dream-aux-panel-right").length >= 1
document.querySelectorAll(".dream-aux-panel-bottom").length >= 1
```

Inspect computed backgrounds for each marked structural layer and confirm they are transparent; confirm `[data-codex-terminal="true"]` uses the configured translucent tint.

- [ ] **Step 3: Verify visually**

Capture the Codex window with both panels open and confirm the wallpaper is visible through their outer surfaces, separators remain visible, content is readable, and no panel overlaps the composer or sidebar.

- [ ] **Step 4: Push and wait for CI**

```powershell
git push fork fix/dream-skin-companion-current-composer
```

Expected: Windows artifacts and both macOS DMG checks complete successfully on PR #1613.
