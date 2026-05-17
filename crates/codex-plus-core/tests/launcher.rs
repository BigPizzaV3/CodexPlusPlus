use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use codex_plus_core::app_paths::{
    build_codex_executable, codex_app_version, find_latest_codex_app_dir,
    find_latest_codex_app_dir_from_roots, find_macos_codex_app, packaged_app_user_model_id,
    user_data_candidates_from,
};
use codex_plus_core::launcher::{
    CodexLaunch, DefaultLaunchHooks, LaunchHooks, LaunchOptions, MacosCleanupPolicy,
    build_codex_arguments, build_codex_command, build_macos_cleanup_command,
    build_macos_open_command, build_packaged_activation, codex_process_environment_from,
    launch_and_inject_with_hooks, with_temporary_proxy_environment,
};
#[cfg(windows)]
use codex_plus_core::launcher::{WindowsProcessControlStrategy, windows_process_control_strategy};
use codex_plus_core::ports::select_platform_loopback_port_with;
use codex_plus_core::proxy::{detect_local_proxy_with, has_proxy_environment};
use codex_plus_core::settings::BackendSettings;
use codex_plus_core::status::StatusStore;

#[test]
fn app_paths_find_latest_windows_package_prefers_highest_version_app_dir() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("OpenAI.Codex_1.2.3.0_x64__abc/app")).unwrap();
    std::fs::create_dir_all(temp.path().join("OpenAI.Codex_26.429.8261.0_x64__abc/app")).unwrap();
    std::fs::create_dir_all(temp.path().join("OpenAI.Codex_not-a-version_x64__abc")).unwrap();

    let latest = find_latest_codex_app_dir(temp.path()).unwrap();

    assert_eq!(
        latest,
        temp.path().join("OpenAI.Codex_26.429.8261.0_x64__abc/app")
    );
}

#[test]
fn app_paths_find_latest_windows_package_returns_package_when_app_dir_missing() {
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("OpenAI.Codex_26.429.8261.0_x64__abc");
    std::fs::create_dir_all(&package).unwrap();

    assert_eq!(find_latest_codex_app_dir(temp.path()).unwrap(), package);
}

#[test]
fn app_paths_find_latest_windows_package_checks_roots_before_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("WindowsApps");
    std::fs::create_dir_all(root.join("OpenAI.Codex_1.0.0.0_x64__abc/app")).unwrap();
    std::fs::create_dir_all(root.join("OpenAI.Codex_26.513.3673.0_x64__abc/app")).unwrap();

    let latest = find_latest_codex_app_dir_from_roots(&[root]).unwrap();

    assert!(latest.ends_with("OpenAI.Codex_26.513.3673.0_x64__abc/app"));
}

#[test]
fn app_paths_extracts_codex_version_from_windows_package_app_dir() {
    let app_dir =
        PathBuf::from(r"C:\Program Files\WindowsApps\OpenAI.Codex_26.513.3673.0_x64__abc\app");

    assert_eq!(
        codex_app_version(&app_dir).as_deref(),
        Some("26.513.3673.0")
    );
}

#[test]
fn app_paths_user_data_candidates_include_local_and_roaming_variants() {
    let local = PathBuf::from(r"C:\Users\me\AppData\Local");
    let roaming = PathBuf::from(r"C:\Users\me\AppData\Roaming");

    let candidates = user_data_candidates_from(Some(&local), Some(&roaming));

    assert_eq!(
        candidates,
        vec![
            local.join("OpenAI").join("Codex"),
            local.join("OpenAI.Codex"),
            local.join("Codex"),
            roaming.join("OpenAI").join("Codex"),
            roaming.join("OpenAI.Codex"),
            roaming.join("Codex"),
        ]
    );
}

#[test]
fn app_paths_find_macos_codex_app_prefers_first_search_root_and_known_names() {
    let temp = tempfile::tempdir().unwrap();
    let system_root = temp.path().join("Applications");
    let user_root = temp.path().join("Users/me/Applications");
    let system_app = system_root.join("OpenAI Codex.app");
    let user_app = user_root.join("Codex.app");
    std::fs::create_dir_all(&system_app).unwrap();
    std::fs::create_dir_all(&user_app).unwrap();

    assert_eq!(
        find_macos_codex_app(&[system_root, user_root]).unwrap(),
        system_app
    );
}

#[test]
fn app_paths_build_macos_bundle_executable() {
    let app = PathBuf::from("/Applications/OpenAI Codex.app");

    assert_eq!(
        build_codex_executable(&app),
        PathBuf::from("/Applications/OpenAI Codex.app/Contents/MacOS/Codex")
    );
}

