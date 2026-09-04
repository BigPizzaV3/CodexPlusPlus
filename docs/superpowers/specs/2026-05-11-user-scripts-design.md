# User Scripts Design

## Goal

Add a Codex++ user script system so users can extend the injected renderer behavior with their own JavaScript files without modifying the project source.

## Architecture

Codex++ keeps local file access in Python and browser execution in the existing CDP-injected renderer. Python scans two script directories, stores enablement configuration in the user config directory, injects enabled scripts through CDP, and exposes script inventory/actions through the existing bridge. The renderer only displays the script manager UI and sends configuration/reload requests.

## Script locations

- Built-in scripts directory: `codex_session_delete/user_scripts/`
- User scripts directory: platform user config directory plus `Codex++/user_scripts/`
  - Windows: `%APPDATA%/Codex++/user_scripts/`
  - Non-Windows: existing user config home pattern plus `Codex++/user_scripts/`

The user directory is created automatically. Missing built-in directory is treated as empty. Only top-level `*.js` files are loaded in the first version.

## Configuration

Script settings are persisted as JSON in the user config directory, e.g. `%APPDATA%/Codex++/user_scripts.json`:

```json
{
  "enabled": true,
  "scripts": {
    "builtin:example.js": true,
    "user:my-script.js": false
  }
}
```

Newly discovered scripts default to enabled. The global `enabled` flag controls whether any script executes. Per-script flags control individual scripts. Disabled scripts remain visible in the UI.

## Loading behavior

On launch, Python scans built-in scripts then user scripts, sorted by file name within each source. If the global switch is enabled, only individually enabled scripts are injected. Each script is wrapped so one failure does not stop other scripts and so status is written to `window.__codexPlusUserScripts`.

Reloading scripts from the menu rescans directories and reinjects currently enabled scripts into the current page. Disabling a script does not undo side effects from code that has already run; the UI states that a page reload/restart may be needed to fully remove an already-executed script effect.

## Renderer UI

The Codex++ modal gains a “用户脚本” section:

- Global switch: “启用用户脚本”
- Directory hints for built-in and user script directories
- Script list with name, source, status, and per-script toggle
- “重新加载用户脚本” button
- Short warning that disabling a previously executed script may require page reload/restart

Toggles call bridge routes to update JSON config. Reload calls a bridge route that rescans and injects scripts, then returns inventory/status for rendering.

## Bridge routes

Add routes handled by Python:

- `/user-scripts/list`: returns directories, global enabled flag, scripts, and statuses
- `/user-scripts/set-enabled`: updates the global enabled flag
- `/user-scripts/set-script-enabled`: updates one script flag
- `/user-scripts/reload`: rescans and injects enabled scripts into current page, returning updated status

## Script author contract

A user script is a normal JavaScript file executed in the Codex renderer page context. Scripts should be idempotent if they may be reloaded:

```js
if (window.__myScriptInstalled) return;
window.__myScriptInstalled = true;
```

## Testing

- Python unit tests cover scanning, sorting, config defaults/persistence, enable flags, wrapper generation, and bridge routes.
- CDP tests cover injecting concatenated enabled script wrappers.
- Renderer source-contract tests cover the user script UI, global toggle, per-script toggle, reload button, and warning text.
