# Compatibility Matrix

| Component | Version | Compatible with | Upgrade trigger |
| --- | --- | --- | --- |
| Official CodexPlusPlus | `1.3.0` | Xuan plugins `0.1.x`, `xuan-bridge` `0.1.x`, Remote Bridge `1.0.x` | Validate plugin manifest, User Scripts and renderer request smoke tests |
| `xuan-workspace-search` | `0.1.0` | bridge protocol `1`; MCP `2024-11-05` | MCP schema or result-shape change |
| `xuan-usage` | `0.1.0` | bridge protocol `1`; config schema `1` | Provider adapter or credential contract change |
| `xuan-polish` | `0.1.0` | bridge protocol `1`; config schema `1`; user-script ABI `1.0.0` | Composer DOM/bridge URL change |
| `xuan-bridge` | `0.1.0` | plugin protocol `1`; SQLite schema `1` | JSON API or persistent-state migration |
| Remote desktop bridge | `1.0.0` | mobile bridge contract `2.0`; User Script ABI `1.0.0`; remote protocol `2.0` | HTTP, CDP or identity-storage contract change |
| `xuan-plus-remote` app/cloud | `1.0.0` | protocol `2.0` only; cloud SQLite schema `8`; Remote Bridge `1.0.x` | Any public message, pairing, command, task snapshot or cloud storage change |

The official host, plugins and `xuan-bridge` are released independently. The
Remote Bridge, HarmonyOS app and cloud service are a coordinated Remote suite
release. Remote protocol 2.0 does not accept 1.x, and cloud schema 8 does not
read or convert schema 7. User scripts remain the highest-risk official-host
surface and are tested against a small DOM/renderer fixture on every official
release candidate.