#[test]
fn launcher_builds_debug_arguments_and_commands() {
    let app_dir = PathBuf::from(r"C:\Codex\app");

    assert_eq!(
        build_codex_arguments(9229),
        vec![
            "--remote-debugging-port=9229".to_string(),
            "--remote-allow-origins=http://127.0.0.1:9229".to_string(),
        ]
    );
    let command = build_codex_command(&app_dir, 9229);
    assert_eq!(command[1], "--remote-debugging-port=9229");
    assert_eq!(command[2], "--remote-allow-origins=http://127.0.0.1:9229");
}

#[test]
fn launcher_constructs_windows_packaged_activation_without_real_app() {
    let app_dir = PathBuf::from(
        r"C:\Program Files\WindowsApps\OpenAI.Codex_26.506.2212.0_x64__2p2nqsd0c76g0\app",
    );

    assert_eq!(
        packaged_app_user_model_id(&app_dir).unwrap(),
        "OpenAI.Codex_2p2nqsd0c76g0!App"
    );
    assert_eq!(
        build_packaged_activation(&app_dir, 9229).unwrap(),
        CodexLaunch::PackagedActivation {
            app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
            arguments: "--remote-debugging-port=9229 --remote-allow-origins=http://127.0.0.1:9229"
                .to_string(),
            process_id: None,
        }
    );
}

#[test]
fn launcher_packaged_activation_can_preserve_process_id() {
    let launch = CodexLaunch::PackagedActivation {
        app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
        arguments: "--remote-debugging-port=9229".to_string(),
        process_id: Some(4242),
    };

    assert_eq!(launch.process_id(), Some(4242));
}

#[cfg(windows)]
#[test]
fn launcher_windows_packaged_process_management_uses_native_api() {
    assert_eq!(
        windows_process_control_strategy(),
        WindowsProcessControlStrategy::NativeWindowsApi
    );
}

#[test]
fn launcher_macos_open_command_waits_for_app_exit() {
    let command = build_macos_open_command(Path::new("/Applications/Codex.app"), 9229);

    assert_eq!(command[0], "open");
    assert!(command.contains(&"-W".to_string()));
    assert!(command.contains(&"-a".to_string()));
    assert!(command.contains(&"--args".to_string()));
    assert!(command.contains(&"--remote-debugging-port=9229".to_string()));
}

#[test]
fn launcher_packaged_activation_temporarily_applies_proxy_environment() {
    temp_env_remove("HTTP_PROXY");
    temp_env_remove("HTTPS_PROXY");
    temp_env_remove("ALL_PROXY");
    temp_env_set("UNRELATED_PROXY_TEST", "keep");
    let mut env = HashMap::new();
    env.insert(
        "HTTP_PROXY".to_string(),
        "http://127.0.0.1:7897".to_string(),
    );
    env.insert(
        "HTTPS_PROXY".to_string(),
        "http://127.0.0.1:7897".to_string(),
    );
    env.insert("ALL_PROXY".to_string(), "http://127.0.0.1:7897".to_string());

    let seen = with_temporary_proxy_environment(&env, || {
        (
            std::env::var("HTTP_PROXY").ok(),
            std::env::var("HTTPS_PROXY").ok(),
            std::env::var("ALL_PROXY").ok(),
        )
    });

    assert_eq!(
        seen,
        (
            Some("http://127.0.0.1:7897".to_string()),
            Some("http://127.0.0.1:7897".to_string()),
            Some("http://127.0.0.1:7897".to_string()),
        )
    );
    assert!(std::env::var("HTTP_PROXY").is_err());
    assert!(std::env::var("HTTPS_PROXY").is_err());
    assert!(std::env::var("ALL_PROXY").is_err());
    assert_eq!(
        std::env::var("UNRELATED_PROXY_TEST").ok().as_deref(),
        Some("keep")
    );
    temp_env_remove("UNRELATED_PROXY_TEST");
}

#[test]
fn ports_windows_falls_back_to_ephemeral_when_requested_is_busy() {
    let selected = select_platform_loopback_port_with(9229, true, |_| false, || 43001);

    assert_eq!(selected, 43001);
}

#[test]
fn ports_non_windows_keeps_requested_even_when_busy() {
    let selected = select_platform_loopback_port_with(9229, false, |_| false, || 43001);

    assert_eq!(selected, 9229);
}

#[test]
fn proxy_detects_first_local_proxy_port_and_respects_existing_environment() {
    assert_eq!(
        detect_local_proxy_with(|port| port == 7890),
        Some("http://127.0.0.1:7890".to_string())
    );

    let env = HashMap::from([(
        "HTTPS_PROXY".to_string(),
        "http://127.0.0.1:9999".to_string(),
    )]);
    assert!(has_proxy_environment(&env));
    let process_env = codex_process_environment_from(&env, || {
        panic!("proxy auto-detection should not run when env already has proxy")
    });
    assert_eq!(
        process_env.get("HTTPS_PROXY").map(String::as_str),
        Some("http://127.0.0.1:9999")
    );
}

