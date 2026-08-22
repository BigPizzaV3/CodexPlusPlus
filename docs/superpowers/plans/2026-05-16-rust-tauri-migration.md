# Rust/Tauri Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Python backend with a Rust core, a no-window silent launcher, and a Tauri management tool while preserving the existing Codex App injection behavior.

**Architecture:** Add a Rust workspace beside the current Python package, port behavior in verifiable slices, then switch the installed entry points to Rust and remove Python only after parity is proven. The only user-facing entry points are `Codex++` silent launch and `Codex++ 管理工具`; there is no separate CLI.

**Tech Stack:** Rust 1.95, Cargo workspace, Tauri 2, TypeScript/Vite, `rusqlite`, `serde`, `serde_json`, `tokio`, `reqwest`, `tungstenite` or `tokio-tungstenite`, Windows PowerShell shortcut generation, macOS app bundle generation, existing `renderer-inject.js`, existing pytest suite as behavior reference during migration.

---

## Scope Check

The design covers several subsystems: Rust workspace setup, data operations, CDP bridge, silent launcher, management UI, install/update packaging, watcher behavior, documentation, and Python removal. This plan is intentionally staged. Each task below should produce working, testable software and should be committed independently. If a task becomes large during execution, split it into a sub-plan before writing production code.

## File Structure

- Create `Cargo.toml`: workspace root with members under `crates/` and `apps/`.
- Create `crates/codex-plus-core/`: shared launch, CDP, bridge, settings, logs, diagnostics, path resolution, install/update primitives.
- Create `crates/codex-plus-data/`: SQLite, backup, markdown export, provider sync, and filesystem data operations.
- Create `apps/codex-plus-launcher/`: no-window silent launcher binary used by the `Codex++` shortcut.
- Create `apps/codex-plus-manager/`: Tauri management console used by `Codex++ 管理工具`.
- Move or copy runtime assets into `assets/`: icons, sponsor images, and `renderer-inject.js` source used by Rust packaging.
- Create `tests/fixtures/`: shared SQLite, rollout, and settings fixtures for Rust integration tests.
- Modify `README.md` and `README_EN.md`: replace Python usage with Rust/Tauri installation and two-entry behavior.
- Modify `setup.bat`: eventually bootstrap or run the management tool instead of Python setup.
- Remove `pyproject.toml`, `codex_session_delete/`, and Python tests only after Rust parity is complete.

---

### Task 1: Rust Workspace Skeleton

**Files:**
- Create: `Cargo.toml`
- Create: `.cargo/config.toml`
- Create: `crates/codex-plus-core/Cargo.toml`
- Create: `crates/codex-plus-core/src/lib.rs`
- Create: `crates/codex-plus-core/src/version.rs`
- Create: `crates/codex-plus-data/Cargo.toml`
- Create: `crates/codex-plus-data/src/lib.rs`
- Create: `apps/codex-plus-launcher/Cargo.toml`
- Create: `apps/codex-plus-launcher/src/main.rs`
- Create: `apps/codex-plus-manager/package.json`
- Create: `apps/codex-plus-manager/src-tauri/Cargo.toml`
- Create: `apps/codex-plus-manager/src-tauri/src/lib.rs`
- Create: `apps/codex-plus-manager/src-tauri/src/main.rs`
- Create: `apps/codex-plus-manager/src/main.tsx`
- Create: `apps/codex-plus-manager/index.html`

- [ ] **Step 1: Write the workspace manifests**

Create root `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
  "crates/codex-plus-core",
  "crates/codex-plus-data",
  "apps/codex-plus-launcher",
  "apps/codex-plus-manager/src-tauri",
]

[workspace.package]
version = "1.0.8"
edition = "2024"
license = "MIT"
repository = "https://github.com/BigPizzaV3/CodexPlusPlus"

[workspace.dependencies]
anyhow = "1"
base64 = "0.22"
directories = "6"
fs2 = "0.4"
reqwest = { version = "0.12", features = ["json", "stream", "rustls-tls"], default-features = false }
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tempfile = "3"
thiserror = "2"
tokio = { version = "1", features = ["macros", "process", "rt-multi-thread", "time"] }
tokio-tungstenite = { version = "0.26", features = ["rustls-tls-webpki-roots"] }
url = "2"
uuid = { version = "1", features = ["v4"] }
```

