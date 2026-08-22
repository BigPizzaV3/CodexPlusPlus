# Codex++ Rust/Tauri Migration Design

## Summary

Codex++ will fully replace the current Python backend with Rust and add a Tauri management console. The existing Codex App enhancement model remains: Codex++ launches Codex with CDP flags, injects a bridge plus `renderer-inject.js`, and handles local operations such as delete, undo, export, move, settings, status, provider sync, and user scripts.

The final user experience has two desktop entry points:

- **Codex++**: a no-window silent launcher. Double-clicking it starts enhanced Codex directly.
- **Codex++ 管理工具**: a visible Tauri management app for install, uninstall, update, settings, logs, diagnostics, shortcut repair, and optional manual launch.

There is no separate user-facing CLI. Launch behavior belongs to the silent launcher, and management features belong to the Tauri console.

## Goals

- Replace the Python backend completely with Rust.
- Preserve the existing injected Codex App enhancements and user-facing behavior.
- Add a Tauri management console without making it required for daily launch.
- Generate two desktop entry points: silent launcher and management tool.
- Use one shared Rust core for the silent launcher and Tauri commands.
- Migrate safely by building Rust behavior beside Python first, then removing Python after parity is verified.

## Non-Goals

- Replacing `renderer-inject.js` with a full Tauri-native Codex UI.
- Moving session management into the Tauri app as a standalone conversation browser.
- Providing a separate user-facing CLI.
- Requiring the management tool to stay open while Codex++ is running.

## Architecture

The Rust project will be organized as a workspace:

- `codex-plus-core`
  - Codex app path resolution.
  - Loopback port selection.
  - Codex process launch and lifecycle handling.
  - CDP target discovery, websocket communication, bridge installation, script injection, and DevTools opening.
  - Bridge request routing for injected UI calls.
  - Settings, logs, assets, status, and diagnostics primitives.

- `codex-plus-data`
  - SQLite schema detection.
  - Local delete and undo backup.
  - Workspace move operations.
  - Markdown export.
  - Provider sync across rollout files, SQLite rows, and global state metadata.

- `codex-plus-launcher`
  - No-window silent launcher binary.
  - Supports internal launch configuration such as app path, database path, backup path, debug port, and helper port.
  - Used by the `Codex++` desktop entry point.

- `codex-plus-tauri`
  - Visible management console.
  - Tauri commands call `codex-plus-core`; no duplicated business logic.
  - Provides install, uninstall, update, settings, logs, diagnostics, shortcut repair, and optional manual launch controls.

- `renderer-inject.js`
  - Remains the injected Codex renderer enhancement script.
  - Receives helper/bridge configuration from Rust during injection.
  - Keeps the current Codex App in-place enhancements: menu, delete, undo, export, move, settings panel, timeline, plugin unlocks, user scripts, and ads/sponsor assets.

## Entry Points

### Codex++

The `Codex++` desktop entry point is silent:

1. It starts the no-window Rust launcher.
2. It does not show a Tauri management window.
3. It launches Codex App with CDP flags.
4. It starts the local Rust bridge/helper runtime.
5. It injects `renderer-inject.js`.
6. It stays alive until Codex exits.

### Codex++ 管理工具

The management tool opens a Tauri window and owns management workflows:

- Install and uninstall Codex++ entry points.
- Check for updates and perform updates.
- Edit backend settings.
- View latest launch status.
- Open logs and copy diagnostics.
- Repair or recreate the silent `Codex++` shortcut.
- Optionally launch or repair a running Codex++ session.

The management tool is not required during normal use.

## Runtime Flow

Silent launch flow:

1. User double-clicks `Codex++`.
2. The no-window Rust launcher starts without a visible management UI.
3. Rust chooses available loopback ports for CDP and helper/bridge runtime.
4. If Provider Sync is enabled, Rust updates local Codex metadata before launch.
5. Rust launches Codex with:
   - `--remote-debugging-port=<debug_port>`
   - `--remote-allow-origins=http://127.0.0.1:<debug_port>`
6. Rust starts the helper/bridge runtime.
7. Rust discovers the Codex page target through CDP.
8. Rust installs `Runtime.addBinding`, injects the bridge script, and injects `renderer-inject.js`.
9. Rust evaluates enabled user scripts.
10. Runtime stays alive until Codex exits, then shuts down helper resources.

Bridge request flow:

1. Injected UI calls the bridge from `renderer-inject.js`.
2. CDP delivers a `Runtime.bindingCalled` event to Rust.
3. Rust routes the request by path.
4. `codex-plus-core` and `codex-plus-data` execute the operation.
5. Rust resolves or rejects the browser-side promise.
6. The injected UI shows a toast, updates settings, refreshes status, or updates the current UI state.

Management tool flow:

