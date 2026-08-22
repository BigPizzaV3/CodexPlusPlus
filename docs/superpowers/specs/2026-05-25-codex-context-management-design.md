# Codex Context Management Design

## Goal

Add Codex-only management for MCP servers, skills, and plugins, using the same switching concept as CCSwitch while staying inside Codex++'s current settings and relay profile model.

This feature manages shared Codex context entries from the existing `relayCommonConfigContents` TOML snippet and lets each supplier profile choose which entries are active when that supplier is switched on.

## Scope

In scope:

- Manage Codex `config.toml` context sections:
  - `[mcp_servers.*]`
  - `[skills.*]`
  - `[plugins.*]`
- Provide UI to add, edit, delete, and enable context entries per supplier.
- Store all reusable context entries in `relayCommonConfigContents`.
- Store each supplier's selected context ids on that supplier.
- Add supplier-level inputs for:
  - context window size, written as `model_context_window`
  - auto compact limit, written as `model_auto_compact_token_limit`
- Apply only the selected context entries when switching a supplier.
- Preserve existing raw TOML editing for advanced users.

Out of scope:

- Cross-application management for Claude, Gemini, OpenCode, Hermes, or Claude Desktop.
- CCSwitch database migration or database-backed MCP/skill/plugin registries.
- Installing skills/plugins from remote repositories.
- Runtime validation that Codex actually honors context-size settings in every Codex version.

## Data Model

`BackendSettings` keeps the shared TOML in `relayCommonConfigContents`.

`RelayProfile` gains supplier-level context configuration:

```ts
contextSelection: {
  mcpServers: string[]
  skills: string[]
  plugins: string[]
}
contextWindow: string
autoCompactLimit: string
```

Rust should mirror these fields with serde camelCase defaults so existing settings files continue to load.

Empty `contextWindow` and `autoCompactLimit` mean no value is written. Non-empty values must be positive integers before being written.

## TOML Handling

Core code should parse TOML structurally with `toml_edit`.

Needed operations:

- Parse `relayCommonConfigContents` into typed context entries grouped by kind.
- Upsert one context entry back into the common snippet.
- Delete one context entry from the common snippet.
- Build a filtered common snippet for a supplier from `contextSelection`.
- Apply supplier context-size fields to the supplier config before writing live config.

Filtering rules:

- If a supplier has an explicit `contextSelection`, only selected ids are applied.
- If a supplier was created before this feature and has no selection field, default to selecting all current common entries to preserve previous behavior.
- Deleted common entries should be silently ignored by supplier selections until the supplier is edited or saved.

Merge rules:

- Selected common entries are merged into supplier config during switch.
- Non-selected common entries are not written to live config by the switch path.
- Supplier-specific provider configuration remains supplier-owned.
- `model_context_window` and `model_auto_compact_token_limit` are supplier-owned top-level keys.

## UI

Add a context management section to the supplier configuration page.

It should be compact and operational, matching the current settings UI:

- A tab or segmented control for MCP, Skills, and Plugins.
- Each row shows id, enabled-for-this-supplier checkbox, and a short summary.
- Row actions: edit and delete.
- Add action per kind.
- Editing can use a small form for id plus raw TOML body. MCP may also expose command/args/env fields if this stays simple.
- Keep the existing raw `公共 config.toml` textarea as the advanced escape hatch.
- Context window and auto compact limit inputs live beside supplier model/base URL settings because they are supplier-level behavior.

When adding a new supplier:

- The new supplier starts with all existing common context entries selected.
- The user can uncheck entries before saving or switching.
- Context-size inputs start empty.

## Backend Commands

Add Tauri commands that operate on settings rather than directly mutating live Codex files:

- list context entries from a provided settings object or current saved settings
- upsert context entry
- delete context entry
- update supplier context selection

The existing save flow remains the source of persistence.

Switch commands should call a new core function that receives:

- supplier config TOML
- auth JSON
- full common config TOML
- supplier context selection
- supplier context-size fields

The core function produces the effective live `config.toml` and writes it atomically through the existing backup path.

## Error Handling

- Invalid common TOML should surface a clear error and avoid writing live config.
- Invalid context entry TOML should block saving that entry.
- Invalid context-size values should block supplier save or switch with a clear message.
- Deleting a common entry removes it from `relayCommonConfigContents`; stale profile selections are harmless.
- If raw common TOML is edited manually and contains new entries, supplier UI should discover them on refresh.

## Tests

Core tests:

- Parse MCP, skills, and plugins from common TOML.
- Upsert and delete context entries without damaging unrelated TOML.
- Filter common config by supplier selection.
- Apply selected context entries only during full file switch.
- Write `model_context_window` and `model_auto_compact_token_limit` only when non-empty.
- Reject invalid context-size values.

Manager tests:

- Settings serde defaults preserve existing settings.
- Save normalization preserves common entries and supplier selections.
- Switch path passes selection and context-size values into core.

Frontend checks:

- TypeScript checks for new fields.
- New supplier defaults select all common entries.
- Existing suppliers without selection behave as all selected.

## Open Decisions

None. This design intentionally keeps the first implementation Codex-only and TOML-backed, with raw TOML still available for advanced cases.
