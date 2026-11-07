# Image overlay regressions

Run the browser suites with Node and locally available Playwright, Sharp and PNGJS.
Use NODE_PATH for an existing external dependency directory. The shared fixture
accepts CODEX_TEST_BROWSER_PATH or CODEX_TEST_BROWSER (default: msedge).

- node scripts/test-image-overlay.cjs: shared lifecycle, pixel and interaction cases.
- node scripts/test-image-native-surfaces.cjs: current semantic composer attributes,
  legacy/home layouts and browser toolbar capsules with opaque native gradients;
  native media, hover/focus, cleanup and transformed surface alignment.
- node scripts/test-image-native-media.cjs: transparent images, fit/zoom, backdrop
  composition, frozen-JavaScript scrolling and native view-transition snapshots.
- node scripts/test-image-interaction-surfaces.cjs: shared request/approval cards,
  legacy shells, rails, tips and progress capsules in footer/dialog/timeline hosts;
  late ancestor identity, inherited theme variables, nested backgrounds,
  transparent variants and reparenting.
- node scripts/test-image-viewer-surfaces.cjs: image-viewer chrome and composer
  migration, native image pixels, editing/clicking, intrinsic editor resize,
  transition/animation completion, transforms and viewport resize.
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

The native-surface cases reproduce Codex 26.1029's composer attributes and toolbar
materials. They compare empty material pixels against a single fixed wallpaper layer,
while comparing attachments and icons against their untinted native pixels. The
transform cases exercise the native footer and toolbar themselves.
Toolbar lifecycle cases also cover late host designation, nested capsule promotion,
and adding/removing an inline transform without a resize event. A non-identity
transform on an additional shared ancestor that keeps the entire subtree below the
global wallpaper is a pre-existing stacking limitation, not covered as a passing
alignment guarantee by these tests.

Navigation regressions model both native browser hosts and hidden/visible reparenting,
with guest pixels and application header controls checked after each navigation.
Sticky page headers retain their native fades, backing geometry and top insets,
and occlude scrolling icons. Transparent headers do not receive a second wallpaper;
opaque native backing uses static CSS backgrounds, including native pseudo-elements.
Dark/light theme changes and patterned wallpaper remain aligned across header edges.
The below-composer home action bar reveals the global wallpaper without repainting
inside its animated transform; the above-composer project bar keeps its native paint.
True-bottom reverse scrolling must reuse SVG primitives and existing stylesheets.

The overlay, native-media and composite suites share image loading, installation
and pixel helpers. Native-media retains its original device scale and fit matrices.
Sidebar comparison embeds the frozen upstream per-row algorithm; ordinary runs
need no private Git commits. CODEX_OVERLAY_BASELINE optionally selects a local
revision for comparisons. Cached-backdrop allocation, batched stylesheet commits
and pending-scan coalescing have explicit work-count assertions.

The independent image-overlay-regressions.yml workflow runs the six image suites
serially on Windows with the runner's Edge. Pull requests and main pushes trigger
it only for assets/extensions/image-overlay/**, scripts/*image*, or the workflow
itself; workflow_dispatch remains available for manual verification.
Playwright, Sharp and PNGJS are pinned in a temporary directory, outside project
dependencies. Successful cases also check that overlay teardown releases its observers and attributes.

Native annotation attachment pills (`data-composer-attachment-pill`), Sonner
notification cards (`.codex-toast-area [data-sonner-toast] .alert-root`) and native
`Tooltip-*` surfaces have patterned wallpaper pixel checks. Opaque tooltip backing
receives the configured wallpaper strength rather than attenuating an already
blended underlay. Native alpha backing stays unpainted while tooltip contents stay
above the wallpaper. The tests also exercise material/theme changes, late semantic
attributes, generic/native tooltip transitions, notification clicks and icon colors.
Unrelated extension-owned widgets remain excluded; the usage-statusline's browser
`title` tooltip is outside the native-surface adaptation.

The native `group/scroll-to-bottom` button is also checked for wallpaper pixels,
independent CSS translate centering, untinted arrow content, hover/focus, keyboard
and pointer activation, hide/show cycles, late identity, material changes and
remounts. Extension-owned controls with the same class remain excluded.

CI sets CODEX_TEST_SWIFTSHADER=1 to use a consistent ANGLE compositing backend.
Pure software rendering changes 12 antialiased SVG edge pixels at zoom 0.8 on
both the unchanged upstream baseline and this patch. This backend selection keeps
the full-frame comparison and its original tolerance; it does not exclude SVGs.
The composite fixture waits for the first presented video frame before capturing
its native reference, rather than relying on a fixed delay.
Its captureStream source uses a CPU canvas context to avoid Chromium switching
the first frame from I420 to BGRA. The native video must match the source color
and remain a playing, active stream before overlay comparisons run.

Local interaction and viewer surfaces follow #2931's conservative material policy:
opaque colors and full-coverage opaque gradients can receive local wallpaper.
Alpha colors (including modern slash-alpha syntax), transparent/partial gradients
and image backgrounds retain native rendering. Painting a full rectangle over
these would add wallpaper where the parent's existing layer should remain visible;
per-pixel alpha reconstruction is outside this change. A translucent pen housing
therefore stays native, while switching it to an opaque color enables local paint.

Streaming regression: 30 registered cards alongside 20 frames of text, class and
inline-style mutations must cause zero queries and zero style reads. Candidate
ancestors are tracked weakly so unrelated subtrees skip discovery; transparent
candidates, late host identity and reparenting still respond to ancestor themes.
Viewer geometry tests wait for native transition/animation completion before
asserting alignment instead of assuming a fixed 300 ms delay is sufficient.
