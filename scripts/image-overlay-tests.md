# Image overlay regressions

Run the browser suites with Node and locally available Playwright, Sharp and PNGJS.
Use NODE_PATH for an existing external dependency directory. The shared fixture
accepts CODEX_TEST_BROWSER_PATH or CODEX_TEST_BROWSER (default: msedge).

- node scripts/test-image-overlay.cjs: shared lifecycle, pixel and interaction cases.
- node scripts/test-image-native-media.cjs: transparent images, fit/zoom, backdrop
  composition, frozen-JavaScript scrolling and native view-transition snapshots.
- node scripts/test-image-composite.cjs: whole-frame comparison against the original
  fixed-DIV wallpaper, ordinary iframe/shadow content, video/canvas and resource lifetime.
- node scripts/test-background-resume.cjs: scan scheduler suspension and coalescing.
- node scripts/test-session-action-layout.cjs: sidebar layout read/write batching.

The shared suite replaces the overlapping composer-menu, consolidation, layer-order,
live-surfaces, popup-first-paint/material/paint, refactor-regressions, review-sidebar,
rework-surfaces, runtime-work, stable-composer, thread-performance and viewer-scroll
scripts. Related steps now share one fixture and verify their combined lifecycle.

Coverage includes native composer geometry and editable text; opaque attachments
and clickable deletion; roleless input menus above review cards; popup alpha under
nested previews, theme changes and reparenting; browser/native extension boundaries;
inactive right-pane retirement; LTR/RTL scrollbar hit testing; image reentry, late
viewer registration and stale observer callbacks; streamed text and bounded layout
work; sidebar hover suppression during scrolling; background pause/resume.

Native-boundary regressions cover mention node-view ownership, footer occlusion,
bottom-aligned submenu transforms, browser containment/animated bounds, icon badges
and overlapping logo stacks, plus extension widgets above the composer. Composer
contents retain native stacking and shadows; their background uses a static paint
plane without per-control SVGs. Overlay layers 1000-1003 sit below extension widgets;
Codex++ full-page shells retain their own native layer and wallpaper background.

Each shared case checks JavaScript errors, reversibility, released observers and
unchanged native animation APIs. Pixel tolerance remains one channel value.
These browser fixtures verify rendering contracts and work counts, not real App FPS.

The overlay, native-media and composite suites share image loading, installation
and pixel helpers. Native-media retains its original device scale and fit matrices.
Sidebar comparison embeds the frozen upstream per-row algorithm; ordinary runs
need no private Git commits. CODEX_OVERLAY_BASELINE optionally selects a local
revision for comparisons. Cached-backdrop allocation, batched stylesheet commits
and pending-scan coalescing have explicit work-count assertions.
