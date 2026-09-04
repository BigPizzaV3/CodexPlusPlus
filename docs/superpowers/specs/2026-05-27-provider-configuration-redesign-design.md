# Provider Configuration Redesign

## Summary

CodexPlusPlus will replace the current relay-profile-centered provider management with a cc-switch-style provider system. Providers become the primary persisted entity, the manager UI edits providers directly, and runtime configuration is generated from the active provider instead of reading `relayProfiles` as the source of truth.

The redesign aims for high compatibility with cc-switch’s provider model and import semantics without copying every peripheral feature. The focus is provider CRUD, migration from existing relay profiles, active-provider selection, runtime config generation, and external import compatibility.

## Goals

- Replace `relayProfiles` as the main provider data model.
- Introduce a standalone provider entity with cc-switch-compatible core fields.
- Let the manager app manage providers through a dedicated provider configuration screen.
- Generate Codex runtime/live configuration from the active provider.
- Migrate existing user data from `relayProfiles` automatically.
- Preserve and improve external provider import flows so they create providers directly.

## Non-goals

- Full one-to-one replication of all cc-switch provider-adjacent features.
- A full DB/schema clone of cc-switch internals.
- Supporting long-term dual editing of both providers and `relayProfiles`.
- Broad unrelated refactors outside provider, launch, import, and manager flows.

## Current State

Today provider-like data is stored in `BackendSettings.relayProfiles` and selected by `activeRelayId` in `crates/codex-plus-core/src/settings.rs`. Each relay profile mixes persisted provider details with runtime-oriented relay configuration fields such as base URL, API key, protocol, auth/config text, and context-related relay options.

The manager UI exposes settings and relay-related behavior through `apps/codex-plus-manager/src/App.tsx`, but there is no dedicated standalone provider resource layer. The backend already contains an external import path in `apps/codex-plus-manager/src-tauri/src/commands.rs` and `crates/codex-plus-core/src/ccs_import.rs`, but imports currently materialize as relay profiles.

This creates three problems:

1. Provider data is embedded inside a larger settings blob.
2. Runtime profile concerns and provider identity are coupled.
3. Interop with cc-switch-style provider resources requires ad hoc mapping.

## Proposed Architecture

### 1. Provider becomes the primary persisted resource

Add a standalone provider model in the Rust core/data layer. This provider model becomes the only authoritative persisted provider representation.

The provider model should align closely with cc-switch’s core fields:

- `id`
- `name`
- `settingsConfig`
- `websiteUrl` (optional)
- `notes` (optional)
- `meta` (optional structured metadata)

`settingsConfig` is the main payload and carries the provider-specific runtime configuration currently spread across relay-profile fields. The exact JSON/TOML shape should be chosen so current launch/runtime code can deterministically derive the generated Codex config.

### 2. BackendSettings only stores global state

`BackendSettings` remains for app-global behavior, but provider details move out of it. After the redesign it should only contain:

- `activeProviderId`
- non-provider global feature flags
- launch and wrapper settings that are genuinely global
- other manager-wide preferences unrelated to provider identity

`relayProfiles`, `activeRelayId`, and relay-profile-specific editing paths stop being the primary editable model. Temporary compatibility code may read them during migration only.

### 3. Runtime config is generated from the active provider

All flows that currently depend on relay profiles should switch to the following pipeline:

1. Read `activeProviderId` from settings.
2. Load the referenced provider from provider storage.
3. Derive the live Codex config, auth config, provider selection, and any runtime relay/provider artifacts from that provider.
4. Apply those generated artifacts to the current Codex home/config targets.

This keeps runtime behavior deterministic while decoupling editing/storage from generated live files.

## Provider Data Model

### Provider structure

The new provider entity should preserve high-value compatibility with cc-switch while staying scoped to CodexPlusPlus needs.

Recommended fields:

- `id: String`
- `name: String`
- `settings_config: serde_json::Value`
- `website_url: Option<String>`
- `notes: Option<String>`
- `meta: Option<ProviderMeta>`

`ProviderMeta` should start intentionally small and only cover fields that CodexPlusPlus will consume in this phase. It may include compatibility placeholders for future interop, but implementation should not depend on unused surface area.

Initial meta candidates:

- protocol / API format hints where needed
- optional import provenance
- optional compatibility markers for generated auth/config behavior
- optional UI-only metadata if needed later

Avoid introducing broad metadata like icon packs, partner catalogs, or unrelated marketplace annotations unless the manager UI actually consumes them in this redesign.

### settingsConfig mapping

The current relay-profile fields should be migrated into `settingsConfig` or provider metadata so provider data becomes self-contained.

At minimum, migration must preserve equivalent information for:

- upstream/base URL
- API key or auth payload references
- protocol or wire API mode
- provider-specific TOML config content
- provider-specific auth JSON content
- test-model-related information if still provider-specific after redesign

The storage format can differ from cc-switch internals, but field semantics should remain close enough that import/export mapping is straightforward and not lossy for supported fields.

## Storage Design

Introduce a dedicated provider store separate from `BackendSettings` persistence. The store should live in the Rust core/data layer and expose CRUD operations consumed by Tauri commands.

Requirements:

- list providers
- get provider by id
- create provider
- update provider
- delete provider
- set active provider indirectly via settings update
- load providers during runtime config generation

