//! ClaudeMom 的命令：管理多组上游（Base URL + Key）、从 cc-switch / Magpie / Codex++ 导入、拉模型列表。
//!
//! Claude 本身的状态、启动、停止、开关仍走 `claude_local_*` 三个命令（commands.rs）。
//! 每次改了上游都立刻交给 Claude 本地服务（`claude_mom::apply_active`），状态里的「需要重启」随之更新。

use std::sync::Mutex;

use codex_plus_core::claude_mom::{self, UpstreamBook, UpstreamModel, UpstreamProfile};
use serde::Serialize;

use crate::commands::{CommandResult, failed, ok};

/// 读-改-写上游列表时串行，避免两次保存互相覆盖。
static BOOK_LOCK: Mutex<()> = Mutex::new(());

/// payload 必须是结构体：`CommandResult` 会把它 flatten 到顶层，数组没法 flatten。
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookPayload {
    profiles: Vec<UpstreamProfile>,
    active_id: Option<String>,
}

impl From<UpstreamBook> for BookPayload {
    fn from(book: UpstreamBook) -> Self {
        Self { profiles: book.profiles, active_id: book.active_id }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ModelsPayload {
    models: Vec<UpstreamModel>,
}

fn load() -> anyhow::Result<UpstreamBook> {
    claude_mom::load_book(&claude_mom::default_book_path())
}

/// 改上游列表并存盘，然后把选中的那组交给 Claude 本地服务。
fn mutate<T>(change: impl FnOnce(&mut UpstreamBook) -> anyhow::Result<T>) -> anyhow::Result<(UpstreamBook, T)> {
    let _guard = BOOK_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut book = load()?;
    let value = change(&mut book)?;
    claude_mom::save_book(&claude_mom::default_book_path(), &book)?;
    claude_mom::apply_active(&book);
    Ok((book, value))
}

fn respond<T>(result: anyhow::Result<(UpstreamBook, T)>, message: impl FnOnce(T) -> String, failure: &str) -> CommandResult<BookPayload> {
    match result {
        Ok((book, value)) => ok(&message(value), book.into()),
        Err(error) => failed(&format!("{failure}：{error:#}"), load().unwrap_or_default().into()),
    }
}

/// 启动时调用：选中的上游交给 Claude 本地服务。读不到配置也切到直连（空上游），不回落到网关。
pub fn apply_saved_upstream() {
    let book = load().unwrap_or_else(|error| {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "claude_mom.load_upstreams_failed",
            serde_json::json!({ "message": format!("{error:#}") }),
        );
        UpstreamBook::default()
    });
    claude_mom::apply_active(&book);
}

/// ClaudeMom 只有完整功能模式：设置里还是 3p 时改过来。
fn ensure_full_mode() -> anyhow::Result<()> {
    let store = codex_plus_core::settings::SettingsStore::default();
    let mut settings = store.load()?;
    if !settings.claude_desktop_local.enabled {
        settings.claude_desktop_local.enabled = true;
        store.save(&settings)?;
    }
    Ok(())
}

#[tauri::command]
pub fn claude_mom_load() -> CommandResult<BookPayload> {
    if let Err(error) = ensure_full_mode() {
        return failed(&format!("切换完整功能模式失败：{error:#}"), BookPayload::default());
    }
    match load() {
        Ok(book) => {
            claude_mom::apply_active(&book);
            ok("已读取上游。", book.into())
        }
        Err(error) => failed(&format!("{error:#}"), BookPayload::default()),
    }
}

/// 新增（id 为空）或更新一组；`activate` 为真时同时设为当前。
#[tauri::command]
pub fn claude_mom_save_profile(profile: UpstreamProfile, activate: Option<bool>) -> CommandResult<BookPayload> {
    let result = mutate(|book| {
        let id = book.upsert(profile);
        if activate.unwrap_or(false) {
            book.active_id = Some(id.clone());
        }
        Ok(book.profiles.iter().find(|item| item.id == id).map(|item| item.name.clone()).unwrap_or_default())
    });
    respond(result, |name| format!("已保存「{name}」。"), "保存上游失败")
}

#[tauri::command]
pub fn claude_mom_delete_profile(id: String) -> CommandResult<BookPayload> {
    let result = mutate(|book| {
        let name = book.profiles.iter().find(|item| item.id == id).map(|item| item.name.clone()).unwrap_or_default();
        book.remove(&id);
        Ok(name)
    });
    respond(result, |name| format!("已删除「{name}」。"), "删除上游失败")
}

#[tauri::command]
pub fn claude_mom_set_active(id: String) -> CommandResult<BookPayload> {
    let result = mutate(|book| {
        let name = book
            .profiles
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.name.clone())
            .ok_or_else(|| anyhow::anyhow!("这组上游已经不在了"))?;
        book.active_id = Some(id.clone());
        Ok(name)
    });
    respond(result, |name| format!("已切换到「{name}」，重新启动 Claude 后生效。"), "切换上游失败")
}

