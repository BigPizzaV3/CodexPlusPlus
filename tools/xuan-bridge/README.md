# xuan-bridge

`xuan-bridge` is the local process boundary for Xuan plugins. It exposes the
same contract through JSON-lines (for MCP child processes) and loopback HTTP:

- `xuan-bridge` reads JSON-RPC-like requests from stdin and writes one response per line.
- `xuan-bridge --http 127.0.0.1:57324` serves `/v1/health`, `/v1/search/*`, `/v1/usage`, `/v1/polish` and `/v1/mobile/*`.
- `xuan-bridge migrate <legacy-settings.json> [output-root]` writes namespaced config and read-only backups.

The current bridge fully implements bounded workspace search, generic usage
requests and OpenAI-compatible polish requests. Provider/profile credential
resolution is intentionally environment-based until a generic host adapter is
added. Mobile requests forward to `xuan-plus-remote` through
`XUAN_MOBILE_BRIDGE_URL`.

Important environment variables:

```text
XUAN_HOME
XUAN_WORKSPACE_ROOTS
XUAN_USAGE_BASE_URL
XUAN_USAGE_API_KEY
XUAN_POLISH_BASE_URL
XUAN_POLISH_API_KEY
XUAN_POLISH_MODEL
XUAN_MOBILE_BRIDGE_URL
XUAN_MOBILE_BRIDGE_TOKEN
```
