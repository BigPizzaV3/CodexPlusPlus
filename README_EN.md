# Codex++

<p align="center">
  <img src="docs/images/codex-plus-plus.png" alt="Codex++ icon" width="160">
</p>

<p align="center">
  <a href="README.md">中文</a> | English
</p>

<p align="center">
  <img alt="Release" src="https://img.shields.io/github/v/release/BigPizzaV3/CodexPlusPlus">
  <img alt="Stars" src="https://img.shields.io/github/stars/BigPizzaV3/CodexPlusPlus">
  <img alt="License" src="https://img.shields.io/github/license/BigPizzaV3/CodexPlusPlus">
  <img alt="Python" src="https://img.shields.io/badge/python-3.11%2B-blue">
</p>

Codex++ is an external enhancement launcher for the Codex App. It does not modify the original installation; it launches Codex externally and injects enhancements through the Chromium DevTools Protocol.

## Quick Start

On Windows, double-click `setup.bat` in the project root and choose:

```text
[1] Install Codex++
```

Then launch from the desktop `Codex++.lnk` shortcut.

Command line:

```bash
python -m pip install -e .
python -m codex_session_delete setup
python -m codex_session_delete launch
```

On macOS:

```bash
python -m codex_session_delete setup
```

This creates `/Applications/Codex++.app`.

## Community and Support

Scan the QR code to join the Codex++ discussion group, report issues, share usage notes, or suggest features:

<img src="docs/images/discussion-group-qr.jpg" alt="Codex++ discussion group QR code" width="260">

If Codex++ has helped you, you can buy me a coffee or send a small tip to support continued maintenance.

<p align="center">
  <img src="docs/images/sponsor-alipay.jpg" alt="Alipay sponsor QR code" width="220">
  <img src="docs/images/sponsor-wechat.jpg" alt="WeChat sponsor QR code" width="220">
</p>

## Sponsor

<table>
  <tr>
    <th>🏆 Sponsor 🏆</th>
  </tr>
  <tr>
    <td>👉 <a href="https://rawchat.cn">RawChat | Codex Relay</a> A long-running Codex relay provider with monthly plans, low-rate calls, high cache-hit performance, Pro/Plus account pools, and dedicated all-day maintenance.</td>
  </tr>
</table>

## Highlights

- Adds a `Codex++` menu to manage enhancement features.
- Plugin entry unlock for API Key mode.
- Forced plugin install when the frontend blocks App unavailable states.
- Session delete with confirmation and undo.
- Markdown export from local rollout files.
- Project move for normal conversations and local projects.
- Conversation Timeline with question markers, hover summaries, and quick jump.
- Provider Sync so historical conversations remain visible after switching `model_provider`.
- Windows shortcuts, uninstall entries, optional watcher takeover, and GitHub Release updates.
- macOS `/Applications/Codex++.app` bundle generation.

## Pain Points and Fixes

In API Key mode, the native Codex plugin entry may require ChatGPT login and remain unavailable:

![Plugin entry unavailable in API Key mode](docs/images/pain-plugin-disabled.png)

The native Codex session list only has archive actions and no real delete button:

![Native session list lacks delete action](docs/images/pain-no-delete-button.png)

After launching through Codex++, the plugin entry is unlocked and a delete button appears when hovering a session:

![Codex++ unlocks plugin entry and adds delete button](docs/images/solution-plugin-and-delete.png)

The top bar shows `Codex++`, backend status, and the settings panel:

![Codex++ backend status indicator](docs/images/backend-status-indicator.png)
![Codex++ settings panel](docs/images/settings-panel.png)

## How It Works

1. Starts the Codex App with CDP flags:
   - `--remote-debugging-port=9229`
   - `--remote-allow-origins=http://127.0.0.1:9229`
2. Starts a local helper service for health checks, settings, export, move, and delete operations.
3. Injects `renderer-inject.js` through CDP.
4. The renderer calls local services through the CDP bridge. Delete/undo HTTP routes are not exposed by default, preventing accidental deletion from unrelated local pages.
5. Codex inherits existing proxy environment variables; if none are set, Codex++ auto-detects common local proxy ports for GitHub resources.

