# xuan-bridge

`xuan-bridge` is the local process boundary for Xuan plugins. It exposes the
same contract through JSON-lines (for MCP child processes) and loopback HTTP:

- `xuan-bridge` reads JSON-RPC-like requests from stdin and writes one response per line.
- `xuan-bridge --http 127.0.0.1:57324` serves `/v1/health`, `/v1/search/*`, `/v1/usage`, `/v1/polish` and `/v1/mobile/*`.
- `xuan-bridge init [output-root]` creates `xuan-plugins.json` and the versioned `xuan-bridge.sqlite` schema.
- `xuan-bridge migrate <legacy-settings.json> [output-root]` writes namespaced config and read-only backups.

The bridge implements bounded streaming workspace search, generic and OwlAI
usage adapters, and Chat Completions, Responses and Anthropic polish adapters.
Profiles are selected from `xuan-plugins.json`; credentials are referenced by
environment-variable name and are never stored in that file. Mobile requests
forward to `xuan-plus-remote` through `XUAN_MOBILE_BRIDGE_URL`.

Example profile configuration:

```json
{
  "schemaVersion": 1,
  "plugins": {
    "xuan-usage": {
      "defaultProfile": "relay",
      "profiles": {
        "relay": {
          "provider": "generic",
          "baseUrl": "https://relay.example/v1",
          "apiKeyEnv": "XUAN_USAGE_API_KEY"
        }
      }
    },
    "xuan-polish": {
      "defaultProfile": "polish",
      "profiles": {
        "polish": {
          "protocol": "responses",
          "baseUrl": "https://relay.example/v1",
          "model": "polish-model",
          "apiKeyEnv": "XUAN_POLISH_API_KEY"
        }
      }
    }
  }
}
```

The HTTP listener rejects non-loopback addresses. Browser calls use a narrow
origin allowlist and may require `XUAN_BRIDGE_HTTP_TOKEN`; the Composer script
reads that token only from the runtime `window.__XUAN_BRIDGE_TOKEN__` hook.

Important environment variables:

```text
XUAN_HOME
XUAN_WORKSPACE_ROOTS
XUAN_USAGE_BASE_URL
XUAN_USAGE_API_KEY
XUAN_USAGE_PROVIDER
XUAN_POLISH_BASE_URL
XUAN_POLISH_API_KEY
XUAN_POLISH_MODEL
XUAN_POLISH_PROTOCOL
XUAN_BRIDGE_HTTP_TOKEN
XUAN_BRIDGE_ALLOWED_ORIGINS
XUAN_MOBILE_BRIDGE_URL
XUAN_MOBILE_BRIDGE_TOKEN
```
