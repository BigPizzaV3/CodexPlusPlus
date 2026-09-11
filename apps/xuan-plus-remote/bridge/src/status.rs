use std::path::PathBuf;

use anyhow::Context;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestStatus {
    pub debug_port: Option<u16>,
}

#[derive(Default)]
pub struct StatusStore;

impl StatusStore {
    pub fn load_latest(&self) -> anyhow::Result<Option<LatestStatus>> {
        if let Some(port) = std::env::var("XUAN_CODEX_DEBUG_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
        {
            return Ok(Some(LatestStatus {
                debug_port: Some(port),
            }));
        }
        let Some(path) = status_path() else {
            return Ok(None);
        };
        if !path.is_file() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("无法读取 Codex 状态文件 {}", path.display()))?;
        let status = serde_json::from_str(&raw).context("Codex 状态文件格式无效")?;
        Ok(Some(status))
    }
}

fn status_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XUAN_CODEX_STATUS_PATH") {
        return Some(PathBuf::from(path));
    }
    directories::BaseDirs::new().map(|dirs| {
        dirs.home_dir()
            .join(".codex-session-delete")
            .join("latest-status.json")
    })
}
