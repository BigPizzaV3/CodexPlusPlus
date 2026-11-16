//! 「任务面板」命令：列出、新建、修改、删除任务。存储与动作分发都在 core 的 `task_board` 里，
//! 每个动作都返回整个面板。读写的是本地小文件，放到阻塞线程池里跑，不占异步运行时。

use codex_plus_core::task_board;
use serde_json::Value;

#[tauri::command]
pub async fn task_board_request(action: String, args: Option<Value>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        task_board::dispatch(&action, &args.unwrap_or(Value::Null)).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}
