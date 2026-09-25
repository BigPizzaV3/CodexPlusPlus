use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose, Engine as _};

pub const TASKBOARD_PORT: u16 = 47823;
pub const TASKBOARD_URL: &str = "http://127.0.0.1:47823/?host=codex";
const TASKBOARD_HOST_ENV: &str = "CODEX_TASKBOARD_HOST";
const TASKBOARD_SHARED_SECRET_ENV: &str = "TASKBOARD_SHARED_SECRET";
const LOOPBACK_TASKBOARD_HOST: &str = "127.0.0.1";
const LAN_TASKBOARD_HOST: &str = "0.0.0.0";
const TASKBOARD_BIND_HOST_HEADER: &str = "x-taskboard-bind-host";
const LAN_SHARING_WARNING: &str =
    "LAN sharing is enabled; requests require Basic auth and same-origin protection.";

fn taskboard_requested_host() -> Option<String> {
    let host = std::env::var(TASKBOARD_HOST_ENV).ok()?;
    let host = host.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn taskboard_shared_secret() -> Option<String> {
    let secret = std::env::var(TASKBOARD_SHARED_SECRET_ENV).ok()?;
    let secret = secret.trim();
    (!secret.is_empty()).then(|| secret.to_string())
}

fn taskboard_lan_sharing_enabled() -> bool {
    taskboard_requested_host().as_deref() == Some(LAN_TASKBOARD_HOST)
}

fn taskboard_expected_host() -> &'static str {
    if taskboard_lan_sharing_enabled() {
        LAN_TASKBOARD_HOST
    } else {
        LOOPBACK_TASKBOARD_HOST
    }
}

pub fn taskboard_launch_warning() -> Option<&'static str> {
    taskboard_lan_sharing_enabled().then_some(LAN_SHARING_WARNING)
}

pub fn wait_for_taskboard_health(timeout: Duration) -> bool {
    let started_at = Instant::now();
    while started_at.elapsed() < timeout {
        if taskboard_health_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }
    false
}

pub fn taskboard_health_ok() -> bool {
    taskboard_health_endpoint_ok() && taskboard_page_entry_ok()
}

fn taskboard_health_endpoint_ok() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], TASKBOARD_PORT));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(250)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(750)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(750)));
    let request = taskboard_health_request(taskboard_shared_secret().as_deref());
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).is_ok()
        && taskboard_health_response_matches(&response, taskboard_expected_host())
}

fn taskboard_page_entry_ok() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], TASKBOARD_PORT));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(250)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(750)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(750)));
    let request = taskboard_page_entry_request(taskboard_shared_secret().as_deref());
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).is_ok() && taskboard_page_entry_response_matches(&response)
}

fn taskboard_health_request(shared_secret: Option<&str>) -> String {
    let authorization = shared_secret
        .map(|secret| {
            let token = general_purpose::STANDARD.encode(format!("codex:{secret}"));
            format!("Authorization: Basic {token}\r\n")
        })
        .unwrap_or_default();
    format!(
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1:47823\r\n{authorization}Connection: close\r\n\r\n"
    )
}

fn taskboard_page_entry_request(shared_secret: Option<&str>) -> String {
    let authorization = shared_secret
        .map(|secret| {
            let token = general_purpose::STANDARD.encode(format!("codex:{secret}"));
            format!("Authorization: Basic {token}\r\n")
        })
        .unwrap_or_default();
    format!(
        "GET /?host=codex HTTP/1.1\r\nHost: 127.0.0.1:47823\r\n{authorization}Connection: close\r\n\r\n"
    )
}

fn taskboard_health_response_matches(response: &str, expected_host: &str) -> bool {
    if !response.contains(" 200 ") || !response.contains("\"status\":\"ok\"") {
        return false;
    }
    response.lines().any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        name.eq_ignore_ascii_case(TASKBOARD_BIND_HOST_HEADER) && value.trim() == expected_host
    })
}

fn taskboard_page_entry_response_matches(response: &str) -> bool {
    response.contains(" 200 ")
        && response.lines().any(|line| {
            line.to_ascii_lowercase()
                .starts_with("content-type: text/html")
        })
        && response.contains("<div id=\"root\"")
}

pub fn spawn_taskboard_service() -> anyhow::Result<()> {
    let lan_sharing_enabled = taskboard_lan_sharing_enabled();
    if lan_sharing_enabled && taskboard_shared_secret().is_none() {
        return Err(anyhow::anyhow!(
            "{TASKBOARD_HOST_ENV}=0.0.0.0 requires {TASKBOARD_SHARED_SECRET_ENV} for LAN sharing"
        ));
    }
    if taskboard_health_ok() {
        return Ok(());
    }
    let root = taskboard_root_from_current_exe();
    if !lan_sharing_enabled {
        if let Some(root) = root.as_ref() {
            // Embedded Taskboard only binds loopback; LAN sharing must use the Node service.
            let paths = crate::taskboard_embedded::EmbeddedTaskboardPaths::from_root(root.clone());
            if crate::taskboard_embedded::spawn(paths)? {
                return Ok(());
            }
        }
    }

    let mut command = taskboard_service_command(root.as_deref());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    command.spawn()?;
    Ok(())
}

