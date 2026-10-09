//! 向已启动的隔离 Codex 页面注入真实桥接；不启动/重启 App、不切换账号。
use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use codex_plus_core::{
    bridge,
    launcher::{DefaultLaunchHooks, LaunchHooks},
    models::{DeleteResult, ExportResult, SessionRef},
    routes::{BridgeContext, BridgeDataService, BridgeSettingsService, CoreRuntimeService},
    settings::BackendSettings,
    status::StatusStore,
};
use serde_json::Value;

struct SmokeSettings(BackendSettings);
struct SmokeData;

#[async_trait]
impl BridgeDataService for SmokeData {
    async fn delete(&self, _session: SessionRef) -> Result<DeleteResult> {
        anyhow::bail!("插件测试不操作会话")
    }
    async fn undo(&self, _token: String) -> Result<DeleteResult> {
        anyhow::bail!("插件测试不操作会话")
    }
    async fn export_markdown(&self, _session: SessionRef) -> Result<ExportResult> {
        anyhow::bail!("插件测试不操作会话")
    }
    async fn thread_usage_history(&self, _session: SessionRef) -> Result<Value> {
        anyhow::bail!("插件测试不操作会话")
    }
    async fn find_archived_thread_by_title(&self, _title: String) -> Result<Option<SessionRef>> {
        anyhow::bail!("插件测试不操作会话")
    }
}

#[async_trait]
impl BridgeSettingsService for SmokeSettings {
    async fn get_settings(&self) -> Result<BackendSettings> {
        Ok(self.0.clone())
    }
    async fn set_settings(&self, _payload: Value) -> Result<BackendSettings> {
        anyhow::bail!("隔离测试不修改管理工具设置")
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(
        args.len() == 3,
        "plugin_market_smoke <debug-port> <helper-port> <stop-file>"
    );
    let debug_port: u16 = args[0].parse()?;
    let helper_port: u16 = args[1].parse()?;
    let stop_file = PathBuf::from(&args[2]);
    ensure!(
        stop_file.is_absolute() && !stop_file.exists(),
        "stop file must be new and absolute"
    );
    let home = std::env::var_os("CODEX_HOME").context("需要隔离 CODEX_HOME")?;
    let home_path = PathBuf::from(home).canonicalize()?;
    let temporary_root = std::env::temp_dir().canonicalize()?;
    let in_unix_tmp = PathBuf::from("/tmp")
        .canonicalize()
        .is_ok_and(|root| home_path.starts_with(root));
    ensure!(
        home_path.starts_with(temporary_root) || in_unix_tmp,
        "CODEX_HOME 必须在临时目录内"
    );
    let targets = codex_plus_core::cdp::list_targets(debug_port).await?;
    let target = codex_plus_core::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .context("没有页面调试连接")?;
    let mut settings = BackendSettings::default();
    settings.relay_profiles_enabled = false;
    settings.codex_app_model_whitelist_unlock = false;
    settings.codex_app_plugin_marketplace_unlock = true;
    let context = BridgeContext::new(
        Arc::new(SmokeSettings(settings.clone())),
        Arc::new(CoreRuntimeService::new(debug_port, StatusStore::default())),
        Arc::new(SmokeData),
    );
    let hooks = DefaultLaunchHooks::default();
    hooks.start_helper(helper_port).await?;
    bridge::install_bridge(
        websocket,
        bridge::BRIDGE_BINDING_NAME,
        Arc::new(move |route, payload| {
            let context = context.clone();
            Box::pin(async move {
                Ok(codex_plus_core::routes::handle_bridge_request(context, &route, payload).await)
            })
        }),
        &[codex_plus_core::assets::injection_script_with_settings(
            helper_port,
            &settings,
        )],
    )
    .await?;
    println!("PLUGIN_MARKET_SMOKE_READY debug={debug_port} helper={helper_port}");
    for _ in 0..900 {
        if stop_file.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    hooks.shutdown_helper(helper_port).await;
    Ok(())
}
