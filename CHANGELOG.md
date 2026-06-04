# Changelog

## 1.1.8 - 2026-05-26

- Added upstream branch worktree support for creating and selecting independent workspaces from an upstream repository and branch.
- Added upstream branch listing, default handling, remote parsing, and related worktree creation APIs and tests.
- Optimized provider sync to preserve rollout file mtimes and reduce unnecessary session status changes after sync.
- Added an independent Tools and Plugins page to manage Codex++ / Codex MCP servers, skills, and plugins without binding them to a single provider.
- Provider switching now merges the currently enabled tools and plugins while avoiding provider-specific configuration leaking into the shared configuration.
- Tool and plugin lists now read enabled state live from the current Codex configuration and support direct toggles and deletion.
- Adjusted shared configuration extraction to be manual, reducing automatic overwrites and configuration pollution.
- Fixed provider switching isolation so `model_catalog_json`, legacy `model_provider`, historical provider tables, and old `auth.json` are not carried into the new provider.
- Fixed pure API mode not writing the API key to `auth.json`, and pinned the provider name to `CodexPlusPlus`.
- Optimized model catalog writing so it can merge with the original model catalog and show the real path in preview.
- Added model insertion mode, model list, context size, compacted context size, and target capability settings to the provider configuration page.
- Hidden model list and model insertion mode controls in official mode when they only apply to mixed API key scenarios.
- Moved Base URL, API Key, and upstream protocol before the model list, and moved test model plus context options into More Options.
- Fixed duplicate `model_reasoning_effort` and `plan_mode_reasoning_effort` writes that could break TOML parsing.
- Fixed duplicate plugin tables, empty configuration bodies, and boolean parsing issues that could break configuration parsing.
- Optimized the provider detail page layout by keeping the top back button and notice area fixed, increasing the default window size, and reducing the top gap.
- Removed the checksum blocker during script installation to avoid failed installs when market script checksums are inconsistent.
- Cleaned up login, active provider, and configuration path fields that no longer need to be shown on the About and Status pages.
- Centered notice messages to avoid covering the restart button.
- Updated discussion group QR code, README text, and macOS DMG packaging script.
