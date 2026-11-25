//! 在隔离环境里单独跑本地账号的兼容服务（`https://localhost:8000`），供端到端验证插件目录与安装。
//!
//! 环境变量：
//!   CODEX_HOME             隔离的 Codex 目录（必填，不能是 ~/.codex；插件目录缓存从这里读）
//!   CODEX_PLUS_STATE_DIR   隔离的状态目录（必填；证书、已安装列表放这里）
//!
//! 启动后把 `launch_env()` 打出来，Codex 带上这些环境变量就会把 ChatGPT 后端请求发到这里。
//!
//! cargo run -p codex-plus-core --example chatgpt_compat_serve

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let home = std::path::PathBuf::from(std::env::var("CODEX_HOME")?);
    let default_home = codex_plus_core::user_dirs::BaseDirs::new().map(|dirs| dirs.home_dir().join(".codex"));
    if Some(&home) == default_home.as_ref() {
        anyhow::bail!("CODEX_HOME 不能是 ~/.codex：只在隔离环境里跑");
    }
    std::env::var("CODEX_PLUS_STATE_DIR")?;
    codex_plus_core::chatgpt_compat::start().await?;
    for (key, value) in codex_plus_core::chatgpt_compat::launch_env()? {
        println!("{key}={value}");
    }
    println!("ready");
    // 一直跑，直到进程被结束。
    std::future::pending::<()>().await;
    Ok(())
}
