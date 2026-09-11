# Cloud Release Candidate Verification - 2026-09-11

## Release identity

- Branch: `codex/migrate-official-1.3.0`
- Official baseline: CodexPlusPlus `1.3.0` at
  `be6a45852f9992a688f33be933f444ee098fc67a`
- Cloud service: `1.0.0`
- Remote Bridge: `1.0.0`
- HarmonyOS app: `1.0.0` (`versionCode` `1000010`)
- Remote protocol: `2.0` only
- Cloud SQLite schema: `8` only

## In-place upgrade scope

The release keeps the existing production identity and operations surface:

- `workagents-remote-cloud.service`
- `/opt/workagents-remote-dev/workagents-remote-cloud`
- `/etc/workagents-remote/dev.env`
- `/var/lib/workagents-remote-dev/remote.sqlite3`
- `/usr/local/sbin/workagents-remote-deploy`
- `WORKAGENTS_REMOTE_*`
- `/workagents-remote`

No second service, database root, environment-variable family, nginx route or
deployment command was introduced.

## Code and storage changes

- The cloud binary exposes a machine-readable `--version` result and health
  metadata for service `1.0.0`, protocol `2.0`, and schema `8`.
- Protocol `1.x` requests and Gateway hello messages are rejected.
- Schema `8` adds `service_metadata` and records the current protocol/storage
  generation. Schema `7` and unversioned pre-existing tables are rejected.
- The release script does not convert schema `7` rows. It stops the service,
  moves the SQLite main/WAL/SHM files into the root-only release backup, and
  lets the new binary initialize a fresh schema `8` database.
- Manual rollback is transactional: if restoring the selected backup fails at
  any stage, the script restores the generation that was running before the
  rollback attempt. A double failure leaves the service stopped and preserves
  the recovery snapshot for explicit operator action.

## Verification results

- Cloud Rust tests: 45 passed; one explicit loopback integration-server test ignored.
- Cloud strict Clippy, format, debug build and Windows release build: passed.
- Cloud loopback smoke: health metadata passed; protocol `1.5` and unauthenticated
  requests were rejected.
- `xuan-bridge`: 19 tests, strict Clippy, format and release build passed.
- Remote Bridge: 36 tests passed; one explicit read-only live-data test ignored;
  strict Clippy, format, release build, `--version` and contract checks passed.
- Plugin manifests, MCP adapters and both User Script DOM suites: 10 tests passed.
- Remote protocol: 39 message definitions and 41 fixtures passed.
- Deployment contract and Bash syntax checks: passed.
- Upgrade/rollback rehearsal: old binary, SQLite main/WAL/SHM and four configuration
  files restored; failed generation preserved; no parallel deployment created.
- HarmonyOS API 20 build: passed. The current unsigned HAP contains bundle
  `com.dyys.workagents.remote.dev`, version `1.0.0`, and versionCode `1000010`.

## Existing release procedure

Build the Linux x86-64 ELF on the existing release host, run its tests and
version gate, install the updated same-name controlled deploy script, and use:

```bash
sudo -n /usr/local/sbin/workagents-remote-deploy \
  "$RELEASE_ID" "$SHA256" 8 --reset-database-from 7
```

Rollback uses the same entry:

```bash
sudo -n /usr/local/sbin/workagents-remote-deploy \
  --rollback-release "$RELEASE_ID"
```

## Remaining release gates

Production was not changed during this verification. The SSH host, port and key
are intentionally not stored in the repository and were not available in this
session. The existing release host must still produce and validate the Linux
x86-64 ELF before the in-place production command is allowed.

The HAP verified here is unsigned. A signed build and redacted real-device
register, pair, reconnect, task sync, send, stop and final-reply acceptance run
remain required for the coordinated Remote release.
