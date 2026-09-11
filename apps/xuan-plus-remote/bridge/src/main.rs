use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, bail};
use tokio::net::TcpListener;
use xuan_plus_remote_bridge::{MobileRemote, RemoteBridgeState, router};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("install-user-script") {
        let destination = std::env::args().nth(2).map(std::path::PathBuf::from);
        let installed =
            xuan_plus_remote_bridge::installer::install_user_script(destination.as_deref())?;
        println!(
            "{}",
            serde_json::json!({"status": "ok", "userScriptPath": installed})
        );
        return Ok(());
    }
    let address = listen_address()?;
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("无法监听手机桥接地址 {address}"))?;
    let address = listener.local_addr()?;
    let remote = Arc::new(MobileRemote::default());
    remote.restore().await;
    let state = RemoteBridgeState::new(remote, expected_token());

    println!(
        "{}",
        serde_json::json!({
            "status": "ready",
            "bridgeVersion": env!("CARGO_PKG_VERSION"),
            "url": format!("http://{address}"),
            "authentication": if state.requires_authentication() { "bearer" } else { "loopback" },
        })
    );
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("手机桥接服务异常退出")
}

fn listen_address() -> anyhow::Result<SocketAddr> {
    let raw =
        std::env::var("XUAN_MOBILE_BRIDGE_BIND").unwrap_or_else(|_| "127.0.0.1:17421".to_owned());
    let address: SocketAddr = raw
        .parse()
        .context("XUAN_MOBILE_BRIDGE_BIND 不是有效地址")?;
    if !address.ip().is_loopback() {
        bail!("手机桌面桥仅允许监听 loopback 地址");
    }
    Ok(address)
}

fn expected_token() -> Option<String> {
    std::env::var("XUAN_MOBILE_BRIDGE_TOKEN")
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn default_address_is_loopback() {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 17421);
        assert!(address.ip().is_loopback());
    }
}
