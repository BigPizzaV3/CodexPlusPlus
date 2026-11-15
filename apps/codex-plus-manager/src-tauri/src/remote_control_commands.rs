//! 「远程控制」命令：开关、状态、更换令牌、局域网访问、配对与设备管理。
//! 服务本体在 core 的 `remote_control` 里，由管理器进程托管。
//!
//! 一个命令按 `action` 分发，和 `config_doctor_commands` 一致；带参数的动作从 `args` 取。
//! 启停会等监听线程就绪，放到阻塞线程池里跑。

use codex_plus_core::remote_control as rc;
use serde_json::Value;

fn arg<'a>(args: &'a Option<Value>, key: &str) -> Result<&'a str, String> {
    args.as_ref()
        .and_then(|args| args.get(key))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("缺少参数 {key}"))
}

#[tauri::command]
pub async fn remote_control_request(action: String, args: Option<Value>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let result = match action.as_str() {
            "status" => Ok(rc::status()),
            "enable" => rc::set_enabled(true),
            "disable" => rc::set_enabled(false),
            "resetToken" => rc::reset_token(),
            "enableLan" => rc::set_lan_enabled(true),
            "disableLan" => rc::set_lan_enabled(false),
            "startPairing" => rc::start_pairing(arg(&args, "role")?),
            "cancelPairing" => Ok(rc::cancel_pairing()),
            "setDeviceRole" => rc::set_device_role(arg(&args, "id")?, arg(&args, "role")?),
            "revokeDevice" => rc::revoke_device(arg(&args, "id")?),
            other => return Err(format!("未知操作：{other}")),
        };
        result.map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 管理器启动时：开关打开就把服务拉起来（后台线程，不拖慢窗口出现）。
pub fn start_from_saved_state_in_background() {
    std::thread::spawn(rc::start_from_saved_state);
}
