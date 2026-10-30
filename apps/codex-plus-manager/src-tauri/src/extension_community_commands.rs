//! 管理器「社区拓展」入口：GitHub 上打了 `codexpp-extension` topic 的拓展包，列表、安装与开关。
//! 安装分 prepare（下载校验、返回实际版本的权限）和 install（用户确认后换进拓展目录）两步。

use anyhow::Context;
use codex_plus_core::extension_discovery::{CommunityStore, Endpoints};
use codex_plus_core::user_scripts::default_user_scripts_config_dir;
use serde_json::{Value, json};

#[tauri::command]
pub async fn extension_community_request(action: String, payload: Option<Value>) -> Result<Value, String> {
    handle(&action, payload.unwrap_or(Value::Null))
        .await
        .map_err(|error| format!("{error:#}"))
}

async fn handle(action: &str, payload: Value) -> anyhow::Result<Value> {
    let store = CommunityStore::new(default_user_scripts_config_dir());
    let manager = crate::commands::default_user_script_manager();
    let endpoints = Endpoints::github();
    match action {
        "list" => {
            let force = payload.get("force").and_then(Value::as_bool).unwrap_or(false);
            Ok(store.list(&endpoints, &manager, force).await)
        }
        "prepare" => store.prepare(&endpoints, &manager, text(&payload, "repo")?).await,
        "install" => {
            let token = text(&payload, "token")?.to_string();
            let enable = payload.get("enable").and_then(Value::as_bool).unwrap_or(false);
            let replace = payload.get("replace").and_then(Value::as_bool).unwrap_or(false);
            let mut result = tauri::async_runtime::spawn_blocking(move || {
                let manager = crate::commands::default_user_script_manager();
                store.install(&manager, &token, enable, replace)
            })
            .await??;
            // 文案由界面拼：这里只给出钩子同步的结果。
            if result["hooks"] == true {
                let hooks = codex_plus_core::extension_hooks::sync_default(&manager);
                result["hooksError"] = hooks.get("error").cloned().unwrap_or(Value::Null);
                result["hooksAdded"] = json!(hooks["added"].as_array().is_some_and(|added| !added.is_empty()));
            }
            Ok(result)
        }
        "discard" => {
            store.discard(text(&payload, "token")?);
            Ok(json!({}))
        }
        "setEnabled" => {
            let enabled = payload.get("enabled").and_then(Value::as_bool).context("缺少参数 enabled")?;
            store.set_enabled(enabled)?;
            Ok(store.list(&endpoints, &manager, false).await)
        }
        other => anyhow::bail!("未知操作 {other}"),
    }
}

fn text<'a>(payload: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("缺少参数 {key}"))
}