/// 从其他工具导入：`ccswitch` / `magpie` / `codexpp`。网关类来源（Magpie、Codex++）没有模型时顺手拉一次。
#[tauri::command]
pub async fn claude_mom_import(source: String) -> CommandResult<BookPayload> {
    let imported = match source.as_str() {
        claude_mom::SOURCE_CC_SWITCH => claude_mom::cc_switch_profiles(&claude_mom::default_cc_switch_db()),
        claude_mom::SOURCE_MAGPIE => Ok(vec![claude_mom::magpie_profile(
            &claude_mom::default_magpie_dir(),
            std::env::var("MAGPIE_ADDR").ok().as_deref(),
        )]),
        claude_mom::SOURCE_CODEX_PLUS_PLUS => {
            claude_mom::codex_plus_plus_profile(&codex_plus_core::paths::codex_plus_plus_state_dir()).map(|profile| vec![profile])
        }
        other => Err(anyhow::anyhow!("未知的导入来源：{other}")),
    };
    let mut imported = match imported {
        Ok(imported) => imported,
        Err(error) => return failed(&format!("{error:#}"), load().unwrap_or_default().into()),
    };
    if imported.is_empty() {
        return failed("没有找到可导入的 Claude 供应商（只导入 Anthropic 格式的）。", load().unwrap_or_default().into());
    }
    let mut unreachable = Vec::new();
    for profile in imported.iter_mut().filter(|profile| profile.models.is_empty()) {
        match claude_mom::fetch_models(&profile.base_url, &profile.api_key).await {
            Ok(models) => {
                profile.models = models.iter().map(|model| model.id.clone()).collect();
                profile.model_names = models
                    .iter()
                    .filter_map(|model| Some((model.id.clone(), model.display_name.clone()?)))
                    .collect();
                if profile.model.is_empty() {
                    profile.model = profile.models.first().cloned().unwrap_or_default();
                }
                profile.context_window = models
                    .iter()
                    .find(|model| model.id == profile.model)
                    .and_then(|model| model.context_window);
            }
            Err(_) => unreachable.push(profile.name.clone()),
        }
    }
    let result = tauri::async_runtime::spawn_blocking(move || mutate(|book| Ok(book.merge_imported(imported))))
        .await
        .unwrap_or_else(|error| Err(anyhow::anyhow!("{error}")));
    respond(
        result,
        |(added, updated)| {
            let mut message = format!("已导入：新增 {added} 组，更新 {updated} 组。");
            if !unreachable.is_empty() {
                message.push_str(&format!("{} 暂时连不上，模型列表稍后再拉。", unreachable.join("、")));
            }
            message
        },
        "导入失败",
    )
}

#[tauri::command]
pub async fn claude_mom_fetch_models(base_url: String, api_key: String) -> CommandResult<ModelsPayload> {
    match claude_mom::fetch_models(&base_url, &api_key).await {
        Ok(models) => ok(&format!("拉到 {} 个模型。", models.len()), ModelsPayload { models }),
        Err(error) => failed(&format!("{error:#}"), ModelsPayload::default()),
    }
}