#[tokio::test]
async fn default_helper_serves_backend_status_over_http() {
    let hooks = DefaultLaunchHooks::default();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    hooks.start_helper(port).await.unwrap();
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/backend/status"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let payload: serde_json::Value = response.json().await.unwrap();
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["transport"], "http-helper");
    hooks.shutdown_helper(port).await;
}

#[tokio::test]
async fn launch_lifecycle_runs_sync_before_launch_writes_success_and_shutdowns_on_exit() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone())
        .with_settings(BackendSettings {
            provider_sync_enabled: true,
            ..BackendSettings::default()
        })
        .with_launch_result(CodexLaunch::Process {
            command: vec!["codex".to_string()],
            wait_strategy: codex_plus_core::launcher::ProcessWaitStrategy::TrackedChild,
            macos_cleanup_policy: None,
        });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir.clone()),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "provider-sync",
            "start-helper:57321",
            "launch:9229",
            "inject:9229:57321",
            "status:running",
            "wait-codex",
            "shutdown-helper:57321",
        ]
    );
    assert_eq!(
        handle
            .status_store
            .load_latest()
            .unwrap()
            .unwrap()
            .codex_app
            .as_deref(),
        Some(app_dir.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn launch_lifecycle_keeps_js_injection_in_relay_mode() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        launch_mode: codex_plus_core::settings::LaunchMode::Relay,
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "start-helper:57321",
            "launch:9229",
            "inject:9229:57321",
            "status:running",
            "wait-codex",
            "shutdown-helper:57321",
        ]
    );
}

#[tokio::test]
async fn launch_lifecycle_skips_helper_and_injection_when_enhancements_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        enhancements_enabled: false,
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "launch:9229",
            "status:running",
            "wait-codex",
        ]
    );
}

#[tokio::test]
async fn launch_lifecycle_writes_failure_and_cleans_helper_when_injection_fails() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_inject_error("inject failed");

    let error = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store: status_store.clone(),
        },
        &hooks,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("inject failed"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "start-helper:57321",
            "launch:9229",
            "inject:9229:57321",
            "shutdown-helper:57321",
            "terminate-codex",
            "status:failed",
        ]
    );
    let status = status_store.load_latest().unwrap().unwrap();
    assert_eq!(status.status, "failed");
    assert!(status.message.contains("inject failed"));
}

#[tokio::test]
async fn launch_lifecycle_cleans_helper_when_launch_fails_after_helper_started() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_launch_error("launch failed");

    let error = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store: status_store.clone(),
        },
        &hooks,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("launch failed"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "start-helper:57321",
            "launch:9229",
            "shutdown-helper:57321",
            "status:failed",
        ]
    );
}

#[tokio::test]
async fn launch_lifecycle_cleans_helper_and_codex_when_status_save_fails() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(temp.path().join("status-parent-file"), "not a directory").unwrap();
    let status_store = StatusStore::new(
        temp.path()
            .join("status-parent-file")
            .join("latest-status.json"),
    );
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks =
        FakeHooks::new(events.clone()).with_launch_result(CodexLaunch::PackagedActivation {
            app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
            arguments: "--remote-debugging-port=9229".to_string(),
            process_id: Some(4242),
        });

    let error = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("failed to create directory"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "start-helper:57321",
            "launch:9229",
            "inject:9229:57321",
            "shutdown-helper:57321",
            "terminate-packaged:4242",
            "status:failed",
        ]
    );
}

#[tokio::test]
async fn launch_lifecycle_terminates_packaged_process_id_when_injection_fails() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone())
        .with_launch_result(CodexLaunch::PackagedActivation {
            app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
            arguments: "--remote-debugging-port=9229".to_string(),
            process_id: Some(4242),
        })
        .with_inject_error("inject failed");

    let error = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("inject failed"));
    assert!(
        events
            .lock()
            .unwrap()
            .contains(&"terminate-packaged:4242".to_string())
    );
}

#[tokio::test]
async fn default_provider_sync_enabled_fails_instead_of_silently_skipping() {
    let hooks = FakeHooks::new(Arc::new(Mutex::new(Vec::new()))).with_provider_sync_unsupported();

    let error = hooks
        .run_provider_sync()
        .await
        .expect_err("default-style provider sync should be explicit");

    assert!(
        error
            .to_string()
            .contains("provider sync requires launcher hooks")
    );
}

