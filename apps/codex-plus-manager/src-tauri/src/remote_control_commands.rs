//! 「远程控制」命令：开关、状态、更换令牌、局域网访问、配对与设备管理。
//! 服务本体在 core 的 `remote_control` 里，由管理器进程托管；动作分发与 Codex 页面
//! （bridge 路由 `/remote-control`）共用 `remote_control::dispatch`。
//!
//! 启停会等监听线程就绪，放到阻塞线程池里跑。

use codex_plus_core::remote_control as rc;
use serde_json::Value;

#[tauri::command]
pub async fn remote_control_request(action: String, args: Option<Value>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        rc::dispatch(&action, &args.unwrap_or(Value::Null)).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 管理器启动时：标记服务由本进程托管（Codex 页面经 bridge 操作时据此放行），
/// 开关打开就把服务拉起来（后台线程，不拖慢窗口出现）。
pub fn start_from_saved_state_in_background() {
    rc::mark_hosted_by_this_process();
    std::thread::spawn(rc::start_from_saved_state);
}