This approach does not modify Codex `app.asar` and does not write DLL files into the Codex installation directory.

## Provider Sync

When `Provider Sync` is enabled, Codex++ synchronizes local session metadata before launch so historical conversations remain visible in Desktop and `/resume` after switching providers.

It aligns rollout files, SQLite thread records, and project path caches. It only fixes visibility metadata and does not rewrite message content. Busy files or SQLite locks are skipped so startup can continue.

## Common Commands

```bash
# Install dependencies
python -m pip install -e .

# Launch
python -m codex_session_delete launch

# Install shortcuts / app bundle
python -m codex_session_delete setup

# Remove
python -m codex_session_delete remove

# Remove logs and backup data too
python -m codex_session_delete remove --remove-data

# Check update / update
python -m codex_session_delete check-update
python -m codex_session_delete update

# Optional Windows watcher takeover
python -m codex_session_delete watch-install
python -m codex_session_delete watch-remove
python -m codex_session_delete watch-disable
python -m codex_session_delete watch-enable
```

Launch with a custom Codex path:

```bash
python -m codex_session_delete launch \
  --app-dir "C:/Program Files/WindowsApps/OpenAI.Codex_xxx/app" \
  --debug-port 9229 \
  --helper-port 57321
```

## Data Locations

- Codex local database: `~/.codex/state_5.sqlite`
- Delete backups: `~/.codex-session-delete/backups`
- Provider Sync backups: `~/.codex/backups_state/provider-sync`
- Hidden launch failure logs: `~/.codex-session-delete/launcher.log`
- Watcher logs: `%USERPROFILE%\.codex-session-delete\watcher.log`

## FAQ

### Double-clicking Codex++ does nothing

Check `%USERPROFILE%\.codex-session-delete\launcher.log`.

Common causes: Codex App is not installed, the app path changed, port 9229 is already in use, or Python is unavailable.

### The Codex++ menu does not appear

Make sure you launched from the `Codex++` shortcut instead of the original Codex entry. You can also check whether Codex has `--remote-debugging-port=9229`.

### Skill recommendations fail to load

If the skills page reports `git fetch failed` or cannot connect to GitHub, your machine likely cannot reach GitHub directly. Codex++ inherits proxy environment variables and auto-detects common local proxy ports. You can also specify one manually:

```powershell
$env:HTTP_PROXY="http://127.0.0.1:7897"
$env:HTTPS_PROXY="http://127.0.0.1:7897"
python -m codex_session_delete launch
```

### Old conversations disappear after switching providers

Open the `Codex++` settings panel, enable `Provider Sync`, then restart Codex++.

## Development

```bash
python -m pip install -e .[test]
python -m pytest -q
```

Project structure:

```text
codex_session_delete/
  cli.py                 CLI entry point
  launcher.py            Launches Codex and injects scripts
  cdp.py                 CDP communication and bridge
  helper_server.py       Local helper service
  storage_adapter.py     Local SQLite delete/undo
  provider_sync.py       Provider Sync
  settings_store.py      Codex++ backend settings
  windows_installer.py   Windows shortcuts and uninstall entries
  macos_installer.py     macOS app bundle setup
  watcher.py             Optional Windows watcher takeover
  inject/renderer-inject.js

tests/                   Automated tests
```

## Friendly Links

- [LINUX DO](https://linux.do)

## Contributors and Stars

<a href="https://github.com/BigPizzaV3/CodexPlusPlus/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=BigPizzaV3/CodexPlusPlus" alt="Codex++ contributors">
</a>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=BigPizzaV3/CodexPlusPlus&type=Date&theme=dark">
  <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=BigPizzaV3/CodexPlusPlus&type=Date">
  <img alt="Codex++ Star History" src="https://api.star-history.com/svg?repos=BigPizzaV3/CodexPlusPlus&type=Date">
</picture>

## Notes

Codex++ is an external enhancement tool and does not modify original Codex App files. If a future Codex App update changes page structure, the injection script may need updates.