Create `.cargo/config.toml`:

```toml
[alias]
xtest = "test --workspace --all-targets"
xcheck = "check --workspace --all-targets"
```

- [ ] **Step 2: Add core crate**

Create `crates/codex-plus-core/Cargo.toml`:

```toml
[package]
name = "codex-plus-core"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
anyhow.workspace = true
directories.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
```

Create `crates/codex-plus-core/src/lib.rs`:

```rust
pub mod version;
```

Create `crates/codex-plus-core/src/version.rs`:

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn exposes_workspace_version() {
        assert!(!VERSION.is_empty());
    }
}
```

- [ ] **Step 3: Add data crate**

Create `crates/codex-plus-data/Cargo.toml`:

```toml
[package]
name = "codex-plus-data"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
anyhow.workspace = true
base64.workspace = true
rusqlite.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
```

Create `crates/codex-plus-data/src/lib.rs`:

```rust
pub fn crate_ready() -> bool {
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_is_ready() {
        assert!(super::crate_ready());
    }
}
```

- [ ] **Step 4: Add silent launcher binary skeleton**

Create `apps/codex-plus-launcher/Cargo.toml`:

```toml
[package]
name = "codex-plus-launcher"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[[bin]]
name = "codex-plus-plus"
path = "src/main.rs"

[dependencies]
anyhow.workspace = true
codex-plus-core = { path = "../../crates/codex-plus-core" }
tokio.workspace = true
```

Create `apps/codex-plus-launcher/src/main.rs`:

```rust
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    println!("Codex++ launcher {}", codex_plus_core::version::VERSION);
    Ok(())
}
```

- [ ] **Step 5: Add Tauri manager skeleton**

Create `apps/codex-plus-manager/package.json`:

```json
{
  "name": "codex-plus-manager",
  "version": "1.0.8",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "tauri dev",
    "build": "tauri build",
    "check": "tsc --noEmit"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0.0",
    "@tauri-apps/cli": "^2.0.0",
    "typescript": "^5.8.0",
    "vite": "^6.0.0"
  },
  "devDependencies": {}
}
```

Create `apps/codex-plus-manager/index.html`:

```html
<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Codex++ 管理工具</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

Create `apps/codex-plus-manager/src/main.tsx`:

```tsx
const app = document.getElementById("app");

if (app) {
  app.innerHTML = `
    <main style="font-family: system-ui, sans-serif; padding: 24px">
      <h1>Codex++ 管理工具</h1>
      <p>Rust/Tauri migration shell is ready.</p>
    </main>
  `;
}
```

Create `apps/codex-plus-manager/src-tauri/Cargo.toml`:

```toml
[package]
name = "codex-plus-manager"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[lib]
name = "codex_plus_manager_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[[bin]]
name = "codex-plus-plus-manager"
path = "src/main.rs"

[dependencies]
codex-plus-core = { path = "../../../crates/codex-plus-core" }
serde.workspace = true
serde_json.workspace = true
tauri = { version = "2", features = [] }
```

Create `apps/codex-plus-manager/src-tauri/src/lib.rs`:

```rust
#[tauri::command]
fn backend_version() -> &'static str {
    codex_plus_core::version::VERSION
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![backend_version])
        .run(tauri::generate_context!())
        .expect("failed to run Codex++ manager");
}
```

Create `apps/codex-plus-manager/src-tauri/src/main.rs`:

```rust
fn main() {
    codex_plus_manager_lib::run();
}
```

- [ ] **Step 6: Run Rust checks**

Run:

```bash
cargo test --workspace --all-targets
```

Expected: Rust crates compile and the two skeleton tests pass. If Tauri context files are missing, add the generated `tauri.conf.json` and icons in the same task before committing.

- [ ] **Step 7: Commit skeleton**

