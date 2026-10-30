//! 管理器「社区拓展」与「精选」拓展包入口：GitHub 上打了 `codexpp-extension` topic 的拓展包，列表、安装与开关；
//! 精选清单里锁定提交的拓展包也走同一套安装。安装分 prepare（下载校验、返回实际版本的权限）和
//! install（用户确认后换进拓展目录）两步。社区栏按精选清单过滤：下架的、已进精选的仓库不显示。

use anyhow::Context;
use codex_plus_core::extension_discovery::{CommunityStore, CuratedFilter, Endpoints};
use codex_plus_core::script_market;
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
            Ok(store.list_filtered(&endpoints, &manager, force, &curated_filter().await).await)
        }
        "prepare" => {
            let repo = text(&payload, "repo")?;
            if curated_filter().await.is_blocked(repo) {
                anyhow::bail!("{repo} 已被 Codex++ 下架，不能安装");
            }
            store.prepare(&endpoints, &manager, repo).await
        }
        // 精选 / 官方拓展包：只收 id，仓库与提交从清单里查，不信任前端传来的 sha。
        "preparePackage" => {
            let id = text(&payload, "id")?;
            let manifest = script_market::cached_market_manifest(false).await?;
            let package = manifest
                .packages
                .iter()
                .find(|package| package.id == id)
                .with_context(|| format!("精选清单里没有拓展包 {id}"))?;
            store.prepare_pinned(&endpoints, &manager, &package.pinned()).await
        }
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
            Ok(store.list_filtered(&endpoints, &manager, false, &curated_filter().await).await)
        }
        other => anyhow::bail!("未知操作 {other}"),
    }
}

/// 精选清单拉不到时不过滤（社区栏照常可用，只是少了下架与去重）。
async fn curated_filter() -> CuratedFilter {
    match script_market::cached_market_manifest(false).await {
        Ok(manifest) => script_market::curated_filter(&manifest),
        Err(_) => CuratedFilter {
            official_owners: script_market::OFFICIAL_OWNERS.iter().map(|owner| owner.to_string()).collect(),
            ..CuratedFilter::default()
        },
    }
}

fn text<'a>(payload: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("缺少参数 {key}"))
}
