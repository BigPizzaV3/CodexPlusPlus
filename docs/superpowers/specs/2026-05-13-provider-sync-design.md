# Provider Sync Design

## Goal

Add an optional Codex++ startup sync that keeps existing Codex conversations visible after the user changes `model_provider`.

## User experience

Codex++ settings gains a `Provider 同步` toggle. The toggle is off by default.

When the toggle is off, Codex++ launch behavior is unchanged.

When the toggle is on, Codex++ runs provider sync before launching Codex. The sync targets the current root `model_provider` from `~/.codex/config.toml`; it does not switch providers or edit `config.toml`.

If sync is skipped because another sync is locked, SQLite is busy, or session files are in use, Codex++ writes the reason to `~/.codex-session-delete/launcher.log` and continues launching Codex. If sync starts writing and then cannot safely restore or finish, launch fails and the error is written to the launcher log.

## Architecture

Implement the feature inside Codex++ as Python code. Do not depend on the external Node.js CLI from `Dailin521/codex-provider-sync`, because Codex++ should not require Node 24 or a separate global package.

Add three focused areas:

- `codex_session_delete/settings_store.py` stores backend-readable Codex++ settings in `~/.codex-session-delete/settings.json`.
- `codex_session_delete/provider_sync.py` ports the needed provider sync behavior to Python.
- Existing launcher and bridge code call the sync and expose settings to the injected menu.

The renderer menu still uses existing visual patterns, but the new setting is backend-backed rather than only `localStorage`, because startup sync must be readable before Codex launches.

## Settings storage

Store backend settings at:

```text
~/.codex-session-delete/settings.json
```

Initial content is implicit; if the file is missing or malformed, Codex++ uses defaults:

```json
{
  "providerSyncEnabled": false
}
```

Only backend-required settings should live in this file. Existing renderer-only settings remain in `localStorage` unless they must be read before launch.

Expose bridge endpoints:

- `/settings/get` returns merged defaults and saved backend settings.
- `/settings/set` accepts a boolean `providerSyncEnabled`, writes the file atomically, and returns the merged settings.

## Startup flow

`launch_and_inject()` resolves the Codex app directory and ports as it does today. Before `launch_codex_app()`, it loads backend settings. If `providerSyncEnabled` is true, it runs provider sync against the default Codex home `~/.codex`.

The watcher uses the same launcher path, so watcher-triggered launches also honor the setting.

## Sync behavior

Provider sync reads the current provider from root-level `model_provider = "..."` in `~/.codex/config.toml`. If the root setting is absent, the target provider is `openai`.

The sync updates these locations to the target provider:

- First-line metadata records in `~/.codex/sessions/**/rollout-*.jsonl`.
- First-line metadata records in `~/.codex/archived_sessions/**/rollout-*.jsonl`.
- `threads.model_provider` in `~/.codex/state_5.sqlite`.

It also preserves the visibility repairs from the external project where practical:

- If a rollout file contains user events for a thread id, set that SQLite thread row's `has_user_event` to `1` when the column exists.
- If rollout metadata or events expose a thread working directory, update that SQLite thread row's `cwd` when the column exists.
- Normalize `.codex-global-state.json` workspace paths using the SQLite `threads.cwd` values when possible.

The first version does not implement provider switching, restore commands, status UI, or a standalone CLI.

## Locking and backups

Use lock directory:

```text
~/.codex/tmp/provider-sync.lock
```

If the lock already exists, skip sync and continue launch.

Before writing, create a backup under:

```text
~/.codex/backups_state/provider-sync/<timestamp>
```

The backup includes:

- `config.toml` if present.
- `.codex-global-state.json` and `.codex-global-state.json.bak` if present.
- `state_5.sqlite` and sidecar `-wal` / `-shm` files if present.
- A JSON manifest containing original first-line metadata for rollout files that will be changed.

After successful sync, prune managed provider-sync backups to keep the newest 5.

## Error handling

Skippable conditions continue launch and log a concise message:

- Sync lock already exists.
- SQLite database is busy before writes begin.
- Rollout files are locked or cannot be opened for rewrite.

Fatal conditions stop launch:

- Backup creation fails after sync has decided it needs to write.
- A write starts and then fails, and rollback/restore also fails.
- The Codex home exists but state files are malformed in a way that prevents safe updates.

## Testing

Automated tests should cover:

- Backend settings default to `providerSyncEnabled = false`.
- Backend settings are saved and reloaded from `~/.codex-session-delete/settings.json`.
- Bridge endpoints read and write `providerSyncEnabled`.
- Launcher calls provider sync before launching Codex only when the setting is enabled.
- Lock-existing sync result is skipped and does not block launch.
- Rollout first-line metadata is updated to the current config provider.
- SQLite `threads.model_provider` is updated to the current config provider.
- Backups are created before writes and pruned to 5 managed backups.

Manual verification should cover:

1. Turn on `Provider 同步` in the Codex++ menu.
2. Close Codex.
3. Change `model_provider` in `~/.codex/config.toml`.
4. Launch Codex++.
5. Confirm older conversations remain visible in Codex Desktop and `/resume`.
6. Confirm lock-existing behavior still launches Codex and writes a launcher log entry.
