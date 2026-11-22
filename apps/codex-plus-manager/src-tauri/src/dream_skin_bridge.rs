//! Codex 设置页里的「皮肤」分区（`96-dream-skin-page.js`）经 bridge 路由 `/dream-skin` 调到这里。
//!
//! 皮肤只在 Codex App 里有意义，界面只放在 Codex 里；管理器只留「恢复默认皮肤」这个急救入口。
//! 每个动作调管理器原来用的同一组 Tauri 命令函数（主题库、市场、社区、激活、恢复），不另写一份：
//!
//! - `overview`：开关状态 + 主题库（不含本地路径）
//! - `preview {key}`：主题库里某个主题的预览图（data URL）；路径由服务端按 key 找，页面给不了路径
//! - `activate {key}`：打开皮肤并应用这个主题（同管理器「应用所选主题」：先存开关，再激活）
//! - `restore`：关掉皮肤、恢复 Codex 原始外观
//! - `transparency {value}`：「壁纸透出」偏好（0–100，null = 跟随主题，#3233）。存在 Codex++ 设置里、
//!   不写进主题包，注入时并进主题配置交给上游渲染器；皮肤开着就当场实时重应用，不用重启 Codex
//! - `load {key}` / `save {config, media?}`：自定义主题；`media` 是页面选的文件内容，
//!   服务端落到自己的上传目录再交给保存函数——页面提交的本地路径一律不认
//! - `create {media}` / `import {media}`：从图片或视频新建主题、导入主题包（同样只收文件内容）
//! - `rename {id, name}` / `delete {id}`
//! - `market` / `installMarket {id}` / `community` / `installCommunity {id}`
//! - `status`：实机检查（经 CDP 查 Codex 里皮肤的运行状态）
//! - `restart`：换主题后需要重启 Codex 时，用上次启动的端口重启
//!
//! 不在拓展路由白名单里（见 `91-extension-api.js` 的 `codexPlusExtensionRoutes`）。

use std::path::{Path, PathBuf};

use base64::Engine;
use codex_plus_core::dream_skin_library::{DreamSkinThemeDraft, DreamSkinThemeKind, DreamSkinThemeLibrary};
use codex_plus_core::settings::{DreamSkinThemeConfig, SettingsStore};
use codex_plus_core::status::StatusStore;
use serde_json::{Value, json};

use crate::commands;

/// 页面上传的单个文件上限：与核心对皮肤源文件的上限一致。
const UPLOAD_LIMIT: usize = 50 * 1024 * 1024;
/// 预览图只给图片，太大的不内联（页面显示占位）。
const PREVIEW_LIMIT: u64 = 8 * 1024 * 1024;

pub async fn bridge_request(payload: Value) -> anyhow::Result<Value> {
    let action = payload.get("action").and_then(Value::as_str).unwrap_or("overview");
    let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let value = match action {
        "overview" => overview(),
        "preview" => preview(&text("key")),
        "activate" => activate(&text("key")).await?,
        "restore" => {
            let (debug_port, helper_port) = launch_ports();
            let result = commands::restore_dream_skin(runtime_request(debug_port, helper_port)).await;
            with_overview(serde_json::to_value(result)?)
        }
        "transparency" => set_transparency(payload.get("value")).await?,
        "load" => match draft_for_key(&text("key")) {
            Ok(draft) => json!({ "status": "ok", "message": "", "config": draft.config, "builtin": draft.builtin }),
            Err(error) => json!({ "status": "failed", "message": format!("读取主题失败：{error}") }),
        },
        "save" => save(&payload)?,
        "create" => {
            let upload = Upload::from_payload(&payload, "media", &codex_plus_core::paths::default_app_state_dir())?;
            let result = commands::create_dream_skin_theme(upload.path.to_string_lossy().to_string());
            json!({ "status": result.status, "message": result.message, "id": result.payload.config.id })
        }
        "import" => {
            let upload = Upload::from_payload(&payload, "media", &codex_plus_core::paths::default_app_state_dir())?;
            library_value(commands::import_dream_skin_theme_package(upload.path.to_string_lossy().to_string()))?
        }
        "rename" => library_value(commands::rename_dream_skin_theme(text("id"), text("name")))?,
        "delete" => library_value(commands::delete_dream_skin_theme(text("id")))?,
        "market" => serde_json::to_value(commands::refresh_dream_skin_market().await)?,
        "installMarket" => serde_json::to_value(commands::install_dream_skin_market_theme(text("id")).await)?,
        "community" => serde_json::to_value(commands::refresh_dream_skin_community().await)?,
        "installCommunity" => serde_json::to_value(commands::install_dream_skin_community_theme(text("id")).await)?,
        "status" => {
            let (debug_port, helper_port) = launch_ports();
            serde_json::to_value(commands::dream_skin_status(runtime_request(debug_port, helper_port)).await)?
        }
        "restart" => {
            let (debug_port, helper_port) = launch_ports();
            let latest = StatusStore::default().load_latest().ok().flatten();
            let settings = SettingsStore::default().load().unwrap_or_default();
            let app_path = latest
                .and_then(|launch| launch.codex_app)
                .filter(|path| !path.trim().is_empty())
                .unwrap_or(settings.codex_app_path);
            let result = commands::restart_codex_plus(commands::LaunchRequest {
                app_path,
                debug_port,
                helper_port,
                sync_active_relay: false,
            })
            .await;
            json!({ "status": result.status, "message": result.message })
        }
        other => anyhow::bail!("不支持的皮肤操作：{other}"),
    };
    // 开关、当前主题都存在设置里：让管理器界面跟着刷新。
    if matches!(action, "activate" | "restore" | "save" | "rename" | "delete" | "transparency") {
        codex_plus_core::settings::notify_external_settings_change();
    }
    Ok(value)
}