fn taskboard_service_command(root: Option<&Path>) -> Command {
    if let Some(root) = root.filter(|root| root.join("server").join("index.mjs").is_file()) {
        let mut command = Command::new(taskboard_node_executable());
        command
            .current_dir(root)
            .arg(root.join("server").join("index.mjs"));
        command.env(TASKBOARD_HOST_ENV, taskboard_expected_host());
        return command;
    }

    let mut command = taskboard_cli_command();
    command.env(TASKBOARD_HOST_ENV, taskboard_expected_host());
    command
}

#[cfg(target_os = "windows")]
fn taskboard_cli_command() -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", "codex-taskboard"]);
    command
}

#[cfg(not(target_os = "windows"))]
fn taskboard_cli_command() -> Command {
    Command::new("codex-taskboard")
}

pub fn taskboard_root_from_current_exe() -> Option<PathBuf> {
    taskboard_root_from_env().or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|path| taskboard_root_for_exe_path(&path))
    })
}

fn taskboard_root_from_env() -> Option<PathBuf> {
    let root = std::env::var_os("CODEX_TASKBOARD_ROOT").map(PathBuf::from)?;
    taskboard_runtime_exists(&root).then_some(root)
}

pub fn taskboard_root_for_exe_path(exe_path: &Path) -> Option<PathBuf> {
    let start = if exe_path.is_dir() {
        exe_path
    } else {
        exe_path.parent()?
    };
    for directory in start.ancestors() {
        let packaged_root = directory.join("codex-taskboard");
        if taskboard_runtime_exists(&packaged_root) {
            return Some(packaged_root);
        }
        let dev_root = directory.join("apps").join("codex-taskboard");
        if taskboard_runtime_exists(&dev_root) {
            return Some(dev_root);
        }
    }
    None
}

fn taskboard_runtime_exists(root: &Path) -> bool {
    root.join("scripts").join("codex-injector.mjs").is_file()
        && (root.join("dist").join("web").join("index.html").is_file()
            || root.join("server").join("index.mjs").is_file())
}

pub fn taskboard_node_executable() -> PathBuf {
    if let Some(path) = std::env::var_os("CODEX_TASKBOARD_NODE_EXE").map(PathBuf::from) {
        if path.is_file() {
            return path;
        }
    }
    if let Some(path) = bundled_codex_node_executable() {
        return path;
    }
    PathBuf::from(if cfg!(windows) { "node.exe" } else { "node" })
}

