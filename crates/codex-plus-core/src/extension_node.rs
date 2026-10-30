//! 拓展的 Node 侧执行：钩子、网关中间件、拓展工具都以「一段运行器脚本 + stdin JSON」的方式
//! 起一个 Node 进程，运行器再 import 拓展脚本并调用其默认导出。这里只管起进程、喂输入、
//! 读输出与整体超时；各自的运行器脚本与结果语义由调用方定义。

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use serde_json::Value;

/// 用 `node --input-type=module -e <runner>` 运行，stdin 写 `input`，读 stdout 解析为 JSON。
/// 超过 `limit` 即杀进程并报错。stderr 丢弃（拓展脚本的日志不应混进协议输出）。
pub async fn run_runner(node: &Path, runner: &str, input: &Value, limit: Duration) -> anyhow::Result<Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut command = tokio::process::Command::new(node);
    command
        .args(["--input-type=module", "-e", runner])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    // Windows 上不能闪出黑色终端窗口。
    #[cfg(windows)]
    command.creation_flags(crate::windows_create_no_window());
    let mut child = command.spawn().context("无法启动 Node 运行拓展脚本")?;
    let mut stdin = child.stdin.take().context("Node 进程没有 stdin")?;
    stdin.write_all(input.to_string().as_bytes()).await?;
    drop(stdin);
    let mut stdout = child.stdout.take().context("Node 进程没有 stdout")?;
    let read = async {
        let mut buffer = Vec::new();
        stdout.read_to_end(&mut buffer).await?;
        child.wait().await?;
        anyhow::Ok(buffer)
    };
    let buffer = tokio::time::timeout(limit, read).await.context("拓展脚本整体超时")??;
    serde_json::from_slice(&buffer).context("拓展脚本运行器输出不是 JSON")
}

/// 带宿主调用的运行方式：stdin 第一行写 `input` 且保持打开；运行器中途可以写
/// `{ host, service, method, params }` 请求 Codex++（目前是只读会话查询），回复以
/// `{ hostReply, result | error }` 写回 stdin；最后一行 `{ final: 结果 }` 即运行结果。
pub async fn run_runner_with_host(node: &Path, runner: &str, input: &Value, limit: Duration) -> anyhow::Result<Value> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let mut command = tokio::process::Command::new(node);
    command
        .args(["--input-type=module", "-e", runner])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(crate::windows_create_no_window());
    let mut child = command.spawn().context("无法启动 Node 运行拓展脚本")?;
    let mut stdin = child.stdin.take().context("Node 进程没有 stdin")?;
    let stdout = child.stdout.take().context("Node 进程没有 stdout")?;
    stdin.write_all(format!("{input}\n").as_bytes()).await?;
    stdin.flush().await?;
    let run = async {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await? {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(id) = message.get("host").and_then(Value::as_u64) {
                let reply = match crate::extension_backend::host_call(&message).await {
                    Ok(result) => serde_json::json!({ "hostReply": id, "result": result }),
                    Err(error) => serde_json::json!({ "hostReply": id, "error": format!("{error:#}") }),
                };
                stdin.write_all(format!("{reply}\n").as_bytes()).await?;
                stdin.flush().await?;
            } else if let Some(result) = message.get("final") {
                return anyhow::Ok(result.clone());
            }
        }
        anyhow::bail!("拓展脚本运行器没有给出结果")
    };
    let result = tokio::time::timeout(limit, run).await.context("拓展脚本整体超时")?;
    let _ = child.kill().await;
    result
}

/// 找 Node；找不到时给出统一的提示。
pub fn require_node() -> anyhow::Result<std::path::PathBuf> {
    crate::dsh_plugins::locate_node().context(
        "没有找到 Node 运行时：请安装 Codex App（自带 Node）或 Node.js，也可用 CODEX_PLUS_NODE 指定",
    )
}
