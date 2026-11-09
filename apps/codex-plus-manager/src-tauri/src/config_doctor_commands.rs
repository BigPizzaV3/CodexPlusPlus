//! 「配置体检」命令：本地规则永远可用，AI 深度体检只在用户显式开启后才跑。
//!
//! 一个命令按 `action` 分发，和 `dsh_plugin_commands` / `extension_creator_commands` 一致：
//! 前端只认一条入口，新增动作不用再改 invoke_handler。

use codex_plus_core::config_doctor::{
    self, DoctorSettings, current_source, detect_local_source, report, run_ai_for, run_rules,
    skipped_report,
};
use codex_plus_core::settings::SettingsStore;
use serde_json::{Value, json};

fn status_payload(settings: &DoctorSettings, backend: &codex_plus_core::settings::BackendSettings, home: &std::path::Path, local: config_doctor::LocalSource) -> Value {
    json!({
        "status": "ok",
        "message": "",
        "settings": settings.view(),
        "presets": config_doctor::presets(),
        // 「复用当前供应商」与「本机模型」的可用性：前端据此决定默认选项与提示。
        "sources": { "current": current_source(backend, home), "local": local },
    })
}

fn read_string(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).map(str::to_string)
}

/// `save` 的入参：只覆盖带上的字段，没带的保持原值（避免前端漏传清空 Key）。
fn apply_patch(current: &mut DoctorSettings, payload: &Value) {
    if let Some(enabled) = payload.get("enabled").and_then(Value::as_bool) {
        current.enabled = enabled;
    }
    if let Some(base_url) = read_string(payload, "baseUrl") {
        current.base_url = base_url.trim().to_string();
    }
    if let Some(model) = read_string(payload, "model") {
        current.model = model.trim().to_string();
    }
    if let Some(preset_id) = read_string(payload, "presetId") {
        current.preset_id = preset_id.trim().to_string();
    }
    if let Some(source) = read_string(payload, "source") {
        current.source = source.trim().to_string();
    }
    // 空串表示「清掉 Key」；打码值（含 ***）回传时不覆盖，防止把显示用的掩码写回磁盘。
    if let Some(api_key) = read_string(payload, "apiKey")
        && !api_key.contains("***")
    {
        current.api_key = api_key.trim().to_string();
    }
}

#[tauri::command]
pub async fn config_doctor_request(action: String, payload: Option<Value>) -> Result<Value, String> {
    let payload = payload.unwrap_or(Value::Null);
    match action.as_str() {
        "status" => {
            let settings = config_doctor::load_settings(&config_doctor::settings_path());
            // 读设置是同步 IO；本机模型探测是网络请求，两边并行做。
            let backend = tauri::async_runtime::spawn_blocking(|| SettingsStore::default().load())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            let home = codex_plus_core::codex_home::default_codex_home_dir();
            let local = detect_local_source().await;
            Ok(status_payload(&settings, &backend, &home, local))
        }
        "save" => {
            let path = config_doctor::settings_path();
            let mut settings = config_doctor::load_settings(&path);
            apply_patch(&mut settings, &payload);
            config_doctor::save_settings(&path, &settings).map_err(|error| error.to_string())?;
            let backend = SettingsStore::default().load().unwrap_or_default();
            let home = codex_plus_core::codex_home::default_codex_home_dir();
            let local = detect_local_source().await;
            Ok(status_payload(&settings, &backend, &home, local))
        }
        "run" => {
            let use_ai = payload.get("useAi").and_then(Value::as_bool).unwrap_or(false);
            // 读设置 + 跑规则是同步 IO/CPU，扔到 blocking 池，别占住异步 worker。
            let (settings, home, findings) = tauri::async_runtime::spawn_blocking(move || {
                let store = SettingsStore::default();
                let settings = store.load()?;
                let home = codex_plus_core::codex_home::default_codex_home_dir();
                let findings = run_rules(&settings, &home);
                anyhow::Ok((settings, home, findings))
            })
            .await
            .map_err(|error| format!("体检任务异常：{error}"))?
            .map_err(|error| format!("读取设置失败：{error}"))?;

            let ai_settings = config_doctor::load_settings(&config_doctor::settings_path());
            let snapshot = config_doctor::build_snapshot(&settings, &home, &findings);
            let ai = if use_ai {
                run_ai_for(&ai_settings, &snapshot).await
            } else if ai_settings.enabled {
                skipped_report(&ai_settings, "这次只跑了本地规则")
            } else {
                skipped_report(&ai_settings, "AI 体检没开启，只跑了本地规则")
            };
            serde_json::to_value(report(findings, ai)).map_err(|error| error.to_string())
        }
        other => Err(format!("未知的体检动作：{other}")),
    }
}
