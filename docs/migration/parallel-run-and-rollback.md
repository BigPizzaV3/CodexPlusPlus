# Parallel Run And Rollback

## Preconditions

1. Keep the legacy worktree and installation unchanged.
2. Build official `v1.3.0`, `xuan-bridge`, and the independent Remote Bridge from their recorded
   commits. Do not package uncommitted legacy files into the new installation.
3. Choose a new Xuan data root. The examples below call it `<xuan-home>`.
4. Stop remote-control writes briefly while taking the migration snapshot. The migration uses the
   SQLite online-backup API and supports WAL, but a quiet cutover is easier to audit.

## One-time migration

```text
xuan-bridge migrate <legacy-state-dir>/settings.json <xuan-home>
xuan-plus-remote-bridge install-user-script
```

The command creates:

- `<xuan-home>/xuan-plugins.json`
- `<xuan-home>/xuan-bridge.sqlite`
- `<xuan-home>/legacy-settings-backup-*.json` as read-only
- `<xuan-home>/legacy-state/*` as read-only
- `<xuan-home>/xuan-plus-remote/mobile-remote.sqlite` as the writable Remote DB

API keys are not copied. Configure the named credential environment variables after reviewing
`credentialMigrationRequired` in `xuan-plugins.json`.

## New-stack environment

```text
XUAN_HOME=<xuan-home>
XUAN_MOBILE_BRIDGE_URL=http://127.0.0.1:17421
XUAN_MOBILE_BRIDGE_TOKEN=<local bearer token>
XUAN_CODEX_DEBUG_PORT=<official Codex debug port>
```

Start the Remote Bridge, start `xuan-bridge`, enable the three marketplace plugins, reload User
Scripts, and then start the official host. Validate health, search, usage, polish, task listing,
pairing status, one send-input operation and one stop operation.

## Parallel-run boundary

The legacy and new installations may remain installed and may use separate Xuan configuration
directories. Do not run both phone gateway processes against the same copied device identity at
the same time. Only one gateway may own the cloud connection; disable phone connection in the
inactive stack before enabling it in the other.

For a low-risk soak, use a disposable `CODEX_HOME` first. When validating reuse of official token
history and task indexes, keep one Codex desktop writer active at a time.

## Rollback

1. Stop `xuan-bridge` and `xuan-plus-remote-bridge`.
2. Disable the three Xuan plugins and the Xuan User Scripts in the official host.
3. Start the unchanged legacy installation and re-enable its phone gateway only after the new
   gateway is fully stopped.
4. Do not copy the migrated writable DB back over the legacy DB. The original files were never
   modified; the read-only snapshots are audit evidence and emergency recovery inputs.

Rollback is complete when the legacy search, usage, polish and phone status checks pass. Preserve
the failed new-stack data root for diagnosis instead of editing the read-only backups.
