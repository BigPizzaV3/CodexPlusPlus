//! 管理器「本地拓展 → 发布到 GitHub」：发布前检查、版本号 +1、发布；
//! 「我发布的」：列出、下架、重新上架、删除仓库。鉴权全部交给本机 gh。

use anyhow::Context;
use codex_plus_core::extension_discovery::CommunityStore;
use codex_plus_core::extension_publish::{self, PublishOptions, Publisher};
use codex_plus_core::user_scripts::default_user_scripts_config_dir;
use serde_json::{Value, json};
use tauri::Emitter as _;

/// 发布进度事件：每一步开始、结束各发一次，载荷是 `StepEvent`。
const PUBLISH_PROGRESS_EVENT: &str = "extension-publish-progress";

#[tauri::command]
pub async fn extension_publish_request(app: tauri::AppHandle, action: String, payload: Option<Value>) -> Result<Value, String> {
    let payload = payload.unwrap_or(Value::Null);
    // gh / git 都是阻塞的外部命令，放到阻塞线程里跑。
    tauri::async_runtime::spawn_blocking(move || handle(&app, &action, &payload))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))
}

/// Codex 拓展页「我发布的」分栏经桥接调用（`/extension-market/published`，payload 为 `{ action, payload }`）。
/// 只开放「我发布的」那几个按仓库的操作；发布本身要走管理器的检查与进度界面。
pub(crate) async fn bridge_request(request: Value) -> anyhow::Result<Value> {
    let action = request.get("action").and_then(Value::as_str).unwrap_or_default().to_string();
    let payload = request.get("payload").cloned().unwrap_or(Value::Null);
    tauri::async_runtime::spawn_blocking(move || {
        published(&action, &payload)?.with_context(|| format!("Codex 页面不支持操作 {action}"))
    })
    .await?
}

/// 「我发布的」：按仓库操作，不需要拓展 id。只能动当前 gh 账号名下的仓库（core 里校验）。
/// 不是这几个操作时返回 `None`。
fn published(action: &str, payload: &Value) -> anyhow::Result<Option<Value>> {
    let config_dir = default_user_scripts_config_dir();
    match action {
        "mine" => Publisher::new(&config_dir)?.list_published().map(Some),
        "unlist" | "relist" | "deleteRepo" => {
            let repo = payload.get("repo").and_then(Value::as_str).filter(|repo| !repo.is_empty()).context("缺少仓库")?;
            let publisher = Publisher::new(&config_dir)?;
            match action {
                "unlist" => publisher.unlist(repo)?,
                "relist" => publisher.relist(repo)?,
                _ => publisher.delete_repo(repo)?,
            }
            // 下架 / 删除后社区栏立刻不再显示，不等 10 分钟的列表缓存。
            if action != "relist" {
                let _ = CommunityStore::new(&config_dir).forget_repo(repo);
            }
            Ok(Some(json!({ "ok": true })))
        }
        _ => Ok(None),
    }
}

fn handle(app: &tauri::AppHandle, action: &str, payload: &Value) -> anyhow::Result<Value> {
    if let Some(result) = published(action, payload)? {
        return Ok(result);
    }
    let manager = crate::commands::default_user_script_manager();
    let config_dir = default_user_scripts_config_dir();
    let id = payload.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).context("缺少拓展 id")?;
    match action {
        "check" => check(&manager, &config_dir, id),
        "bump" => {
            let part = payload.get("part").and_then(Value::as_str).unwrap_or("patch");
            extension_publish::bump_version(&manager, id, part)?;
            check(&manager, &config_dir, id)
        }
        "publish" => {
            let publisher = Publisher::new(&config_dir)?;
            let options = PublishOptions {
                repo_name: payload.get("repoName").and_then(Value::as_str).unwrap_or("").to_string(),
                license: payload.get("license").and_then(Value::as_str).filter(|license| !license.is_empty() && *license != "none").map(str::to_string),
                generate_readme: payload.get("generateReadme").and_then(Value::as_bool).unwrap_or(true),
            };
            Ok(publisher.publish_with_progress(&manager, id, &options, &|event| {
                let _ = app.emit(PUBLISH_PROGRESS_EVENT, event);
            }))
        }
        other => anyhow::bail!("未知操作 {other}"),
    }
}

/// 检查结果加上 gh 状态：没装或没登录时给出指引，界面据此禁用发布按钮。
fn check(manager: &codex_plus_core::user_scripts::UserScriptManager, config_dir: &std::path::Path, id: &str) -> anyhow::Result<Value> {
    let report = extension_publish::check(manager, config_dir, id)?;
    let gh = match Publisher::new(config_dir).and_then(|publisher| publisher.account()) {
        Ok(account) => json!({ "ready": true, "account": account }),
        Err(error) => json!({ "ready": false, "message": format!("{error:#}") }),
    };
    let mut value = serde_json::to_value(report)?;
    value["gh"] = gh;
    Ok(value)
}
