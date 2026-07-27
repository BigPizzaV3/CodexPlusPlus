# Dream Skin Auxiliary Panel Background Design

## Goal

Extend the Windows Dream Skin wide-art treatment to Codex's right-side work panel and bottom panel so they visually share the active wallpaper instead of rendering as opaque native surfaces.

## Scope

- Cover the outer surface of the right-side work panel.
- Cover the outer surface of the bottom panel, including the terminal container.
- Apply only while `codex-dream-skin`, `dream-art-wide`, and a supported task presentation mode are active.
- Preserve the existing left sidebar, main task surface, composer, and companion behavior.

## Design

The renderer will identify the current right and bottom auxiliary panel roots from stable structural or accessibility attributes and add Dream Skin-owned marker classes. The existing `ensure()` lifecycle will reconcile those marker classes whenever Codex changes panel visibility or layout.

The Windows Dream Skin stylesheet will use those marker classes to replace only the panel roots' opaque backgrounds with coordinated translucent surfaces derived from the existing task immersive variables. Panel headers and structural wrappers may be made transparent where necessary so the root treatment remains visible. Editors, terminal content, input controls, menus, dialogs, cards, and other functional content surfaces will retain their native or existing themed backgrounds for readability.

When a panel closes or its DOM node is replaced, stale marker classes will be removed during the next reconciliation. Disabling or cleaning up Dream Skin will also remove all auxiliary panel marker classes.

## Alternatives Rejected

- Clearing every `bg-token-main-surface-primary` surface globally would also affect menus, dialogs, and other content that needs an opaque background.
- Painting a new full-window overlay would create stacking, clipping, and pointer-event risks and would duplicate the wallpaper layer already owned by `body`.

## Testing

- Add a regression test that requires the Windows renderer to discover and reconcile both auxiliary panel roles.
- Add a regression test that requires scoped right-panel and bottom-panel selectors in the Dream Skin stylesheet.
- Run the manager test suite, TypeScript check, production frontend build, JavaScript syntax checks, Rust formatting, and the upstream asset hash test in CI.
- Visually verify with both panels open that the wallpaper treatment reaches their outer surfaces while text, terminal content, and controls remain readable.

## Success Criteria

- The right-side work panel no longer appears as a solid unrelated block when Dream Skin wide-art task mode is active.
- The bottom terminal panel no longer appears as a solid unrelated block in the same mode.
- Closing, reopening, resizing, or docking either panel does not disable the skin or leave stale styling behind.
- Menus, dialogs, editors, terminal content, and input controls remain legible and interactive.
