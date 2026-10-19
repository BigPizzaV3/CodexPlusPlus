use anyhow::Context;
use serde_json::json;

pub fn apply_config() -> anyhow::Result<()> {
    let config = codex_plus_core::global_dictation::load_config();
    codex_plus_core::global_dictation::replace_handle(None);
    if !config.enabled { return Ok(()); }
    codex_plus_core::global_dictation::validate_hotkey(&config.hotkey)?;
    let handle = codex_plus_core::global_dictation::start(&config.hotkey, || {
        let _ = tauri::async_runtime::block_on(trigger_global_toggle());
    })?;
    codex_plus_core::global_dictation::replace_handle(Some(handle));
    Ok(())
}

async fn trigger_global_toggle() -> anyhow::Result<()> {
    let debug_port = codex_plus_core::status::StatusStore::default()
        .load_latest()
        .ok()
        .flatten()
        .and_then(|status| status.debug_port)
        .unwrap_or(9229);
    let targets = codex_plus_core::cdp::list_targets(debug_port).await?;
    let target = codex_plus_core::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket = target.web_socket_debugger_url.as_deref().context("Codex 页面没有 websocket")?;
    codex_plus_core::bridge::evaluate_script(
        websocket,
        "window.__codexPlusDictationGlobalToggle?.()",
    ).await?;
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "dictation.global_hotkey_toggle",
        json!({ "debugPort": debug_port }),
    );
    Ok(())
}
