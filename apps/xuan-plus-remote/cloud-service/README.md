# XuanPlus Remote Cloud

This directory is the in-place upgrade source for the existing
workagents-remote-cloud.service. It does not define a second cloud service,
database root, nginx site, environment file, or release command.

## Release contract

| Item | Current value |
| --- | --- |
| Remote suite | 1.0.0 |
| Public protocol | 2.0 only |
| SQLite schema | 8 only |
| Binary | workagents-remote-cloud |
| systemd unit | workagents-remote-cloud.service |
| Environment file | /etc/workagents-remote/dev.env |
| Database | /var/lib/workagents-remote-dev/remote.sqlite3 |
| Public route | /workagents-remote |

Protocol 1.x and SQLite schema 7 are not read or converted. The release script
stops the existing service, moves the schema 7 database files into the release
backup, and lets version 1.0.0 initialize a fresh schema 8 database. A rollback
restores the backed-up binary, database, and configuration snapshot.

## Existing environment

The service continues to read only:

- WORKAGENTS_REMOTE_ENVIRONMENT
- WORKAGENTS_REMOTE_LISTEN
- WORKAGENTS_REMOTE_DATABASE
- WORKAGENTS_REMOTE_PUSH_TOKEN_KEY
- WORKAGENTS_REMOTE_HUAWEI_PUSH_SEND_URL
- WORKAGENTS_REMOTE_HUAWEI_PUSH_SERVICE_ACCOUNT_JSON_B64

Do not place real values in the repository or command output.

## Build and verification

    cargo test --manifest-path apps/xuan-plus-remote/cloud-service/Cargo.toml --locked
    cargo build --manifest-path apps/xuan-plus-remote/cloud-service/Cargo.toml --release --locked
    pwsh -File apps/xuan-plus-remote/cloud-service/verify-local.ps1 -ExecutablePath apps/xuan-plus-remote/cloud-service/target/release/workagents-remote-cloud.exe
    pwsh -File apps/xuan-plus-remote/cloud-service/deploy/verify-deploy-contract.ps1

The production artifact must be built for Linux x86-64 using the existing
release host/toolchain. The deploy command rejects a candidate unless
workagents-remote-cloud --version reports service 1.0.0, protocol 2.0, and
storage schema 8.

## In-place release

Keep the current upload location and invocation:

    RELEASE_ID="$(date -u +%Y%m%d%H%M%S)"
    SHA256="$(sha256sum workagents-remote-cloud | awk '{print $1}')"

    # Upload the binary to:
    # /var/lib/workagents-remote-deploy/incoming/$RELEASE_ID/workagents-remote-cloud

    sudo /usr/local/sbin/workagents-remote-deploy "$RELEASE_ID" "$SHA256" 8 --reset-database-from 7

Before stopping the service, the command verifies the current health endpoint,
nginx configuration, current database version, artifact hash, architecture,
dynamic libraries, and embedded release versions. It then writes a root-only
snapshot under /var/lib/workagents-remote-deploy/backups/$RELEASE_ID.

## Rollback

    sudo /usr/local/sbin/workagents-remote-deploy --rollback-release "$RELEASE_ID"

The same release command restores the previous binary, schema 7 database files,
environment file, systemd unit, and nginx snippets. It keeps the failed schema
8 runtime in a timestamped rollback-attempt directory for diagnosis.

After the schema reset, existing phone registrations and bindings no longer
exist in the active database. The HarmonyOS app and desktop bridge must both be
on protocol 2.0 and devices must register and pair again.
