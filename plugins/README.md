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
```

Set `XUAN_BRIDGE_BIN` when the bridge executable is not available on `PATH`.
The bridge uses a versioned JSON-lines protocol so the same contract can later
be exposed through loopback HTTP or a named pipe without changing the plugins.

The mobile feature is intentionally not packaged here. It remains the
independent `xuan-plus-remote` project, with its own bridge adapter, HarmonyOS
application and protocol/cloud-service compatibility matrix.