The provider store may use JSON or SQLite, but it should satisfy two constraints:

1. clear separation from settings storage
2. easy compatibility mapping for imported cc-switch-like provider records

Recommendation: use a dedicated JSON-backed or structured-file-backed store first unless an existing project persistence abstraction strongly favors SQLite. The key design concern is clean ownership and migration simplicity, not maximizing storage sophistication.

## Migration Plan

### Automatic one-time migration from relayProfiles

On first load after upgrade, detect whether provider storage is empty while legacy relay-profile data exists.

If true:

1. Read `BackendSettings.relayProfiles` and `activeRelayId`.
2. Convert each relay profile into a provider entity.
3. Persist the resulting providers in the new provider store.
4. Map `activeRelayId` to `activeProviderId`.
5. Save updated settings without relying on relay profiles as the source of truth.

### Migration behavior requirements

- Migration must be idempotent.
- Migration must not duplicate providers on repeated startup.
- Migration must preserve the active selection when possible.
- Migration must tolerate partially empty legacy values.
- If migration cannot fully preserve a legacy field, the fallback must still produce a valid provider that can launch.

### Post-migration compatibility

After successful migration:

- new UI edits only provider data
- runtime generation reads only provider data plus global settings
- legacy relay-profile data is ignored for normal operation

A temporary compatibility reader may remain for safety, but it should not become a second editing path.

## Import Compatibility

### Existing external import flow

The current `ccs_import` path should be upgraded so imports create providers directly instead of relay profiles.

That means:

- keep the current external source parsing logic where it is still useful
- change its output target from `RelayProfile` to the new provider model
- preserve de-duplication behavior based on stable provider identity semantics

### Supported import sources in this phase

1. Automatic migration from legacy local relay profiles
2. Existing external Codex provider import path
3. Optional direct import from cc-switch-compatible provider datasets if the format is already accessible with reasonable effort

The first two are required for this redesign. The third is nice-to-have and should only be included if it does not delay the core refactor.

## Manager UI Design

Add a dedicated provider configuration page in the manager app, modeled after the cc-switch interaction pattern.

### Layout

- left column: provider list
- right column: provider detail/edit form
- top or inline actions: add, delete, duplicate if needed later, set active, import

### Core interactions

- create provider
- edit provider fields
- delete provider
- set provider as active
- import providers from supported external sources
- reflect active provider clearly in the list

### Fields exposed in phase one

Expose the core provider fields and the provider configuration payload needed to generate runtime config. The form should prioritize usability over replicating every relay profile field one-to-one.

Recommended visible fields:

- provider name
- base URL / upstream endpoint
- API key or auth content entry
- protocol/API mode
- provider config content
- provider auth content
- optional website URL
- optional notes

If some fields are better represented as raw config editors than deeply structured controls, prefer the smallest UI that keeps the system understandable and editable.

### UI behavior changes

- Remove or demote relay-profile-centered editing controls from settings screens.
- Keep settings pages focused on global app configuration.
- Route all provider editing to the dedicated provider page.

## Runtime and Launch Integration

The launch pipeline in core code should stop assuming relay profiles are present in settings. Instead, it should request the active provider and build the runtime configuration from it.

This includes any code path that currently reads:

- relay base URL
- relay API key
- relay protocol/mode
- relay provider config TOML
- relay auth contents

The generation step should produce the same effective live files as before for equivalent user configuration, but from the new source model.

## Error Handling and Recovery

- If `activeProviderId` is missing or points to a deleted provider, select a deterministic fallback provider and surface that state in the UI.
- If provider storage is unreadable, fail with a clear manager/backend message instead of silently reverting to legacy state.
- If imported or migrated data is malformed, keep unaffected providers and report which records were skipped.
- If runtime config generation fails for the active provider, the manager should show a precise message tied to that provider.

## Testing Strategy

### Rust/backend tests

Add tests for:

- provider store CRUD
- relayProfiles -> providers migration
- active provider mapping from legacy state
- import flow producing providers directly
- runtime config generation from provider data
- fallback behavior when active provider is missing

### UI tests

Add tests for:

- provider list rendering
- selecting active provider
- editing and saving provider fields
- creating and deleting providers
- import action updating the list

### Regression focus

Pay special attention to preserving current runtime behavior for equivalent configurations. The redesign is structural, but users should still be able to launch with the same effective upstream settings after migration.

## Implementation Order

1. Add provider domain model and provider store.
2. Add migration from legacy relay profiles.
3. Update importers to create providers.
4. Refactor runtime/launch/config generation to read active provider.
5. Add Tauri commands for provider CRUD and selection.
6. Build the manager provider configuration screen.
7. Remove or demote relay-profile editing paths.
8. Add and run regression tests.

## Open Decisions Resolved

The following decisions are fixed for this implementation:

- Providers are the only source of truth going forward.
- The redesign should be highly compatible with cc-switch’s provider model, but not a full clone.
- `BackendSettings` stores global app state and active provider selection, not provider details.
- Migration from `relayProfiles` is automatic and one-time.
- Existing external import support remains, but targets providers directly.

## Success Criteria

The redesign is complete when:

- users can manage providers entirely from a dedicated manager page
- existing relay-profile users are migrated automatically without losing launchability
- active-provider selection drives generated runtime config
- external provider import creates usable providers directly
- normal operation no longer depends on `relayProfiles` as editable state