```bash
git add Cargo.toml .cargo/config.toml crates apps
git commit -m "feat: add Rust Tauri workspace skeleton"
```

---

### Task 2: Shared Models, Settings, And Status Files

**Files:**
- Create: `crates/codex-plus-core/src/models.rs`
- Create: `crates/codex-plus-core/src/paths.rs`
- Create: `crates/codex-plus-core/src/settings.rs`
- Create: `crates/codex-plus-core/src/status.rs`
- Modify: `crates/codex-plus-core/src/lib.rs`

- [ ] **Step 1: Write model and settings tests**

Create tests inside the new modules for:

```rust
#[test]
fn settings_default_matches_python_behavior() {
    let settings = BackendSettings::default();
    assert!(!settings.provider_sync_enabled);
    assert!(!settings.cli_wrapper_enabled);
    assert_eq!(settings.cli_wrapper_api_key_env, "CUSTOM_OPENAI_API_KEY");
}

#[test]
fn settings_deserialize_uses_existing_json_keys() {
    let settings: BackendSettings = serde_json::from_str(
        r#"{"providerSyncEnabled":true,"cliWrapperEnabled":true,"cliWrapperBaseUrl":"https://example.test","cliWrapperApiKey":"sk-test","cliWrapperApiKeyEnv":""}"#,
    )
    .unwrap();
    assert!(settings.provider_sync_enabled);
    assert!(settings.cli_wrapper_enabled);
    assert_eq!(settings.cli_wrapper_base_url, "https://example.test");
    assert_eq!(settings.cli_wrapper_api_key, "sk-test");
    assert_eq!(settings.cli_wrapper_api_key_env, "CUSTOM_OPENAI_API_KEY");
}
```

- [ ] **Step 2: Implement models and settings**

