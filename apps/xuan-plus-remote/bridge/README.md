# Desktop Bridge Adapter

The desktop bridge adapter is intentionally separate from the official
CodexPlusPlus workspace. `xuan-bridge` forwards the versioned `mobile.*`
methods to this component through `XUAN_MOBILE_BRIDGE_URL` and an optional
`XUAN_MOBILE_BRIDGE_TOKEN`.

The adapter owns pairing, DPAPI/HUKS-backed identity, protocol 1.5, task
synchronization and cloud transport. The Codex plugin never receives device
keys, cloud credentials or raw task history.

Expected endpoints are `GET /v1/mobile/status` and `POST` endpoints for
`pair`, `confirm`, `tasks`, `send-input` and `stop`. Keep these endpoints
backward compatible with the protocol fixtures under `../protocol/`.
