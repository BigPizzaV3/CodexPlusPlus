//! 在隔离的 Codex 上单独跑远程控制服务，供 tools/remote-control-p0/run-p1.mjs / run-p2.mjs 端到端验证。
//!
//! 环境变量：
//!   P1_HOME      隔离的 CODEX_HOME（必填，不能是 ~/.codex）
//!   P1_APP       Codex 的 .app 目录（必填，用于找 CLI 与 IPC 版本表）
//!   P1_CDP_PORT  隔离桌面的调试端口（默认 9333）
//!   P1_PORT      本机入口端口（默认 57399）
//!   P1_TOKEN     主令牌（必填）
//!   P1_LAN_PORT  可选：同时开局域网 HTTPS 入口（自签证书）
//!   P1_STATE     开局域网入口时必填：隔离的状态文件路径（设备表、证书都放在它旁边）
//!   P1_PAIR_ROLES 开局域网入口时可选：依次要发的配对码权限，如 `view,full`（见 harness_control）
//!
//! cargo run -p codex-plus-core --example remote_control_serve

use std::path::PathBuf;
use std::sync::Arc;

use codex_plus_core::remote_control::server::{Scope, ServerContext, serve, serve_tls};
use codex_plus_core::remote_control::service::{RemoteService, ServiceConfig};
use codex_plus_core::remote_control::{set_state_path_for_tests, tls};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let home = PathBuf::from(std::env::var("P1_HOME")?);
    let default_home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".codex"));
    if Some(&home) == default_home.as_ref() {
        anyhow::bail!("P1_HOME 不能是 ~/.codex：只在隔离实例上跑");
    }
    let app = PathBuf::from(std::env::var("P1_APP")?);
    let debug_port = std::env::var("P1_CDP_PORT").ok().and_then(|port| port.parse().ok()).unwrap_or(9333);
    let port: u16 = std::env::var("P1_PORT").ok().and_then(|port| port.parse().ok()).unwrap_or(57399);
    let token = std::env::var("P1_TOKEN")?;
    let lan_port: Option<u16> = std::env::var("P1_LAN_PORT").ok().and_then(|port| port.parse().ok());
    if lan_port.is_some() {
        set_state_path_for_tests(Some(PathBuf::from(std::env::var("P1_STATE")?)));
    }
    let service = RemoteService::new(ServiceConfig { codex_home: home, app_dir: Some(app), debug_port: Some(debug_port) });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("remote control listening on http://127.0.0.1:{port}/#token={token}");
    let local = serve(listener, Arc::new(ServerContext { service: service.clone(), token, port, scope: Scope::Loopback }));
    let Some(lan_port) = lan_port else {
        local.await;
        return Ok(());
    };
    let certificate = tls::ensure_certificate_at(&tls::tls_path(), &tls::local_addresses())?;
    println!("lan https://0.0.0.0:{lan_port}/  fingerprint {}", certificate.fingerprint());
    let acceptor = tokio_rustls::TlsAcceptor::from(tls::server_config(&certificate)?);
    let lan_listener = tokio::net::TcpListener::bind(("0.0.0.0", lan_port)).await?;
    let lan = serve_tls(lan_listener, acceptor, Arc::new(ServerContext { service, token: String::new(), port: lan_port, scope: Scope::Lan }));
    tokio::join!(local, lan, harness_control(PathBuf::from(std::env::var("P1_STATE")?)));
    Ok(())
}

/// 端到端脚本代替管理器做的事：按 P1_PAIR_ROLES（逗号分隔）依次发配对码，写到状态目录的 pairing-code.txt；
/// 脚本写 revoke.txt（设备 ID）就吊销那台设备。只在 example 里，产品里这些动作只能从管理器发起。
async fn harness_control(state: PathBuf) {
    use codex_plus_core::remote_control::devices::{self, Role};
    let dir = state.parent().map(PathBuf::from).unwrap_or_default();
    let mut roles: Vec<Role> = std::env::var("P1_PAIR_ROLES")
        .unwrap_or_default()
        .split(',')
        .filter_map(|role| Role::parse(role.trim()).ok())
        .rev()
        .collect();
    loop {
        if devices::current_pairing().is_none() {
            match roles.pop() {
                Some(role) => {
                    let pairing = devices::start_pairing(role);
                    let _ = std::fs::write(dir.join("pairing-code.txt"), format!("{} {}", pairing.code, role.as_str()));
                }
                None => {
                    let _ = std::fs::remove_file(dir.join("pairing-code.txt"));
                }
            }
        }
        if let Ok(id) = std::fs::read_to_string(dir.join("revoke.txt")) {
            let _ = std::fs::remove_file(dir.join("revoke.txt"));
            let _ = devices::revoke(id.trim());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}