Implement these public types:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionRef {
    pub session_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteStatus {
    ServerDeleted,
    LocalDeleted,
    Partial,
    Failed,
    Undone,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeleteResult {
    pub status: DeleteStatus,
    pub session_id: String,
    pub message: String,
    pub undo_token: Option<String>,
    pub backup_path: Option<String>,
}
```

Implement `BackendSettings` with serde field names matching the current JSON keys:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackendSettings {
    #[serde(rename = "providerSyncEnabled", default)]
    pub provider_sync_enabled: bool,
    #[serde(rename = "cliWrapperEnabled", default)]
    pub cli_wrapper_enabled: bool,
    #[serde(rename = "cliWrapperBaseUrl", default)]
    pub cli_wrapper_base_url: String,
    #[serde(rename = "cliWrapperApiKey", default)]
    pub cli_wrapper_api_key: String,
    #[serde(rename = "cliWrapperApiKeyEnv", default = "default_api_key_env", deserialize_with = "empty_as_default_api_key_env")]
    pub cli_wrapper_api_key_env: String,
}
```

Add `SettingsStore::load`, `save`, and `update` with atomic temp-file writes to `~/.codex-session-delete/settings.json`.

- [ ] **Step 3: Implement latest status file**

Create `LaunchStatus` with fields:

```rust
pub struct LaunchStatus {
    pub status: String,
    pub message: String,
    pub started_at_ms: u64,
    pub debug_port: Option<u16>,
    pub helper_port: Option<u16>,
    pub codex_app: Option<String>,
}
```

Implement `StatusStore::save_latest` and `StatusStore::load_latest` at `~/.codex-session-delete/latest-status.json`.

- [ ] **Step 4: Run tests and commit**

```bash
cargo test -p codex-plus-core settings status models
git add crates/codex-plus-core
git commit -m "feat: add Rust settings and status models"
```

---

### Task 3: Data Layer Parity

**Files:**
- Create: `crates/codex-plus-data/src/backup.rs`
- Create: `crates/codex-plus-data/src/storage.rs`
- Create: `crates/codex-plus-data/src/markdown.rs`
- Create: `crates/codex-plus-data/src/provider_sync.rs`
- Modify: `crates/codex-plus-data/src/lib.rs`
- Create: `crates/codex-plus-data/tests/storage_adapter.rs`
- Create: `crates/codex-plus-data/tests/provider_sync.rs`

- [ ] **Step 1: Port fixture behavior from Python tests**

Use these Python tests as references:

```text
tests/test_storage_adapter.py
tests/test_backup_store.py
tests/test_markdown_exporter.py
tests/test_provider_sync.py
```

Create Rust integration fixtures that cover:

- `sessions/messages` schema deletion and undo.
- `threads` schema deletion, related table cleanup, and rollout file backup.
- Workspace move updating both SQLite `cwd` and rollout first-line metadata.
- Sort key lookup for one and many sessions.
- Markdown export from rollout JSONL.
- Provider sync rewrite of rollout first-line `model_provider`, SQLite `threads.model_provider`, `has_user_event`, and `cwd`.

- [ ] **Step 2: Implement backup store**

Implement backup JSON compatible with current Python backups:

```rust
pub struct BackupStore {
    root: PathBuf,
}

impl BackupStore {
    pub fn new(root: impl Into<PathBuf>) -> Self;
    pub fn write_backup(&self, session_id: &str, source_db: &Path, tables: serde_json::Value) -> anyhow::Result<String>;
    pub fn read_backup(&self, token: &str) -> anyhow::Result<serde_json::Value>;
    pub fn path_for(&self, token: &str) -> PathBuf;
}
```

- [ ] **Step 3: Implement storage adapter**

Implement a `SQLiteStorageAdapter` with methods matching the current bridge needs:

```rust
pub fn delete_local(&self, session: &SessionRef) -> DeleteResult;
pub fn undo(&self, token: &str) -> DeleteResult;
pub fn find_archived_thread_by_title(&self, title: &str) -> Option<SessionRef>;
pub fn move_codex_thread_workspace(&self, session: &SessionRef, target_cwd: &str) -> serde_json::Value;
pub fn codex_thread_sort_key(&self, session: &SessionRef) -> serde_json::Value;
pub fn codex_thread_sort_keys(&self, sessions: &[SessionRef]) -> serde_json::Value;
```

- [ ] **Step 4: Implement markdown export and provider sync**

Mirror behavior in:

```text
codex_session_delete/markdown_exporter.py
codex_session_delete/provider_sync.py
```

Provider sync must lock with `~/.codex/tmp/provider-sync.lock`, backup writable state before changes, restore rollout first lines on failure, and return skipped rather than failing launch for lock or SQLite busy conditions.

- [ ] **Step 5: Run data tests and commit**

```bash
cargo test -p codex-plus-data --all-targets
git add crates/codex-plus-data
git commit -m "feat: port data operations to Rust"
```

---

### Task 4: CDP Bridge And Injection Runtime

**Files:**
- Create: `crates/codex-plus-core/src/cdp.rs`
- Create: `crates/codex-plus-core/src/bridge.rs`
- Create: `crates/codex-plus-core/src/assets.rs`
- Modify: `crates/codex-plus-core/src/lib.rs`
- Create: `crates/codex-plus-core/tests/cdp_bridge.rs`
- Copy asset: `assets/inject/renderer-inject.js`
- Copy assets: `assets/images/codex-plus-plus.ico`, `assets/images/codex-plus-plus.png`, sponsor images

- [ ] **Step 1: Write bridge script tests**

Test that the Rust bridge script defines:

```text
window.__codexSessionDeleteBridge
window.__codexSessionDeleteResolve
window.__codexSessionDeleteReject
codexSessionDeleteV2
```

Test that injection prefix includes:

```text
window.__CODEX_SESSION_DELETE_HELPER__
window.__CODEX_PLUS_SPONSOR_IMAGES__
```

- [ ] **Step 2: Implement CDP target discovery**

Implement:

```rust
pub async fn list_targets(debug_port: u16) -> anyhow::Result<Vec<CdpTarget>>;
pub fn pick_page_target(targets: &[CdpTarget]) -> anyhow::Result<CdpTarget>;
```

Selection must prefer page targets whose title or URL contains `codex`, then fall back to the first page target.

- [ ] **Step 3: Implement evaluate and bridge install**

Implement websocket calls for:

- `Runtime.enable`
- `Runtime.removeBinding`
- `Runtime.addBinding`
- `Page.addScriptToEvaluateOnNewDocument`
- `Runtime.evaluate`

Add a bridge loop that receives `Runtime.bindingCalled`, routes `(path, payload)` to a Rust handler, and resolves/rejects the browser promise by evaluating the resolve/reject expression.

- [ ] **Step 4: Implement asset loading**

Use `include_str!` and `include_bytes!` for injected JS and sponsor images. The injected JS should remain sourced from one canonical file path during the migration to avoid asset drift.

- [ ] **Step 5: Run tests and commit**

```bash
cargo test -p codex-plus-core cdp bridge assets
git add crates/codex-plus-core assets
git commit -m "feat: port CDP bridge injection to Rust"
```

---

### Task 5: Silent Launcher Runtime

**Files:**
- Create: `crates/codex-plus-core/src/app_paths.rs`
- Create: `crates/codex-plus-core/src/ports.rs`
- Create: `crates/codex-plus-core/src/proxy.rs`
- Create: `crates/codex-plus-core/src/launcher.rs`
- Modify: `apps/codex-plus-launcher/src/main.rs`
- Create: `crates/codex-plus-core/tests/launcher.rs`

- [ ] **Step 1: Port path and port tests**

Use these Python tests as references:

```text
tests/test_app_paths.py
tests/test_launcher_cli.py
tests/test_cdp.py
```

Test Windows packaged Codex path detection, macOS `.app` executable construction, debug argument construction, and loopback port fallback.

- [ ] **Step 2: Implement app path resolution and process environment**

Port:

```text
codex_session_delete/app_paths.py
codex_session_delete/launcher.py::codex_process_environment
codex_session_delete/launcher.py::build_codex_arguments
```

Keep proxy auto-detection ports:

```text
7897, 7890, 10809, 10808, 1080
```

- [ ] **Step 3: Implement Windows packaged app activation**

Port `activate_packaged_app` with Rust Windows APIs. Protect this behind `cfg(windows)`. Tests should validate command construction without requiring a real packaged app.

- [ ] **Step 4: Implement launch lifecycle**

Implement:

```rust
pub async fn launch_and_inject(options: LaunchOptions) -> anyhow::Result<LaunchHandle>;
```

It must:

- Select ports.
- Load settings.
- Run provider sync if enabled.
- Start helper/bridge runtime.
- Launch Codex.
- Retry injection.
- Write latest status on success/failure.
- Shut down helper resources when Codex exits.

- [ ] **Step 5: Implement no-window launcher binary**

`apps/codex-plus-launcher/src/main.rs` should call `launch_and_inject`. On Windows release builds, configure the binary as no-console if packaging requires it.

- [ ] **Step 6: Run launcher tests and commit**

```bash
cargo test -p codex-plus-core launcher ports app_paths proxy
cargo build -p codex-plus-launcher
git add crates/codex-plus-core apps/codex-plus-launcher
git commit -m "feat: add Rust silent launcher runtime"
```

---

### Task 6: Bridge Route Parity

**Files:**
- Create: `crates/codex-plus-core/src/routes.rs`
- Modify: `crates/codex-plus-core/src/bridge.rs`
- Modify: `crates/codex-plus-core/Cargo.toml`
- Create: `crates/codex-plus-core/tests/bridge_routes.rs`

- [ ] **Step 1: Write route tests**

Create tests covering all current route names:

```text
/settings/get
/settings/set
/user-scripts/list
/user-scripts/set-enabled
/user-scripts/set-script-enabled
/user-scripts/reload
/devtools/open
/backend/status
/backend/repair
/ads
/delete
/undo
/export-markdown
/archived-thread
/move-thread-workspace
/thread-sort-key
/thread-sort-keys
```

- [ ] **Step 2: Implement route dispatcher**

Implement a typed dispatcher:

```rust
pub async fn handle_bridge_request(ctx: BridgeContext, path: &str, payload: serde_json::Value) -> serde_json::Value;
```

Unknown routes must return:

```json
{"status":"failed","session_id":"","message":"Unknown bridge path"}
```

- [ ] **Step 3: Wire routes to data and runtime services**

Connect delete/export/move/sort/provider/settings/status routes to Rust implementations. Keep user script inventory behavior compatible with current `UserScriptManager`.

- [ ] **Step 4: Run route tests and commit**

```bash
cargo test -p codex-plus-core bridge_routes
git add crates/codex-plus-core
git commit -m "feat: port bridge route handling to Rust"
```

---

### Task 7: Tauri Management Console

**Files:**
- Create: `apps/codex-plus-manager/src/App.ts`
- Create: `apps/codex-plus-manager/src/styles.css`
- Modify: `apps/codex-plus-manager/src/main.tsx`
- Modify: `apps/codex-plus-manager/src-tauri/src/lib.rs`
- Create: `apps/codex-plus-manager/src-tauri/src/commands.rs`
- Create: `apps/codex-plus-manager/src-tauri/src/install.rs`

- [ ] **Step 1: Implement Tauri commands**

Expose commands:

```rust
backend_version
load_overview
launch_codex_plus
load_settings
save_settings
install_entrypoints
uninstall_entrypoints
repair_shortcuts
check_update
perform_update
read_latest_logs
copy_diagnostics
reset_settings
```

Each command returns serializable result objects with `status`, `message`, and relevant payload fields.

- [ ] **Step 2: Build workbench UI**

Implement the left navigation:

```text
Overview
Launch
Install
Update
Settings
Logs
Diagnostics
```

Use a compact operational layout. Avoid landing-page hero composition. Do not put cards inside cards. Keep controls dense and predictable.

- [ ] **Step 3: Implement Overview**

Show:

- Codex app found/missing.
- Silent shortcut installed/missing.
- Management shortcut installed/missing.
- Latest launch status.
- Current version.
- Update status.
- Quick actions: launch, repair shortcuts, open logs.

- [ ] **Step 4: Implement management screens**

Implement:

- Launch: manual launch, app path override, debug/helper port fields, repair backend.
- Install: install, uninstall, repair shortcuts, optional remove owned data.
- Update: check update, release summary, progress, install update.
- Settings: provider sync, Codex command wrapper settings, user scripts summary.
- Logs: latest log viewer with refresh and copy.
- Diagnostics: generated report with copy button.

- [ ] **Step 5: Build and smoke test**

```bash
cd apps/codex-plus-manager
npm install
npm run check
npm run build
```

Expected: TypeScript check passes and Tauri build succeeds.

- [ ] **Step 6: Commit management console**

```bash
git add apps/codex-plus-manager crates/codex-plus-core
git commit -m "feat: add Tauri management console"
```

---

### Task 8: Install, Uninstall, Update, And Watcher In Rust

**Files:**
- Create: `crates/codex-plus-core/src/install/windows.rs`
- Create: `crates/codex-plus-core/src/install/macos.rs`
- Create: `crates/codex-plus-core/src/install/mod.rs`
- Create: `crates/codex-plus-core/src/update.rs`
- Create: `crates/codex-plus-core/src/watcher.rs`
- Modify: `crates/codex-plus-core/src/lib.rs`
- Create: `crates/codex-plus-core/tests/installers.rs`
- Create: `crates/codex-plus-core/tests/updater.rs`

- [ ] **Step 1: Port installer script tests**

Use these Python tests as references:

```text
tests/test_windows_installer.py
tests/test_macos_installer.py
tests/test_installers.py
tests/test_setup_bat.py
tests/test_updater.py
tests/test_watcher.py
```

Test generated Windows shortcut scripts contain both:

```text
Codex++.lnk
Codex++ 管理工具.lnk
```

Test generated macOS app bundle metadata contains both:

```text
Codex++.app
Codex++ 管理工具.app
```

- [ ] **Step 2: Implement Windows install/uninstall**

Generate two shortcuts and one uninstall registry entry. The silent shortcut points to `codex-plus-plus.exe`; the management shortcut points to `codex-plus-plus-manager.exe`.

- [ ] **Step 3: Implement macOS install/uninstall**

Generate two app bundles:

```text
/Applications/Codex++.app
/Applications/Codex++ 管理工具.app
```

The silent bundle launches the no-window launcher. The management bundle launches the Tauri manager.

- [ ] **Step 4: Implement update in management core**

Port GitHub Release checking, version parsing, asset selection, download, and install orchestration. Expose it only through Tauri commands.

- [ ] **Step 5: Implement watcher behavior**

Port existing watcher behavior so management UI can install/remove/enable/disable it. Watcher management is not exposed as a separate CLI.

- [ ] **Step 6: Run tests and commit**

```bash
cargo test -p codex-plus-core installers updater watcher
git add crates/codex-plus-core
git commit -m "feat: port install update and watcher management to Rust"
```

---

### Task 9: Switch Runtime Assets And Documentation

**Files:**
- Modify: `README.md`
- Modify: `README_EN.md`
- Modify: `setup.bat`
- Modify: `.github` release workflow files if present
- Modify: `docs/images` only if icon packaging requires path updates

- [ ] **Step 1: Update README commands**

Replace Python commands with two-entry usage:

```text
双击 Codex++：静默启动增强版 Codex。
双击 Codex++ 管理工具：安装、卸载、更新、设置、日志和诊断。
```

Remove references to:

```text
python -m codex_session_delete launch
python -m codex_session_delete setup
python -m codex_session_delete update
```

- [ ] **Step 2: Update setup.bat**

Make `setup.bat` open or install the Rust/Tauri management tool. It should not call Python modules.

- [ ] **Step 3: Update release docs**

Document release artifacts:

```text
codex-plus-plus.exe
codex-plus-plus-manager.exe
Codex++.app
Codex++ 管理工具.app
```

- [ ] **Step 4: Run docs tests and commit**

```bash
python -m pytest -q tests/test_readme.py tests/test_setup_bat.py
git add README.md README_EN.md setup.bat .github docs
git commit -m "docs: document Rust Tauri entry points"
```

---

### Task 10: Python Removal Gate

**Files:**
- Delete: `codex_session_delete/`
- Delete: `pyproject.toml`
- Delete or replace: `tests/test_*.py` that only validate Python internals
- Modify: `.gitignore` if Python-only entries are obsolete

- [ ] **Step 1: Run full Rust verification**

```bash
cargo test --workspace --all-targets
cd apps/codex-plus-manager
npm run check
npm run build
```

Expected: all Rust tests pass and Tauri app builds.

- [ ] **Step 2: Run remaining Python docs tests**

```bash
python -m pytest -q tests/test_readme.py tests/test_setup_bat.py
```

Expected: pass, or replace these tests with Rust/Node equivalents before deleting Python test infrastructure.

- [ ] **Step 3: Manual smoke checks**

Verify:

- `Codex++` shortcut starts Codex without opening management UI.
- Codex App shows the Codex++ injected menu.
- Delete/undo/export/move/settings routes work from the injected UI.
- `Codex++ 管理工具` opens and shows Overview status.
- Management tool can repair shortcuts and read latest logs.
- Update check works with mocked or real release metadata.

- [ ] **Step 4: Remove Python backend**

Delete Python package and obsolete Python tests only after the checks above pass.

- [ ] **Step 5: Commit removal**

```bash
git add -A
git commit -m "chore: remove Python backend after Rust migration"
```

---

## Self-Review

- Spec coverage: the plan covers Rust core, data operations, CDP bridge, silent launcher, Tauri management tool, two desktop entry points, install/uninstall/update, watcher, docs, verification, and Python removal.
- Placeholder scan: no `TODO` or `TBD` placeholders are left. Tasks that are intentionally broad identify exact files, reference tests, expected APIs, and verification commands.
- Type consistency: the plan consistently uses `codex-plus-launcher` for the no-window launcher and does not introduce a separate user-facing CLI.
