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

- Checked: HarmonyOS app, protocol 2.0, cloud service and desktop implementation are
  separable. The only renderer dependency was the legacy
  `window.__codexPlusMobileRemoteCommand` hook, which official 1.3.0 does not install by default.
- Decision: keep app/cloud/protocol/runtime in `apps/xuan-plus-remote`; migrate the runtime into an
  independent Cargo crate, replace official-core imports with narrow path/CDP/diagnostic shims,
  and restore the renderer hook through the official User Scripts mechanism.
- Files: `apps/xuan-plus-remote/bridge/Cargo.toml`, `src/**`, `tests/**`, `user-scripts/**`, the
  mobile bridge contract, and `tools/xuan-bridge/tests/mobile_forwarding.rs`.
- Verification: 36 Remote Bridge Rust tests pass, with one explicit read-only live-data test
  ignored. Strict Clippy, release build, `--version`, User Script execution, six HTTP endpoints,
  `xuan-bridge` forwarding, the 39-message protocol contract and all 41 fixtures pass.
- Risk/next: signed HAP, real cloud pairing, reconnect, send/stop and complete-reply behavior still
  require a redacted real-device acceptance run.

## Stage 6: Upgrade And Rollback

- Checked: local marketplace registration, all three plugin installs and installed-cache MCP
  startup pass in an isolated `CODEX_HOME`.
- Decision: version the official host, plugins and `xuan-bridge` independently. Release Remote
  Bridge, HarmonyOS app and cloud together as 1.0.0 with protocol 2.0 and cloud schema 8 only.
- Files: `docs/migration/compatibility-matrix.md` and this status report.
- Verification: legacy desktop settings migration creates read-only local backups without copying
  API keys. Cloud deployment backs up the current binary, schema 7 database and configuration
  before replacing the single running service; it does not migrate cloud rows. Local loopback,
  deployment contract, Bash syntax, automatic rollback and manual rollback reversal checks pass.
- Risk/next: build the Linux x86-64 artifact on the existing release host, run the same pre-deploy
  gates there, and complete signed-device acceptance before production cutover.

## Stage 7: Cloud In-Place Upgrade

- Checked: the existing production workflow uses `workagents-remote-cloud.service`,
  `/usr/local/sbin/workagents-remote-deploy`, `/etc/workagents-remote/dev.env`, the
  `WORKAGENTS_REMOTE_*` variables, `/var/lib/workagents-remote-dev/remote.sqlite3`, and the
  existing `/workagents-remote` nginx routes. No parallel cloud deployment script exists.
- Decision: preserve every operational identifier. Upgrade the service in place to Remote 1.0.0,
  protocol 2.0 and schema 8; reject protocol 1.x and schema 7 at runtime.
- Files: `apps/xuan-plus-remote/cloud-service/**`, protocol schema/fixtures, HarmonyOS protocol
  constants, Remote Bridge transport constants, shared service config, compatibility matrix and
  `docs/migration/cloud-in-place-upgrade-and-rollback.md`.
- Verification: cloud tests report 45 passed and one explicit integration-server test ignored;
  strict Clippy, debug/release builds, `--version`, loopback health, protocol 1.5 rejection,
  unauthenticated rejection, deployment contract, Bash syntax and rollback rehearsal all pass.
  HarmonyOS API 20 build succeeds; the unsigned HAP retains
  `com.dyys.workagents.remote.dev`, version `1.0.0`, and versionCode `1000010`.
- Risk/next: the repository contains no production SSH host, port or key by design. Production
  release is therefore not executed in this session. Use the existing release host and upload
  workflow, then run the recorded in-place deploy command only after its Linux gates pass.
