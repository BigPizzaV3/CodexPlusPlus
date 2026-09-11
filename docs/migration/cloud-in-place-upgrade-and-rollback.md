# Cloud In-Place Upgrade And Rollback

## Scope

The cloud is upgraded as the next version of the current service. There is one
systemd unit, one nginx route, one environment file, one database location, and
one controlled deploy command. No legacy instance, v2 sidecar, dual-write path,
compatibility endpoint, or parallel database is created.

Target release:

- Remote suite: 1.0.0
- Protocol: 2.0 only
- SQLite: schema 8 only
- Previous production generation: protocol 1.5, SQLite schema 7

## Pre-deploy gates

1. Build and test cloud, Remote Bridge, xuan-bridge, protocol fixtures, User
   Script smoke tests, and HarmonyOS sources.
2. Build the Linux x86-64 workagents-remote-cloud artifact with the existing
   release toolchain.
3. Run the artifact with --version; it must report 1.0.0, 2.0, and schema 8.
4. Confirm the existing service is healthy and reports schema 7.
5. Install the updated controlled deploy script through the existing
   install-workagents-remote-deploy entry.
6. Upload only the new binary into the existing incoming release directory.

Do not inspect, print, or copy production secrets into the repository.

## Upgrade command

    sudo /usr/local/sbin/workagents-remote-deploy "$RELEASE_ID" "$SHA256" 8 --reset-database-from 7

The command creates a root-only backup containing:

- previous workagents-remote-cloud
- remote.sqlite3, remote.sqlite3-wal, and remote.sqlite3-shm when present
- /etc/workagents-remote/dev.env
- the current systemd unit
- the two current nginx snippets
- non-sensitive release metadata and the previous binary hash

The service is then stopped, schema 7 files are moved into the backup, the
binary is replaced, and a fresh schema 8 database is initialized. A failed
start or failed target-version health check automatically restores the previous
binary and schema 7 files.

## Post-deploy gates

The local and public health endpoints must both report ok=true,
environment=dev, service_version=1.0.0, contract_version=2.0, and
storage_schema_version=8.

Then verify:

1. unauthenticated Gateway and app requests still fail closed;
2. a protocol 1.5 request is rejected;
3. a protocol 2.0 device can register and pair;
4. the Remote Bridge can connect, synchronize a task, send input, stop a turn,
   and receive the final reply;
5. Huawei Push dispatch remains free of task content and credentials.

## Rollback

    sudo /usr/local/sbin/workagents-remote-deploy --rollback-release "$RELEASE_ID"

Rollback is a whole-release restore. It restores the previous binary, schema 7
database, environment file, unit, and nginx snippets from the pre-deploy
snapshot. It does not translate schema 8 rows into schema 7.

After rollback, verify the old local and public health endpoints and one
authenticated old-client connection. Keep the failed schema 8 snapshot for
diagnosis; do not point the old binary at it.

## Operational risk

The intentional schema reset removes active registrations, bindings, task read
models, commands, and Push outbox rows from the new active database. Users must
register and pair again after the 2.0 release. Rollback restores the complete
pre-deploy schema 7 snapshot, including its prior registrations and bindings.
