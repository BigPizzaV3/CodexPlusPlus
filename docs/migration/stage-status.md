# Migration Stage Status

## Stage 0: Official 1.3.0 Baseline

- Checked: `v1.3.0` resolves to commit `be6a45852f9992a688f33be933f444ee098fc67a`.
- Decision: develop in `codex/migrate-official-1.3.0` and the separate
  `D:\WORK_GIT\XuanPlusPlus-migration` worktree so the legacy worktree stays untouched.
- Files: no official source changes; baseline checkpoint is commit `f4ff404`.
- Verification: Rust core/launcher/data compile. Full Tauri build is blocked because
  `apps/codex-plus-manager/dist` and frontend dependencies are absent.
- Risk/next: restore the official frontend dependency environment before release packaging.

## Stages 1-2: Bridge, Plugins And Search

- Checked: official plugin manifests support `skills` and `mcpServers`; Codex CLI accepts
  `.agents/plugins/marketplace.json`. A plugin cannot depend on files outside its package root.
- Decision: each plugin carries a local MCP adapter and calls the independent `xuan-bridge`.
  Search streams ripgrep output and enforces root, timeout, file-size and result-count bounds.
- Files: `.agents/plugins/marketplace.json`, `plugins/xuan-*`, `plugins/*contract.test.mjs`,
  and `tools/xuan-bridge`.
- Verification: manifest/MCP tests pass; search runs end to end through an installed plugin copy.
- Risk/next: the old injected search modal is not retained; add an official interactive App only
  if MCP/Skill workflow proves insufficient after user acceptance.

## Stages 3-4: Usage And Polish

- Checked: official 1.3.0 already implements per-thread token history; the legacy fork adds
  provider account usage and three polish protocols on top.
- Decision: keep official token history untouched. Put provider profiles in `xuan-plugins.json`,
  keep credentials in environment variables, and support generic/OwlAI usage plus Chat
  Completions/Responses/Anthropic polish in the bridge.
- Files: `tools/xuan-bridge/src/lib.rs`, provider contract tests, usage/polish MCP schemas,
  and the thin Composer user script.
- Verification: local mock-provider contracts and executable DOM smoke tests pass; secrets are
  asserted absent from bridge responses and migrated JSON.
- Risk/next: production provider variants and proxy behavior need acceptance tests with redacted
  real accounts; no credential is migrated automatically.

## Stage 5: Phone Connection

- Checked: HarmonyOS app, protocol 1.5, cloud service and legacy desktop implementation are
  separable. The only renderer dependency was the legacy
  `window.__codexPlusMobileRemoteCommand` hook, which official 1.3.0 does not install by default.
- Decision: keep app/cloud/protocol/runtime in `apps/xuan-plus-remote`; migrate the runtime into an
  independent Cargo crate, replace official-core imports with narrow path/CDP/diagnostic shims,
  and restore the renderer hook through the official User Scripts mechanism.
- Files: `apps/xuan-plus-remote/bridge/Cargo.toml`, `src/**`, `tests/**`, `user-scripts/**`, the
  mobile bridge contract, and `tools/xuan-bridge/tests/mobile_forwarding.rs`.
- Verification: 36 Remote Bridge Rust tests pass, including HTTP auth, task indexing, CDP source,
  DPAPI identity and legacy receipt tables. User-script execution, six HTTP endpoints,
  `xuan-bridge` forwarding, the 39-message protocol contract and all 41 fixtures pass.
- Risk/next: signed HAP, real cloud pairing, reconnect, send/stop and complete-reply behavior still
  require a redacted real-device acceptance run. The cloud crate offline test remains blocked by
  the missing cached `jsonwebtoken` crate.

## Stage 6: Upgrade And Rollback

- Checked: local marketplace registration, all three plugin installs and installed-cache MCP
  startup pass in an isolated `CODEX_HOME`.
- Decision: version official host, plugins, bridge, storage schema and remote protocol separately;
  expand compatibility only after contract tests pass on a candidate official tag.
- Files: `docs/migration/compatibility-matrix.md` and this status report.
- Verification: legacy settings migration creates read-only backups, namespaced JSON and SQLite
  migration records without copying API keys. A separate writable Remote DB preserves the legacy
  identity/receipt format, while an existing migrated DB is never overwritten.
- Risk/next: perform a parallel-run soak with real user data copied to a disposable directory and
  record release/rollback runbooks before replacing the legacy installation.
