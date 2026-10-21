//! 显式选择修复范围；CODEX_SQLITE_HOME 与管理器保持一致。
use anyhow::{Context, bail};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().context(
        "usage: recover_session_history --catalog|--messages <codex-home>",
    )?;
    let home = PathBuf::from(args.next().context("missing explicit codex home")?);
    if args.next().is_some() || !home.is_dir() {
        bail!("expected one existing Codex home directory");
    }
    if mode == "--catalog" {
        let report =
            codex_plus_data::provider_sync::run_online_session_catalog_recovery(Some(&home));
        println!("{}", serde_json::to_string_pretty(&report)?);
        if report.status != codex_plus_data::ProviderSyncStatus::Synced {
            bail!("catalog recovery did not complete");
        }
    } else if mode == "--messages" {
        let report = codex_plus_data::repair_session_index(Some(&home))?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if report.aborted_reason.is_some() {
            bail!("message repair aborted; see report");
        }
    } else {
        bail!("expected --catalog or --messages");
    }
    Ok(())
}
