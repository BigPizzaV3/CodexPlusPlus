//! Codex 里的「微信连接」页（`45-weixin-connect-page.js`）经 bridge 路由 `/weixin-connect` 调到这里。
//!
//! 扫码会话、运行中的连接都在本进程（管理器）内存里，所以登记成宿主路由（见 `lib.rs` 的
//! `register_codex_bridge_routes`）；每个动作直接调管理器微信页用的同一组 Tauri 命令函数，
//! 不另写一份逻辑：
//!
//! - `state`：连接状态 + 页面能看到的设置（不含登录凭据）+ 远程控制开没开（加加要用）
//! - `save {settings}`：只合并白名单字段（core `connect::page`），再走管理器的 `save_settings`
//!   （同一套校验、热更新运行中的连接）
//! - `qrStart` / `qrStatus`：扫码登录，网关地址与路由标记取已保存的设置
//! - `start` / `stop`：启动 / 停止连接
//! - `catalog`：模型目录（模型与推理强度下拉用）
//!
//! 不在拓展路由白名单里。改了设置后通知管理器界面重新读取，两边看到的一致。

use codex_plus_core::connect::page;
use codex_plus_core::settings::SettingsStore;
use serde_json::{Value, json};

use crate::commands;

pub async fn bridge_request(payload: Value) -> anyhow::Result<Value> {
    let action = payload.get("action").and_then(Value::as_str).unwrap_or("state");
    match action {
        "state" => state(),
        "save" => {
            let patch = payload.get("settings").cloned().unwrap_or(Value::Null);
            let saved = tokio::task::spawn_blocking(move || save(&patch)).await??;
            codex_plus_core::settings::notify_external_settings_change();
            Ok(saved)
        }
        "qrStart" => {
            let settings = SettingsStore::default().load()?;
            let result = commands::weixin_connect_qr_start(
                settings.weixin_connect_base_url,
                settings.weixin_connect_route_tag,
            )
            .await;
            Ok(serde_json::to_value(result)?)
        }
        "qrStatus" => {
            let result = commands::weixin_connect_qr_status().await;
            // 扫码确认后凭据已写进设置：通知管理器界面刷新。
            if result.payload.qr_status == "confirmed" {
                codex_plus_core::settings::notify_external_settings_change();
            }
            Ok(serde_json::to_value(result)?)
        }
        "start" | "stop" => {
            let result = if action == "start" {
                commands::weixin_connect_start().await
            } else {
                commands::weixin_connect_stop().await
            };
            // 启停会改「开机自动连接」开关。
            codex_plus_core::settings::notify_external_settings_change();
            let mut value = state()?;
            value["status"] = json!(result.status);
            value["message"] = json!(result.message);
            Ok(value)
        }
        "catalog" => Ok(json!({
            "status": "ok",
            "catalog": commands::weixin_connect_catalog().await,
        })),
        other => anyhow::bail!("未知操作：{other}"),
    }
}

/// 页面需要的全部状态。远程控制没由本进程托管（理论上不会，宿主路由只在管理器里登记）时按没开算。
fn state() -> anyhow::Result<Value> {
    let settings = SettingsStore::default().load()?;
    let connection = commands::weixin_connect_status().payload;
    let remote_running = codex_plus_core::remote_control::status()
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(json!({
        "status": "ok",
        "connection": connection,
        "settings": page::page_settings(&settings),
        "remoteControlRunning": remote_running,
    }))
}

fn save(patch: &Value) -> anyhow::Result<Value> {
    let mut settings = SettingsStore::default().load()?;
    page::apply_page_settings(&mut settings, patch)?;
    let result = commands::save_settings(settings, None);
    if result.status != "ok" {
        anyhow::bail!("{}", result.message);
    }
    let mut value = state()?;
    value["message"] = json!("已保存，下一条微信消息生效。");
    Ok(value)
}