/// 管理器里确认网页一键换肤（`dreamskin://` 链接）后，应用刚装好的主题：与 Codex 皮肤分区的「应用」同一流程。
#[tauri::command]
pub async fn activate_installed_dream_skin_theme(id: String) -> Value {
    activate(&format!("stored:{}", id.trim()))
        .await
        .unwrap_or_else(|error| json!({ "status": "failed", "message": format!("应用主题失败：{error}") }))
}

/// 上次启动 Codex 用的调试端口和 helper 端口（与管理器取的一致）。
fn launch_ports() -> (u16, u16) {
    let latest = StatusStore::default().load_latest().ok().flatten();
    (
        latest.as_ref().and_then(|launch| launch.debug_port).unwrap_or(9229),
        latest
            .as_ref()
            .and_then(|launch| launch.helper_port)
            .unwrap_or_else(codex_plus_core::protocol_proxy::protocol_proxy_port),
    )
}

fn runtime_request(debug_port: u16, helper_port: u16) -> commands::DreamSkinRuntimeRequest {
    commands::DreamSkinRuntimeRequest { debug_port, helper_port }
}

fn current_library() -> anyhow::Result<DreamSkinThemeLibrary> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    codex_plus_core::dream_skin_library::list_dream_skin_themes(
        &codex_plus_core::paths::default_app_state_dir(),
        &settings,
    )
}

/// 主题库给页面的样子：去掉本地路径，只留 key / 名称 / 状态。
fn library_themes(library: &DreamSkinThemeLibrary) -> Value {
    Value::Array(
        library
            .themes
            .iter()
            .map(|theme| {
                json!({
                    "key": theme.key,
                    "id": theme.id,
                    "name": theme.name,
                    "kind": theme.kind,
                    "builtin": theme.builtin,
                    "active": theme.active,
                    "modified": theme.modified,
                    "hasPreview": !theme.preview_path.trim().is_empty(),
                })
            })
            .collect(),
    )
}

fn overview() -> Value {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let (themes, message) = match current_library() {
        Ok(library) => (library_themes(&library), String::new()),
        Err(error) => (json!([]), format!("读取主题库失败：{error}")),
    };
    json!({
        "status": if message.is_empty() { "ok" } else { "failed" },
        "message": message,
        "enabled": settings.codex_app_dream_skin_enabled,
        "paused": settings.codex_app_dream_skin_paused,
        "enhancementsEnabled": settings.enhancements_enabled,
        "activeName": settings.codex_app_dream_skin_theme_config.name,
        "userTransparency": settings.codex_app_dream_skin_user_transparency,
        // 只有 dream-skin 引擎认这个偏好；雪境、玻璃视界、赛博主题等引擎不读它。
        "transparencySupported": codex_plus_core::assets::dream_skin_target_engine(&settings) == "dream-skin",
        "themes": themes,
    })
}

/// 命令结果（`status` / `message`）再附上最新的开关和主题库，页面一次刷新完。
fn with_overview(mut value: Value) -> Value {
    let fresh = overview();
    if let (Some(target), Some(source)) = (value.as_object_mut(), fresh.as_object()) {
        for key in ["enabled", "paused", "enhancementsEnabled", "activeName", "userTransparency", "transparencySupported", "themes"] {
            target.insert(key.to_string(), source[key].clone());
        }
    }
    value
}

fn library_value(result: commands::CommandResult<DreamSkinThemeLibrary>) -> anyhow::Result<Value> {
    Ok(with_overview(json!({ "status": result.status, "message": result.message })))
}

