//! 「远程控制」命令：开关、状态、更换令牌。服务本体在 core 的 `remote_control` 里，由管理器进程托管。
//!
//! 一个命令按 `action` 分发，和 `config_doctor_commands` 一致。启停会等监听线程就绪，放到阻塞线程池里跑。

use serde_json::Value;

#[tauri::command]
pub async fn remote_control_request(action: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || match action.as_str() {
        "status" => Ok(codex_plus_core::remote_control::status()),
        "enable" => codex_plus_core::remote_control::set_enabled(true).map_err(|error| error.to_string()),
        "disable" => codex_plus_core::remote_control::set_enabled(false).map_err(|error| error.to_string()),
        "resetToken" => codex_plus_core::remote_control::reset_token().map_err(|error| error.to_string()),
        other => Err(format!("未知操作：{other}")),
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 管理器启动时：开关打开就把服务拉起来（后台线程，不拖慢窗口出现）。
pub fn start_from_saved_state_in_background() {
    std::thread::spawn(codex_plus_core::remote_control::start_from_saved_state);
}
