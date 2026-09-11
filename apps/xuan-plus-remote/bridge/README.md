# Desktop Bridge Adapter

The desktop bridge adapter is an independent Cargo crate and is intentionally
separate from the official CodexPlusPlus workspace. `xuan-bridge` forwards the
versioned `mobile.*` methods to it through `XUAN_MOBILE_BRIDGE_URL` and an
optional `XUAN_MOBILE_BRIDGE_TOKEN`.

The adapter owns pairing, DPAPI/HUKS-backed identity, protocol 2.0, task
synchronization and cloud transport. The Codex plugin never receives device
keys, cloud credentials or raw task history.

The runtime preserves the legacy `mobile_identity` and
`mobile_command_receipts` SQLite tables, including Windows DPAPI protection.
It reads official Codex task indexes and rollout files without writing them.

## Build and run

```text
cargo build --manifest-path apps/xuan-plus-remote/bridge/Cargo.toml --locked
xuan-plus-remote-bridge install-user-script
xuan-plus-remote-bridge
```

The service defaults to `127.0.0.1:17421` and rejects non-loopback bind
addresses. Configure both local processes with the same optional token:

```text
XUAN_MOBILE_BRIDGE_URL=http://127.0.0.1:17421
XUAN_MOBILE_BRIDGE_TOKEN=<local secret>
```

Use `XUAN_HOME` to make the bridge read the migrated
`xuan-plus-remote/mobile-remote.sqlite`. `XUAN_CODEX_DEBUG_PORT` explicitly
selects an existing official Codex CDP endpoint; otherwise the bridge reads the
configured launcher's `latest-status.json` when no explicit debug port is supplied.

Remote create-task model choices come from `mobile.models` in
`xuan-plugins.json`:

```json
{
  "mobile": {
    "models": [
      { "model": "gpt-5", "provider": "official" }
    ]
  }
}
```

Expected endpoints are `GET /v1/mobile/status` and `POST` endpoints for
`pair`, `confirm`, `tasks`, `send-input` and `stop`. Keep these endpoints
aligned with the protocol fixtures under `../protocol/` and the current-only
contract in `mobile-bridge-contract.json`.
