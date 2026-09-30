//! 显式指定目录后执行，与管理器及启动器共用同一修复实现。
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 || args[0] != "--home" {
        anyhow::bail!("usage: repair_session_index --home <CODEX_HOME>");
    }
    let path = std::path::Path::new(&args[1]);
    let report = codex_plus_data::repair_session_index(Some(path))?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