#[test]
fn launcher_macos_cleanup_command_targets_specific_app_bundle() {
    let command = build_macos_cleanup_command(
        Path::new("/Applications/OpenAI Codex.app"),
        MacosCleanupPolicy::QuitIfNotPreviouslyRunning,
    )
    .expect("cleanup command should be allowed");

    assert_eq!(command[0], "osascript");
    assert!(command.iter().any(|part| part.contains("OpenAI Codex")));
    assert!(!command.iter().any(|part| part == "Codex"));
}

#[test]
fn launcher_macos_cleanup_is_skipped_when_app_was_already_running() {
    let command = build_macos_cleanup_command(
        Path::new("/Applications/OpenAI Codex.app"),
        MacosCleanupPolicy::SkipQuitBecauseAlreadyRunning,
    );

    assert_eq!(command, None);
}

#[tokio::test]
async fn default_launch_hooks_provider_sync_enabled_returns_explicit_error() {
    let error = DefaultLaunchHooks::default()
        .run_provider_sync()
        .await
        .expect_err("default provider sync should not silently skip");

    assert!(
        error
            .to_string()
            .contains("provider sync requires launcher hooks")
    );
}

#[derive(Clone)]
struct FakeHooks {
    events: Arc<Mutex<Vec<String>>>,
    settings: BackendSettings,
    launch_result: CodexLaunch,
    launch_error: Option<String>,
    inject_error: Option<String>,
    provider_sync_unsupported: bool,
}

impl FakeHooks {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            events,
            settings: BackendSettings::default(),
            launch_result: CodexLaunch::Process {
                command: vec!["codex".to_string()],
                wait_strategy: codex_plus_core::launcher::ProcessWaitStrategy::TrackedChild,
                macos_cleanup_policy: None,
            },
            launch_error: None,
            inject_error: None,
            provider_sync_unsupported: false,
        }
    }

    fn with_settings(mut self, settings: BackendSettings) -> Self {
        self.settings = settings;
        self
    }

    fn with_launch_result(mut self, launch_result: CodexLaunch) -> Self {
        self.launch_result = launch_result;
        self
    }

    fn with_inject_error(mut self, message: &str) -> Self {
        self.inject_error = Some(message.to_string());
        self
    }

    fn with_launch_error(mut self, message: &str) -> Self {
        self.launch_error = Some(message.to_string());
        self
    }

    fn with_provider_sync_unsupported(mut self) -> Self {
        self.provider_sync_unsupported = true;
        self
    }

    fn event(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }
}

fn temp_env_set(key: &str, value: &str) {
    unsafe {
        std::env::set_var(key, value);
    }
}

fn temp_env_remove(key: &str) {
    unsafe {
        std::env::remove_var(key);
    }
}

#[async_trait::async_trait(?Send)]
impl LaunchHooks for FakeHooks {
    fn resolve_app_dir(&self, app_dir: Option<&Path>) -> anyhow::Result<PathBuf> {
        app_dir
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("missing app dir"))
    }

    fn select_debug_port(&self, requested: u16) -> u16 {
        self.event(format!("select-debug:{requested}"));
        requested
    }

    fn select_helper_port(&self, requested: u16) -> u16 {
        self.event(format!("select-helper:{requested}"));
        requested
    }

    async fn load_settings(&self) -> anyhow::Result<BackendSettings> {
        self.event("load-settings");
        Ok(self.settings.clone())
    }

    async fn run_provider_sync(&self) -> anyhow::Result<()> {
        self.event("provider-sync");
        if self.provider_sync_unsupported {
            anyhow::bail!("provider sync requires launcher hooks");
        }
        Ok(())
    }

    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()> {
        self.event(format!("start-helper:{helper_port}"));
        Ok(())
    }

    async fn launch_codex(&self, app_dir: &Path, debug_port: u16) -> anyhow::Result<CodexLaunch> {
        assert!(app_dir.ends_with("Codex.app"));
        self.event(format!("launch:{debug_port}"));
        if let Some(message) = &self.launch_error {
            anyhow::bail!(message.clone());
        }
        Ok(self.launch_result.clone())
    }

    async fn inject(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        self.event(format!("inject:{debug_port}:{helper_port}"));
        if let Some(message) = &self.inject_error {
            anyhow::bail!(message.clone());
        }
        Ok(())
    }

    async fn write_status(&self, status: &str) {
        self.event(format!("status:{status}"));
    }

    async fn wait_for_codex_exit(&self, _launch: &CodexLaunch) -> anyhow::Result<()> {
        self.event("wait-codex");
        Ok(())
    }

    async fn shutdown_helper(&self, helper_port: u16) {
        self.event(format!("shutdown-helper:{helper_port}"));
    }

    async fn terminate_codex(&self, launch: &CodexLaunch) {
        if let Some(process_id) = launch.process_id() {
            self.event(format!("terminate-packaged:{process_id}"));
        } else {
            self.event("terminate-codex");
        }
    }
}
