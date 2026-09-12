# Xuan Plugins

These packages target the official CodexPlusPlus `v1.3.0` plugin contract.
Feature logic belongs to `tools/xuan-bridge`; plugin packages only declare
skills/MCP tools and optional renderer scripts.

The repo marketplace is declared at `.agents/plugins/marketplace.json`. A local
installation uses the repository root as the marketplace source:

```text
codex plugin marketplace add <repository-root>
codex plugin add xuan-workspace-search@xuan-curated
codex plugin add xuan-usage@xuan-curated
codex plugin add xuan-polish@xuan-curated
codex plugin add xuan-mobile@xuan-curated
```

Set `XUAN_BRIDGE_BIN` when the bridge executable is not available on `PATH`.
The bridge uses a versioned JSON-lines protocol and loopback HTTP endpoints.
The mobile plugin is an independent package: its renderer User Script provides
the desktop pairing entry, QR code, local confirmation and task selection while
the `xuan-plus-remote` project supplies the HarmonyOS client and remote bridge.