1. User opens `Codex++ 管理工具`.
2. Tauri UI calls Rust commands.
3. Commands call shared core/data modules.
4. UI displays results for install, update, settings, logs, diagnostics, repair, and launch actions.

## Error Handling And Logging

Silent launch should remain quiet but leave actionable diagnostics.

Automatic recovery:

- Pick another helper or debug port if the requested port is unavailable.
- Retry CDP discovery and injection for a bounded number of attempts.
- Continue detecting common local proxy ports when no proxy environment is configured.
- Avoid killing the current launcher process while cleaning up stale launchers.

Failure recording:

- Write human-readable launch logs.
- Write a latest structured status file for the management tool.
- Include error codes or categories where useful, such as app-not-found, port-unavailable, cdp-target-not-found, injection-failed, sqlite-error, update-error, and shortcut-error.
- Keep operation-level failures visible in the injected Codex UI where the action occurred.

Management diagnostics:

- Show Codex app detection status.
- Show whether shortcuts are installed.
- Show latest launch result and timestamp.
- Show debug/helper ports used by the latest run.
- Show settings file and database path.
- Provide actions to open logs, copy diagnostics, repair shortcuts, retry launch, reset settings, and check updates.

## Installation And Update Behavior

Windows install creates two desktop shortcuts:

- `Codex++.lnk`: silent launcher, no management window.
- `Codex++ 管理工具.lnk`: opens the Tauri management console.

Windows packaging uses separate entry binaries so window behavior is predictable:

- `codex-plus-plus.exe`: no-window silent launcher for the `Codex++` shortcut.
- `codex-plus-plus-manager.exe`: Tauri management console for `Codex++ 管理工具`.

Windows uninstall removes both shortcuts and the uninstall registry entry. Optional data removal deletes Codex++-owned data such as logs, settings, and backups.

macOS install provides equivalent two-entry behavior with two app bundles:

- `Codex++.app`: silent launch wrapper.
- `Codex++ 管理工具.app`: visible management console.

Updates are initiated from the management console. No separate command-line update entry point is provided.

## Management Console UI

The Tauri management console uses a workbench layout rather than a marketing page:

- Left navigation: Overview, Launch, Install, Update, Settings, Logs, Diagnostics.
- Overview: shortcut status, Codex app detection, latest launch result, current version, update status, and quick actions.
- Launch: manual launch button, app path override, debug/helper port settings, and repair backend action.
- Install: install, uninstall, repair shortcuts, and optional remove owned data.
- Update: check update, release summary, download/install progress, and restart guidance.
- Settings: provider sync, Codex command wrapper settings, proxy-related environment summary, and user script enablement.
- Logs: latest launcher/helper logs with copy/open controls.
- Diagnostics: bundled report for issue reporting, including paths, version, OS, shortcuts, ports, settings location, and latest status.

## Migration Strategy

Use a parallel Rust replacement strategy:

1. Add the Rust workspace beside the existing Python package.
2. Port behavior by domain: models/settings, data operations, CDP/bridge, launch lifecycle, install/update/watcher, Tauri management UI.
3. Keep Python tests and modules as behavior references during migration.
4. Add Rust tests and integration fixtures for each migrated domain.
5. Switch entry points to Rust only after launch, bridge, data operations, install/update, watcher, and management UI are verified.
6. Remove Python package files, Python packaging metadata, and obsolete tests after parity is established.
7. Update README and setup instructions to describe the Rust/Tauri distribution.

## Testing Strategy

Core unit tests:

- Settings serialization and persistence.
- Version parsing and update asset selection.
- Path resolution.
- Loopback port selection.
- SQLite schema detection.
- Backup and undo serialization.
- Provider sync rollout transformations.

Integration tests:

- SQLite delete, undo, move, sort key, and markdown export using fixtures.
- Bridge route dispatch.
- CDP target selection and bridge script construction.
- Shortcut/app wrapper generation.
- Updater download and install orchestration with mocked network/process calls.
- Watcher behavior where platform facilities can be safely mocked.

Application checks:

- Tauri management app builds.
- Management window opens and can call status/settings/log commands.
- Silent launcher dry run validates app resolution and configuration.
- Renderer injection remains compatible with Rust bridge responses.
- Windows and macOS packaging produce two visible user entry points.

Python removal gate:

Python files should only be deleted after the Rust implementation passes parity checks for:

- Silent launch.
- Codex App CDP injection.
- Bridge routes.
- Delete and undo.
- Markdown export.
- Session workspace move.
- Provider sync.
- Settings.
- Install and uninstall.
- Update.
- Watcher.
- Management tool.

## Release Artifacts

Release artifacts should include platform installers or archives that contain the silent launcher, management app, injected renderer assets, icons, and required metadata. Exact installer technology can be selected during implementation, but the release must install the two user-facing desktop entry points and keep update actions inside the management console.
