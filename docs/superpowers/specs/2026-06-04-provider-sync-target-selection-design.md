# Provider Sync Target Selection Design

## Goal

Add a user-selectable sync target to the Codex++ session sync page, and align Codex++ provider discovery and sync behavior with `Dailin521/codex-provider-sync`.

The feature lets users choose which Provider ID historical sessions should be synchronized to, while keeping the current Codex `config.toml` provider unchanged unless a future separate switch feature is added.

## Reference behavior

The reference project fills provider choices in `desktop/CodexProviderSync.Core/ProviderDiscoveryService.cs` through `BuildProviderOptions(StatusSnapshot status, AppSettings settings)`. It merges provider IDs from:

- Configured providers in `config.toml`.
- Provider IDs already present in session rollout files.
- Provider IDs already present in archived session rollout files.
- Provider IDs already present in `state_5.sqlite`.
- Saved and manually-added providers from app settings.
- The current root `model_provider`.

It sorts the current provider first, then the remaining IDs by provider ID.

The reference sync behavior in `desktop/CodexProviderSync.Core/CodexSyncService.cs` accepts an optional explicit provider. If present, sync targets that provider; otherwise it uses the current root `model_provider`, falling back to the default provider.

## User experience

The existing session sync / historical session repair area gains a target selector above or beside the sync button:

```text
同步目标
[ apigather（配置 / 当前） ▼ ]

[立即同步会话]
```

The selector is a dropdown, not a freeform input in the first UI version. Each option shows the provider ID and source badges in Chinese:

```text
apigather     配置 / 当前
custom        配置 / 会话
openai        配置 / 会话 / 索引
old-provider  会话
```

Source labels map as follows:

| Internal source | UI label |
| --- | --- |
| `config` | 配置 |
| `rollout` | 会话 |
| `sqlite` | 索引 |
| `manual` | 手动 |

If the target list loads successfully, the selected target defaults to:

1. Previously selected provider if it is still present.
2. Current root `model_provider` if present.
3. First provider option.
4. `openai` only if no provider can be discovered.

The progress message includes the chosen target:

```text
正在同步到 openai…
正在扫描历史会话与索引…
正在写入修复与备份…
已同步到 openai：修复 3 个会话文件，更新 8 行索引。
```

## Architecture

Keep provider sync inside `crates/codex-plus-data/src/provider_sync.rs`; do not depend on the external Node.js or .NET project at runtime.

Add three focused backend capabilities:

1. Provider target discovery, matching the reference project's source merge rules.
2. Explicit-target provider sync, matching `codex-provider sync --provider <id>` semantics.
3. More complete sync repair logic for global workspace state, matching the reference project where Codex++ currently differs.

The Tauri manager exposes two commands:

- `load_provider_sync_targets()` returns the dropdown choices.
- `sync_providers_now(target_provider: Option<String>)` syncs to the chosen target.

The launcher keeps calling the existing no-argument provider sync behavior so automatic startup sync remains unchanged.

## Backend data model

Add serializable provider target types in `crates/codex-plus-data/src/provider_sync.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSyncTargetSource {
    Config,
    Rollout,
    Sqlite,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSyncTargetOption {
    pub id: String,
    pub sources: Vec<ProviderSyncTargetSource>,
    pub is_current_provider: bool,
    pub is_manual: bool,
    pub is_saved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSyncTargetList {
    pub current_provider: String,
    pub targets: Vec<ProviderSyncTargetOption>,
}
```

Use serde rename behavior at the Tauri boundary so frontend payload keys remain camelCase where existing commands already expect camelCase.

## Provider discovery

Add a public function:

```rust
pub fn load_provider_sync_targets(codex_home: Option<&Path>) -> ProviderSyncTargetList
```

It builds a map from provider ID to sources, mirroring the reference project:

1. Add all provider IDs declared by `[model_providers.<id>]` in `config.toml` with source `Config`.
2. Add the current root `model_provider` with source `Config`.
3. Add rollout provider IDs from `sessions/**/rollout-*.jsonl` and `archived_sessions/**/rollout-*.jsonl` with source `Rollout`.
4. Add SQLite provider IDs from `state_5.sqlite` `threads.model_provider`, split by archived/non-archived only for counting; source is still `Sqlite`.
5. Add saved/manual providers from Codex++ settings with source `Manual` once those settings fields exist.
6. Add the default provider `openai` through the config-provider parser behavior, matching the reference `listConfiguredProviderIds()` default inclusion.

Provider IDs that are missing, empty, or whitespace-only are ignored. Missing `model_provider` falls back to `openai`.

