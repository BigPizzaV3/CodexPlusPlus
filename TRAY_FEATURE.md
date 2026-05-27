# Codex++ Manager Tray Minimize Feature

This fork is based on [BigPizzaV3/CodexPlusPlus](https://github.com/BigPizzaV3/CodexPlusPlus) v1.1.8.

## What Changed

- The Codex++ Manager window no longer exits when the window close button is clicked.
- Closing the manager window hides it to the Windows system tray.
- Left-clicking the tray icon restores and focuses the manager window.
- The tray icon menu includes:
  - `Open`: restore the manager window.
  - `Exit`: fully close the manager process.

## Implementation Notes

- The feature is implemented in `apps/codex-plus-manager/src-tauri/src/lib.rs`.
- The Tauri `tray-icon` feature is enabled in `apps/codex-plus-manager/src-tauri/Cargo.toml`.
- A regression test was added in `apps/codex-plus-manager/src-tauri/tests/windows_subsystem.rs`.

## Verification

The following checks were run locally:

```powershell
npm run check
npm run vite:build
cargo test -p codex-plus-manager --test windows_subsystem manager_close_button_minimizes_to_system_tray
cargo build -p codex-plus-manager --release --bin codex-plus-plus-manager
```

## License And Attribution

This fork keeps the upstream MIT license and attribution. The original project is maintained at:

https://github.com/BigPizzaV3/CodexPlusPlus