fn bundled_codex_node_executable() -> Option<PathBuf> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    let node = home
        .join(".cache")
        .join("codex-runtimes")
        .join("codex-primary-runtime")
        .join("dependencies")
        .join("node")
        .join("bin")
        .join(if cfg!(windows) { "node.exe" } else { "node" });
    node.is_file().then_some(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    static TASKBOARD_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn touch_runtime(root: &Path) {
        let injector = root.join("scripts").join("codex-injector.mjs");
        let server = root.join("server").join("index.mjs");
        std::fs::create_dir_all(injector.parent().unwrap()).unwrap();
        std::fs::create_dir_all(server.parent().unwrap()).unwrap();
        std::fs::write(injector, "").unwrap();
        std::fs::write(server, "").unwrap();
    }

    #[test]
    fn taskboard_root_resolution_finds_packaged_sibling() {
        let test_dir = tempfile::tempdir().unwrap();
        let root = test_dir.path().join("codex-taskboard");
        touch_runtime(&root);
        let exe = test_dir.path().join(if cfg!(windows) {
            "codex-plus-plus.exe"
        } else {
            "codex-plus-plus"
        });

        assert_eq!(taskboard_root_for_exe_path(&exe), Some(root));
    }

    #[test]
    fn taskboard_root_resolution_finds_dev_tree_from_debug_exe() {
        let test_dir = tempfile::tempdir().unwrap();
        let root = test_dir.path().join("apps").join("codex-taskboard");
        touch_runtime(&root);
        let exe = test_dir
            .path()
            .join("target")
            .join("debug")
            .join(if cfg!(windows) {
                "codex-plus-plus.exe"
            } else {
                "codex-plus-plus"
            });

        assert_eq!(taskboard_root_for_exe_path(&exe), Some(root));
    }

    #[test]
    fn service_command_uses_runtime_server_when_root_exists() {
        let test_dir = tempfile::tempdir().unwrap();
        let root = test_dir.path().join("codex-taskboard");
        touch_runtime(&root);

        let command = taskboard_service_command(Some(&root));
        let server = root.join("server").join("index.mjs");

        assert_eq!(command.get_current_dir(), Some(root.as_path()));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![server.as_os_str()]
        );
    }

    #[test]
    fn manager_startup_command_defaults_taskboard_host_to_loopback_when_sharing_is_disabled() {
        let _guard = taskboard_env_lock();
        let previous_host = std::env::var(TASKBOARD_HOST_ENV).ok();
        let previous_secret = std::env::var(TASKBOARD_SHARED_SECRET_ENV).ok();
        clear_taskboard_env();

        let test_dir = tempfile::tempdir().unwrap();
        let root = test_dir.path().join("codex-taskboard");
        touch_runtime(&root);
        let command = taskboard_service_command(Some(&root));
        let envs = command.get_envs().collect::<Vec<_>>();

        assert!(envs.iter().any(|(name, value)| {
            *name == std::ffi::OsStr::new(TASKBOARD_HOST_ENV)
                && *value == Some(std::ffi::OsStr::new(LOOPBACK_TASKBOARD_HOST))
        }));

        unsafe {
            std::env::set_var(TASKBOARD_HOST_ENV, LAN_TASKBOARD_HOST);
            std::env::set_var(TASKBOARD_SHARED_SECRET_ENV, "test-secret");
        }
        let lan_command = taskboard_service_command(Some(&root));
        assert!(lan_command.get_envs().any(|(name, value)| {
            name == std::ffi::OsStr::new(TASKBOARD_HOST_ENV)
                && value == Some(std::ffi::OsStr::new(LAN_TASKBOARD_HOST))
        }));

        unsafe {
            match previous_host {
                Some(value) => std::env::set_var(TASKBOARD_HOST_ENV, value),
                None => {
                    let _ = std::env::remove_var(TASKBOARD_HOST_ENV);
                }
            }
            match previous_secret {
                Some(value) => std::env::set_var(TASKBOARD_SHARED_SECRET_ENV, value),
                None => {
                    let _ = std::env::remove_var(TASKBOARD_SHARED_SECRET_ENV);
                }
            }
        }
    }

    #[test]
    fn health_requires_verified_bind_host() {
        let unverified_response =
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"status\":\"ok\"}";
        let loopback_response =
            "HTTP/1.1 200 OK\r\nx-taskboard-bind-host: 127.0.0.1\r\n\r\n{\"status\":\"ok\"}";
        let lan_response =
            "HTTP/1.1 200 OK\r\nx-taskboard-bind-host: 0.0.0.0\r\n\r\n{\"status\":\"ok\"}";

        assert!(!taskboard_health_response_matches(
            unverified_response,
            LOOPBACK_TASKBOARD_HOST
        ));
        assert!(taskboard_health_response_matches(
            loopback_response,
            LOOPBACK_TASKBOARD_HOST
        ));
        assert!(!taskboard_health_response_matches(
            unverified_response,
            LAN_TASKBOARD_HOST
        ));
        assert!(!taskboard_health_response_matches(
            lan_response,
            LOOPBACK_TASKBOARD_HOST
        ));
        assert!(taskboard_health_response_matches(
            lan_response,
            LAN_TASKBOARD_HOST
        ));
    }

    #[test]
    fn page_entry_must_serve_taskboard_html() {
        let taskboard_html = concat!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\n\r\n",
            "<!doctype html><html><body><div id=\"root\"></div></body></html>"
        );
        let stale_health_only_service = concat!(
            "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\n\r\n",
            "{\"error\":{\"code\":\"NOT_FOUND\"}}"
        );
        let wrong_content_type = concat!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n",
            "{\"status\":\"ok\"}"
        );

        assert!(taskboard_page_entry_response_matches(taskboard_html));
        assert!(!taskboard_page_entry_response_matches(
            stale_health_only_service
        ));
        assert!(!taskboard_page_entry_response_matches(wrong_content_type));
    }

    #[test]
    fn lan_health_request_carries_shared_secret_authentication() {
        let request = taskboard_health_request(Some("test-secret"));
        assert!(request.contains("Authorization: Basic Y29kZXg6dGVzdC1zZWNyZXQ="));
        assert!(!taskboard_health_request(None).contains("Authorization:"));
    }

    #[test]
    fn lan_page_entry_request_carries_shared_secret_authentication() {
        let request = taskboard_page_entry_request(Some("test-secret"));
        assert!(request.starts_with("GET /?host=codex HTTP/1.1"));
        assert!(request.contains("Authorization: Basic Y29kZXg6dGVzdC1zZWNyZXQ="));
        assert!(!taskboard_page_entry_request(None).contains("Authorization:"));
    }

    fn taskboard_env_lock() -> std::sync::MutexGuard<'static, ()> {
        TASKBOARD_ENV_LOCK
            .lock()
            .expect("taskboard env lock should not be poisoned")
    }

    fn clear_taskboard_env() {
        unsafe {
            let _ = std::env::remove_var(TASKBOARD_HOST_ENV);
            let _ = std::env::remove_var(TASKBOARD_SHARED_SECRET_ENV);
        }
    }
}