Sort options by:

1. `id == current_provider` descending.
2. `id` ascending with ordinal comparison.

## Settings for saved and manual providers

Extend manager settings with fields equivalent to the reference app state:

```ts
providerSyncSavedProviders: string[];
providerSyncManualProviders: string[];
providerSyncLastSelectedProvider: string;
```

The first UI version does not need a manual-add control. It should still persist `providerSyncLastSelectedProvider` after the user selects a dropdown option. When sync succeeds, the selected provider should be recorded into `providerSyncSavedProviders` if absent.

Manual providers are reserved for a future UI that explicitly adds/removes items, matching the reference project without making the first UI heavier.

## Explicit-target sync

Add a public function:

```rust
pub fn run_provider_sync_with_target(
    codex_home: Option<&Path>,
    target_provider: Option<&str>,
) -> ProviderSyncResult
```

Preserve the existing function as a wrapper:

```rust
pub fn run_provider_sync(codex_home: Option<&Path>) -> ProviderSyncResult {
    run_provider_sync_with_target(codex_home, None)
}
```

Target resolution matches the reference project:

1. If `target_provider` is present and non-empty after trim, use it.
2. Otherwise use root `model_provider` from `config.toml`.
3. Otherwise use default `openai`.

Validate explicit targets before writing. Reject values containing control characters, newlines, or path separators. Valid provider IDs should accept the same practical shape as configured provider IDs: letters, digits, underscore, dash, and dot.

This command is equivalent to the reference project's `codex-provider sync --provider <id>`, not `codex-provider switch <id>`.

## Rollout sync behavior

Keep scanning:

```text
~/.codex/sessions/**/rollout-*.jsonl
~/.codex/archived_sessions/**/rollout-*.jsonl
```

For each rollout file, read the first line only for `session_meta` metadata. If the first line is not valid JSON, is not `type == "session_meta"`, or lacks an object `payload`, skip it.

When the current `payload.model_provider` differs from the target provider, rewrite only that field in the first-line metadata. Do not change the rest of the JSONL conversation content.

If a rollout contains `encrypted_content`, count it by its original provider and return the existing warning behavior. Sync must not decrypt, re-encrypt, or rewrite encrypted conversation payloads.

Continue to preserve file modification time after rewriting, matching current Codex++ behavior and the reference project.

## SQLite sync behavior

Align SQLite updates with the reference `SqliteStateService.UpdateSqliteProviderAsync()` behavior.

When `state_5.sqlite` is absent, sync continues with zero SQLite updates.

When present, update the `threads` table in a transaction:

```sql
UPDATE threads
SET model_provider = $provider
WHERE COALESCE(model_provider, '') <> $provider
```

If the `has_user_event` column exists, set it for threads whose rollout file contains user events:

```sql
UPDATE threads
SET has_user_event = 1
WHERE id = $id AND COALESCE(has_user_event, 0) <> 1
```

If the `cwd` column exists, update it from rollout metadata by thread ID:

```sql
UPDATE threads
SET cwd = $cwd
WHERE id = $id AND COALESCE(cwd, '') <> $cwd
```

Malformed or busy SQLite errors should return a skipped/failed sync result with a clear message and no partial writes reported as success.

## Global state sync behavior

Align Codex++ global state repair with the reference `GlobalStateService.SyncWorkspaceRootsAsync()`.

Read `.codex-global-state.json` if present. If absent, return zero global state updates.

Use SQLite `threads.cwd` stats when possible to resolve workspace root paths. The effective path normalization should repair Desktop-visible workspace roots, not only dedupe existing strings.

Handle these fields:

```text
electron-saved-workspace-roots
project-order
active-workspace-roots
electron-workspace-root-labels
open-in-target-preferences.perPath
```

Codex++ currently handles the first four groups partially; this design adds `open-in-target-preferences.perPath` and uses SQLite cwd stats to resolve stored paths in the same spirit as the reference project.

When global state changes, write both:

```text
.codex-global-state.json
.codex-global-state.json.bak
```

## Backup format

Keep the backup root:

```text
~/.codex/backups_state/provider-sync/<timestamp>
```

Back up the same artifacts as the reference project:

```text
config.toml
.codex-global-state.json
.codex-global-state.json.bak
db/state_5.sqlite
db/state_5.sqlite-wal
db/state_5.sqlite-shm
session-meta-backup.json
metadata.json
```

`session-meta-backup.json` records original first-line metadata for rollout files that will be changed, including path, original first line, separator, and original last-write timestamp when available.

Expand `metadata.json` to include structured fields while preserving Codex++'s existing `managedBy` marker for pruning:

