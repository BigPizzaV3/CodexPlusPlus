//! 在隔离的 Codex 上单独跑远程控制服务，供 tools/remote-control-p0/run-p1.mjs 端到端验证。
//!
//! 环境变量：
//!   P1_HOME      隔离的 CODEX_HOME（必填，不能是 ~/.codex）
//!   P1_APP       Codex 的 .app 目录（必填，用于找 CLI 与 IPC 版本表）
//!   P1_CDP_PORT  隔离桌面的调试端口（默认 9333）
//!   P1_PORT      服务端口（默认 57399）
//!   P1_TOKEN     访问令牌（必填）
//!
//! cargo run -p codex-plus-core --example remote_control_serve

use std::path::PathBuf;
use std::sync::Arc;

use codex_plus_core::remote_control::server::{ServerContext, serve};
use codex_plus_core::remote_control::service::{RemoteService, ServiceConfig};

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
    let service = RemoteService::new(ServiceConfig { codex_home: home, app_dir: Some(app), debug_port: Some(debug_port) });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("remote control listening on http://127.0.0.1:{port}/#token={token}");
    serve(listener, Arc::new(ServerContext { service, token, port })).await;
    Ok(())
}
