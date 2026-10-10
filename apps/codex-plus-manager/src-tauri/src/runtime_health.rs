use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::Duration;

use codex_plus_core::settings::BackendSettings;
use codex_plus_core::status::LaunchStatus;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ServiceState {
    pub status: &'static str,
    pub address: Option<String>,
    pub message: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeHealth {
    pub codex_app: ServiceState,
    pub local_server: ServiceState,
    pub debugger: ServiceState,
    pub protocol_conversion_enabled: Option<bool>,
}

impl RuntimeHealth {
    pub fn unavailable() -> Self {
        let state = ServiceState {
            status: "not_checked",
            address: None,
            message: "等待状态检查",
        };
        Self {
            codex_app: state.clone(),
            local_server: state.clone(),
            debugger: state,
            protocol_conversion_enabled: None,
        }
    }
}

/// 仅在读取概览时检查本地端点，不启动服务、不请求供应商，也不创建后台监控线程。
pub fn inspect(settings: &BackendSettings, latest: Option<&LaunchStatus>) -> RuntimeHealth {
    inspect_with(
        settings,
        latest,
        !codex_plus_core::watcher::find_codex_processes().is_empty(),
        helper_available,
        codex_plus_core::cdp::endpoint_available,
    )
}

fn inspect_with(
    settings: &BackendSettings,
    latest: Option<&LaunchStatus>,
    codex_running: bool,
    helper_check: impl Fn(u16) -> bool,
    debugger_check: impl Fn(u16) -> bool,
) -> RuntimeHealth {
    let proxy_port = codex_plus_core::launcher::required_fixed_helper_port(settings);
    let server_port = proxy_port
        .or_else(|| latest.and_then(|s| s.helper_port))
        .unwrap_or(57321);
    let debug_port = latest.and_then(|s| s.debug_port).unwrap_or(9229);
    let server_required = settings.enhancements_enabled || proxy_port.is_some();
    let server_ok = server_required && helper_check(server_port);
    let debugger_ok = debugger_check(debug_port);
    RuntimeHealth {
        codex_app: ServiceState {
            status: if codex_running { "running" } else { "stopped" },
            address: None,
            message: if codex_running {
                "Codex APP 正在运行"
            } else {
                "Codex APP 未运行"
            },
        },
        local_server: ServiceState {
            status: if !server_required {
                "disabled"
            } else if server_ok {
                "running"
            } else {
                "stopped"
            },
            address: Some(format!("http://127.0.0.1:{server_port}")),
            message: if !server_required {
                "当前配置无需本地服务器"
            } else if server_ok {
                "本地服务器已连接"
            } else {
                "本地服务器未运行"
            },
        },
        debugger: ServiceState {
            status: if debugger_ok {
                "connected"
            } else {
                "disconnected"
            },
            address: Some(format!("http://127.0.0.1:{debug_port}")),
            message: if debugger_ok {
                "Codex 调试端点已连接"
            } else {
                "Codex 调试端点未连接"
            },
        },
        protocol_conversion_enabled: Some(proxy_port.is_some()),
    }
}

fn helper_available(port: u16) -> bool {
    [
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
    ]
    .into_iter()
    .any(|address| helper_available_at(address, port))
}

fn helper_available_at(address: SocketAddr, port: u16) -> bool {
    // 启动恢复期间 CDP 检查可能短暂占用 session 线程，不能把正常排队误报为停止。
    let timeout = Duration::from_secs(1);
    let Ok(mut stream) = TcpStream::connect_timeout(&address, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    if write!(
        stream,
        "GET /backend/status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .is_err()
    {
        return false;
    }
    let mut response = Vec::new();
    if stream.take(32 * 1024).read_to_end(&mut response).is_err() {
        return false;
    }
    let Some(end) = response.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&response[..end]);
    if !headers
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("HTTP/") && line.contains(" 200 "))
    {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(&response[end + 4..])
        .ok()
        .is_some_and(|body| {
            body["status"] == "ok"
                && body["transport"] == "http-helper"
                && body["version"].is_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_plus_core::settings::{RelayMode, RelayProfile, RelayProtocol};

    fn read_helper_request(stream: &mut TcpStream) {
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() < 1024);
        }
        assert!(String::from_utf8_lossy(&request).starts_with("GET /backend/status "));
    }

    #[test]
    fn persisted_running_status_does_not_make_unreachable_services_healthy() {
        let settings = BackendSettings {
            enhancements_enabled: true,
            ..BackendSettings::default()
        };
        let old = LaunchStatus {
            status: "running".into(),
            debug_port: Some(9333),
            helper_port: Some(57322),
            ..LaunchStatus::default()
        };
        let health = inspect_with(&settings, Some(&old), false, |_| false, |_| false);
        assert_eq!(health.local_server.status, "stopped");
        assert_eq!(health.debugger.status, "disconnected");
        assert_eq!(
            health.local_server.address.as_deref(),
            Some("http://127.0.0.1:57322")
        );
    }

    #[test]
    fn protocol_conversion_checks_its_fixed_port_instead_of_an_old_helper_port() {
        let profile = RelayProfile {
            id: "chat".into(),
            relay_mode: RelayMode::PureApi,
            protocol: RelayProtocol::ChatCompletions,
            ..RelayProfile::default()
        };
        let settings = BackendSettings {
            active_relay_id: "chat".into(),
            relay_profiles: vec![profile],
            ..BackendSettings::default()
        };
        let fixed = codex_plus_core::protocol_proxy::protocol_proxy_port();
        let other = if fixed == 57322 { 57323 } else { 57322 };
        let latest = LaunchStatus {
            helper_port: Some(other),
            ..LaunchStatus::default()
        };
        let probed_ports = std::cell::RefCell::new(Vec::new());
        let health = inspect_with(
            &settings,
            Some(&latest),
            true,
            |port| {
                probed_ports.borrow_mut().push(port);
                port == other
            },
            |_| true,
        );
        assert_eq!(health.local_server.status, "stopped");
        assert_eq!(health.debugger.status, "connected");
        assert_eq!(health.protocol_conversion_enabled, Some(true));
        assert_eq!(*probed_ports.borrow(), vec![fixed]);
        assert!(
            health
                .local_server
                .address
                .unwrap()
                .contains(&fixed.to_string())
        );
    }

    #[test]
    fn protocol_conversion_flag_is_independent_from_server_availability() {
        let profile = RelayProfile {
            id: "direct".into(),
            relay_mode: RelayMode::PureApi,
            ..RelayProfile::default()
        };
        let settings = BackendSettings {
            active_relay_id: "direct".into(),
            relay_profiles: vec![profile],
            enhancements_enabled: false,
            ..BackendSettings::default()
        };
        let server_enabled = BackendSettings {
            enhancements_enabled: true,
            ..settings.clone()
        };
        let running = inspect_with(&server_enabled, None, true, |_| true, |_| false);
        assert_eq!(running.local_server.status, "running");
        assert_eq!(running.protocol_conversion_enabled, Some(false));
        let stopped = inspect_with(&server_enabled, None, true, |_| false, |_| false);
        assert_eq!(stopped.local_server.status, "stopped");
        assert_eq!(stopped.protocol_conversion_enabled, Some(false));
        let health = inspect_with(
            &settings,
            None,
            false,
            |_| panic!("disabled helper should not be checked"),
            |_| false,
        );
        assert_eq!(health.local_server.status, "disabled");
        assert_eq!(health.protocol_conversion_enabled, Some(false));
        assert_eq!(
            RuntimeHealth::unavailable().protocol_conversion_enabled,
            None
        );
    }

    #[test]
    fn app_running_is_independent_from_the_debugger_connection() {
        let settings = BackendSettings::default();
        let running = inspect_with(&settings, None, true, |_| false, |_| false);
        assert_eq!(running.codex_app.status, "running");
        assert_eq!(running.debugger.status, "disconnected");
        let stopped = inspect_with(&settings, None, false, |_| false, |_| false);
        assert_eq!(stopped.codex_app.status, "stopped");
    }

    #[test]
    fn helper_identity_is_verified_over_http() {
        for (body, expected) in [
            (
                r#"{"status":"ok","transport":"http-helper","version":"1.7.2"}"#,
                true,
            ),
            (
                r#"{"status":"ok","transport":"other","version":"1.7.2"}"#,
                false,
            ),
        ] {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                read_helper_request(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            });
            assert_eq!(helper_available(port), expected);
            server.join().unwrap();
        }
    }

    #[test]
    fn helper_health_tolerates_a_brief_startup_delay() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            read_helper_request(&mut stream);
            std::thread::sleep(Duration::from_millis(350));
            let body = r#"{"status":"ok","transport":"http-helper","version":"1.7.2"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        assert!(helper_available(port));
        server.join().unwrap();
    }
}