```json
{
  "version": 1,
  "namespace": "provider-sync",
  "codexHome": "...",
  "targetProvider": "...",
  "createdAt": "...",
  "dbFiles": ["state_5.sqlite"],
  "changedSessionFiles": 3,
  "managedBy": "Codex++ provider sync"
}
```

After successful sync, keep pruning managed backups to the newest 5.

## Failure recovery

Improve Codex++ recovery semantics to match the reference project more closely.

Before writes, create a backup. Then:

1. Open SQLite transaction if the database exists.
2. Apply SQLite provider/user-event/cwd updates inside the transaction.
3. Rewrite rollout files and record the actually applied files.
4. Sync global state.
5. Commit SQLite transaction only after rollout and global state writes succeed.
6. On failure before commit, rollback SQLite.
7. Restore any rollout files already rewritten.
8. Restore global state files from backup if they were changed before failure.

If restore itself fails, return a clear skipped/failed result explaining both the original write error and restore error.

## Tauri command changes

Add command:

```rust
#[tauri::command]
pub async fn load_provider_sync_targets() -> CommandResult<Value>
```

Return payload shape:

```json
{
  "currentProvider": "apigather",
  "targets": [
    {
      "id": "apigather",
      "sources": ["config"],
      "isCurrentProvider": true,
      "isManual": false,
      "isSaved": true
    }
  ]
}
```

Modify existing command:

```rust
#[tauri::command]
pub async fn sync_providers_now(target_provider: Option<String>) -> CommandResult<Value>
```

The frontend calls it as:

```ts
call("sync_providers_now", {
  targetProvider: selectedProviderSyncTarget,
});
```

The returned payload continues to include `targetProvider`, update counts, skipped locked files, backup directory, and encrypted content warning.

## Frontend changes

In `apps/codex-plus-manager/src/App.tsx`:

- Add types for provider sync target list and target options.
- Load targets when the app initializes and when the sessions page or sync area refreshes.
- Add state for selected sync target.
- Render a dropdown in the existing sync area.
- Persist the last selected provider in backend settings.
- Pass the selected target into `sync_providers_now`.
- Include the target in progress messages and completion notices.

If target loading fails, the dropdown shows a disabled fallback option and sync uses current backend behavior only when no explicit target is selected.

## Compatibility

Launcher behavior remains compatible:

- `apps/codex-plus-launcher/src/main.rs` continues calling `codex_plus_data::run_provider_sync(None)`.
- Startup sync still targets current root `model_provider`.
- No launcher UI selection is added.

Existing manager manual sync remains compatible:

- If `sync_providers_now` is called without `targetProvider`, it behaves exactly as before.

## Non-goals

This feature does not implement provider switching. It does not rewrite `config.toml` root `model_provider` when the user chooses a dropdown target.

This feature does not add a restore UI, standalone CLI, or manual provider add/remove UI in the first version.

This feature does not inspect or modify `.tmp/cc-switch-src`.

## Testing

Automated tests should cover:

- Provider discovery includes default `openai`.
- Provider discovery includes current root `model_provider`.
- Provider discovery includes `[model_providers.<id>]` entries.
- Provider discovery includes provider IDs from `sessions` rollout metadata.
- Provider discovery includes provider IDs from `archived_sessions` rollout metadata.
- Provider discovery includes provider IDs from SQLite `threads.model_provider`.
- Provider discovery merges sources for duplicate provider IDs.
- Provider discovery sorts current provider first, then by provider ID.
- Explicit target overrides current config provider during sync.
- Missing explicit target preserves existing current-config behavior.
- Invalid explicit target is rejected before writes.
- Sync updates rollout first-line `payload.model_provider` to the selected target.
- Sync updates SQLite `threads.model_provider` to the selected target.
- Sync updates `has_user_event` and `cwd` consistently with existing behavior.
- Global state sync handles `open-in-target-preferences.perPath`.
- Backup `metadata.json` includes structured fields and preserves `managedBy`.
- Failure during rollout/global state write rolls back SQLite and restores applied files.
- Tauri `sync_providers_now` accepts optional `targetProvider` and returns the selected `targetProvider`.

Manual verification should cover:

1. Open Codex++ manager.
2. Confirm the sync target dropdown lists providers from config, rollout files, and SQLite.
3. Pick a non-current provider.
4. Run sync.
5. Confirm result message says the selected target.
6. Confirm historical sessions become visible under the selected provider.
7. Confirm `config.toml` root `model_provider` was not changed.
8. Confirm startup sync still uses current config provider when enabled.