fn preview(key: &str) -> Value {
    let Ok(library) = current_library() else {
        return json!({ "status": "failed", "message": "读取主题库失败" });
    };
    let Some(theme) = library.themes.iter().find(|theme| theme.key == key) else {
        return json!({ "status": "failed", "message": "主题不存在" });
    };
    match image_data_url(Path::new(theme.preview_path.trim())) {
        Some(url) => json!({ "status": "ok", "key": key, "url": url }),
        None => json!({ "status": "ok", "key": key, "url": "" }),
    }
}

fn image_data_url(path: &Path) -> Option<String> {
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    };
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > PREVIEW_LIMIT {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// 按 key 取要激活的草稿：内置主题用默认配置，当前主题（含未存进主题库的）用设置里的那份，其余读主题库。
fn draft_for_key(key: &str) -> anyhow::Result<DreamSkinThemeDraft> {
    let library = current_library()?;
    let theme = library
        .themes
        .iter()
        .find(|theme| theme.key == key)
        .ok_or_else(|| anyhow::anyhow!("主题不存在"))?;
    if theme.builtin {
        return Ok(DreamSkinThemeDraft { config: DreamSkinThemeConfig::default(), image_path: String::new(), builtin: true });
    }
    if theme.active || theme.kind == DreamSkinThemeKind::ActiveUnsaved {
        return Ok(library.active_draft);
    }
    codex_plus_core::dream_skin_library::load_stored_dream_skin_theme(
        &codex_plus_core::paths::default_app_state_dir(),
        &theme.id,
    )
}

/// 同管理器的「应用所选主题」：先打开皮肤开关（取消暂停）存进设置，再走激活命令。
async fn activate(key: &str) -> anyhow::Result<Value> {
    let draft = draft_for_key(key)?;
    SettingsStore::default().update(json!({
        "codexAppDreamSkinEnabled": true,
        "codexAppDreamSkinPaused": false,
    }))?;
    let (debug_port, helper_port) = launch_ports();
    let result = commands::activate_dream_skin_theme(commands::DreamSkinThemeActivationRequest {
        draft,
        debug_port,
        helper_port,
    })
    .await;
    Ok(with_overview(json!({
        "status": result.status,
        "message": result.message,
        "savedForNextLaunch": result.payload.saved_for_next_launch,
    })))
}

/// 「壁纸透出」：存进设置；皮肤正在用就实时重应用（只换了主题配置里的偏好，引擎和样式不变，
/// 不会有别的主题样式残留，所以不像换主题那样要求重启）。实时应用失败时已保存、下次启动生效。
async fn set_transparency(value: Option<&Value>) -> anyhow::Result<Value> {
    let value = transparency_patch_value(value)?;
    let settings = SettingsStore::default().update(json!({ "codexAppDreamSkinUserTransparency": value }))?;
    let live = settings.enhancements_enabled
        && settings.codex_app_dream_skin_enabled
        && !settings.codex_app_dream_skin_paused;
    if !live {
        return Ok(with_overview(json!({ "status": "ok", "message": "已保存，打开皮肤后生效。" })));
    }
    let (debug_port, helper_port) = launch_ports();
    let result = match codex_plus_core::dream_skin_runtime::apply_dream_skin_live(debug_port, helper_port).await {
        Ok(_) => json!({ "status": "ok", "message": "" }),
        Err(error) => json!({
            "status": "pending_restart",
            "message": format!("已保存，重启 Codex 后生效；当前无法实时应用：{error}"),
            "savedForNextLaunch": true,
        }),
    };
    Ok(with_overview(result))
}

/// 页面给的值：没给或 null 是跟随主题；数字取整收进 0–100；别的一律拒绝。
fn transparency_patch_value(value: Option<&Value>) -> anyhow::Result<Value> {
    match value {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(raw) => codex_plus_core::settings::normalize_dream_skin_user_transparency(raw)
            .map(Value::from)
            .ok_or_else(|| anyhow::anyhow!("壁纸透出必须是 0–100 的数字")),
    }
}

/// 保存自定义主题：媒体只认页面上传的文件内容，没传就沿用主题库里这个主题现有的媒体
/// （内置或新主题没有就用默认图）。
fn save(payload: &Value) -> anyhow::Result<Value> {
    let config: DreamSkinThemeConfig = serde_json::from_value(
        payload.get("config").cloned().ok_or_else(|| anyhow::anyhow!("缺少主题配置"))?,
    )?;
    let upload = match payload.get("media") {
        Some(media) if !media.is_null() => Some(Upload::from_payload(payload, "media", &codex_plus_core::paths::default_app_state_dir())?),
        _ => None,
    };
    let image_path = match &upload {
        Some(upload) => upload.path.to_string_lossy().to_string(),
        None => codex_plus_core::dream_skin_library::load_stored_dream_skin_theme(
            &codex_plus_core::paths::default_app_state_dir(),
            &config.id,
        )
        .map(|stored| stored.image_path)
        .unwrap_or_else(|_| {
            // 还没存进主题库的当前主题：沿用设置里正在用的媒体，与管理器「另存」一致。
            let settings = SettingsStore::default().load().unwrap_or_default();
            if settings.codex_app_dream_skin_theme_config.id == config.id {
                settings.codex_app_dream_skin_image_path
            } else {
                String::new()
            }
        }),
    };
    let result = commands::save_dream_skin_theme(DreamSkinThemeDraft { config, image_path, builtin: false });
    library_value(result)
}

/// 页面上传的文件：落到状态目录下上传区的独立子目录，用完连目录一起删。
/// 文件名只保留原名里的普通字符（新建主题用它当名称），内容由核心函数校验。
struct Upload {
    dir: PathBuf,
    path: PathBuf,
}

impl Upload {
    fn from_payload(payload: &Value, key: &str, state_dir: &Path) -> anyhow::Result<Self> {
        let media = payload.get(key).ok_or_else(|| anyhow::anyhow!("没有选择文件"))?;
        let name = media.get("name").and_then(Value::as_str).unwrap_or_default();
        let extension = Path::new(name)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|value| !value.is_empty() && value.len() <= 8 && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .ok_or_else(|| anyhow::anyhow!("文件没有可识别的扩展名"))?;
        let stem: String = Path::new(name)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .chars()
            .filter(|ch| !ch.is_control() && !matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
            .take(80)
            .collect();
        let stem = stem.trim().trim_start_matches('.').to_string();
        let stem = if stem.is_empty() { "Dream Skin".to_string() } else { stem };
        let data = media.get("data").and_then(Value::as_str).unwrap_or_default();
        if data.len() > UPLOAD_LIMIT / 3 * 4 + 4 {
            anyhow::bail!("文件超过 50 MiB");
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| anyhow::anyhow!("文件内容不是有效的 base64"))?;
        if bytes.is_empty() {
            anyhow::bail!("文件是空的");
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let dir = state_dir
            .join("dream-skin")
            .join("uploads")
            .join(format!("upload-{stamp}"));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{stem}.{extension}"));
        let upload = Self { dir, path };
        std::fs::write(&upload.path, bytes)?;
        Ok(upload)
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(name: &str, bytes: &[u8]) -> Value {
        json!({ "media": { "name": name, "data": base64::engine::general_purpose::STANDARD.encode(bytes) } })
    }

    #[test]
    fn upload_keeps_a_safe_file_name_and_cleans_up() {
        let temp = tempfile::tempdir().unwrap();
        let upload = Upload::from_payload(&media("../../etc/春日:樱花?.PNG", b"png-bytes"), "media", temp.path()).unwrap();
        let path = upload.path.clone();
        assert_eq!(path.file_name().unwrap().to_string_lossy(), "春日樱花.png");
        assert!(path.starts_with(temp.path().join("dream-skin").join("uploads")));
        assert_eq!(std::fs::read(&path).unwrap(), b"png-bytes");
        drop(upload);
        assert!(!path.exists() && !path.parent().unwrap().exists(), "用完连上传目录一起删掉");
    }

    #[test]
    fn transparency_patch_accepts_numbers_and_follow_theme_only() {
        assert_eq!(transparency_patch_value(None).unwrap(), Value::Null);
        assert_eq!(transparency_patch_value(Some(&Value::Null)).unwrap(), Value::Null);
        assert_eq!(transparency_patch_value(Some(&json!(42))).unwrap(), json!(42));
        assert_eq!(transparency_patch_value(Some(&json!(12.4))).unwrap(), json!(12));
        assert_eq!(transparency_patch_value(Some(&json!(180))).unwrap(), json!(100));
        assert!(transparency_patch_value(Some(&json!("30"))).is_err());
        assert!(transparency_patch_value(Some(&json!({ "value": 30 }))).is_err());
    }

    #[test]
    fn upload_rejects_missing_extension_bad_data_and_empty_files() {
        let temp = tempfile::tempdir().unwrap();
        assert!(Upload::from_payload(&media("noext", b"x"), "media", temp.path()).is_err());
        assert!(Upload::from_payload(&media("a.p/ng", b"x"), "media", temp.path()).is_err());
        assert!(Upload::from_payload(&json!({ "media": { "name": "a.png", "data": "***" } }), "media", temp.path()).is_err());
        assert!(Upload::from_payload(&media("a.png", b""), "media", temp.path()).is_err());
        assert!(Upload::from_payload(&json!({}), "media", temp.path()).is_err());
    }
}
