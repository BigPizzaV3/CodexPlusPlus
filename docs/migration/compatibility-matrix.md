# Compatibility Matrix

| Component | Version | Compatible with | Upgrade trigger |
| --- | --- | --- | --- |
| Official CodexPlusPlus | `1.3.0` | Xuan plugins `0.1.x`, bridge `0.1.x` | Validate plugin manifest and renderer smoke tests |
| `xuan-workspace-search` | `0.1.0` | bridge protocol `1`; MCP `2024-11-05` | MCP schema or result-shape change |
| `xuan-usage` | `0.1.0` | bridge protocol `1`; config schema `1` | Provider adapter or credential contract change |
| `xuan-polish` | `0.1.0` | bridge protocol `1`; config schema `1`; user-script ABI `0` | Composer DOM/bridge URL change |
| `xuan-bridge` | `0.1.0` | plugin protocol `1`; SQLite schema `1` | JSON API or persistent-state migration |
| `xuan-plus-remote` | source protocol `1.5` | remote fixtures `1.0` through `1.5` | Pairing, command or task snapshot contract change |

The official host and each Xuan component are released independently. An
official upgrade must pass the matrix before the compatibility range is
expanded. User scripts are the highest-risk surface and are tested against a
small DOM fixture on every official release candidate.
