use std::fs;
use std::path::PathBuf;

use anyhow::Context;

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct LaunchStatus {
    pub status: String,
    pub message: String,
    pub started_at_ms: u64,
    pub debug_port: Option<u16>,
    pub helper_port: Option<u16>,
    pub codex_app: Option<String>,
    pub aumid: Option<String>,
    /// 当前启动阶段（如 `provider_sync`、`helper_bind`、`launch_codex`）。
    ///
    /// issue #2244：冷启动 80~110 秒期间 latest-status.json 只有 `starting`，
    /// 用户分不清「还在处理」与「已经卡死」。启动链路每进入一个阶段就重写一次
    /// latest-status，配合 `progress` 给出可判断的中间态。
    ///
    /// 两个字段都是 `Option` + `#[serde(default)]`：旧版写下的 latest-status.json
    /// 没有这两个键，反序列化必须仍然成功，否则升级后第一次启动会读不到历史状态。
    /// 构造点分布在本 crate 之外（launcher / manager），一律用 `..Default::default()`
    /// 补齐，避免以后再加字段时重复改多个文件。
    pub phase: Option<String>,
    /// 启动进度百分比（0~100），与 `phase` 配套。
    pub progress: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct StatusStore {
    path: PathBuf,
}

fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Default for StatusStore {
    fn default() -> Self {
        Self::new(crate::paths::default_latest_status_path())
    }
}

impl StatusStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn save_latest(&self, status: &LaunchStatus) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(status)?;
        crate::settings::atomic_write(&self.path, &bytes)
    }

    /// 在已有状态上打一个阶段标记（issue #2244）。
    ///
    /// 启动链路里各步骤目前没有自己的状态写入点，用这个方法把当前阶段与进度
    /// 叠加到最近一次状态上；此前没有状态文件时用一个 `starting` 占位记录生成，
    /// 保证用户在任何时刻都能看到「卡在哪一步」。
    pub fn save_phase(&self, phase: &str, progress: u8) -> anyhow::Result<()> {
        let mut status = self.load_latest()?.unwrap_or_else(|| LaunchStatus {
            status: "starting".to_string(),
            message: "Codex++ launcher is starting".to_string(),
            started_at_ms: current_timestamp_ms(),
            ..LaunchStatus::default()
        });
        status.phase = Some(phase.to_string());
        status.progress = Some(progress.min(100));
        self.save_latest(&status)
    }

    pub fn load_latest(&self) -> anyhow::Result<Option<LaunchStatus>> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to read latest status {}", self.path.display())
                });
            }
        };

        Ok(serde_json::from_str(&contents).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codex-plus-core-status-test-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn status_store_save_load_latest_roundtrip_uses_custom_path() {
        let dir = temp_dir();
        let store = StatusStore::new(dir.join("nested").join("latest-status.json"));
        let status = LaunchStatus {
            status: "running".to_string(),
            message: "ready".to_string(),
            started_at_ms: 12345,
            debug_port: Some(9222),
            helper_port: Some(4545),
            codex_app: Some("Codex".to_string()),
            aumid: Some("OpenAI.Codex_abc!App".to_string()),
            phase: None,
            progress: None,
        };

        store.save_latest(&status).unwrap();

        assert_eq!(store.load_latest().unwrap(), Some(status));
    }

    /// issue #2244：升级前写下的 latest-status.json 没有 phase/progress，
    /// 反序列化必须仍然成功且两个新字段为 None。
    #[test]
    fn legacy_status_json_without_phase_deserializes() {
        let dir = temp_dir();
        let path = dir.join("latest-status.json");
        std::fs::write(
            &path,
            r#"{
  "status": "starting",
  "message": "Codex++ launcher is starting",
  "started_at_ms": 1789783159114,
  "debug_port": 9229,
  "helper_port": 57321,
  "codex_app": null,
  "aumid": null
}"#,
        )
        .unwrap();

        let status = StatusStore::new(path).load_latest().unwrap().unwrap();
        assert_eq!(status.status, "starting");
        assert_eq!(status.phase, None);
        assert_eq!(status.progress, None);
    }

    /// 阶段写入要在已有状态上叠加，而不是把 status/message 冲掉。
    #[test]
    fn save_phase_annotates_existing_status_without_clobbering_it() {
        let dir = temp_dir();
        let store = StatusStore::new(dir.join("latest-status.json"));
        store
            .save_latest(&LaunchStatus {
                status: "starting".to_string(),
                message: "Codex++ launcher is starting".to_string(),
                started_at_ms: 12345,
                debug_port: Some(9229),
                helper_port: Some(57321),
                ..LaunchStatus::default()
            })
            .unwrap();

        store.save_phase("helper_bind", 60).unwrap();

        let status = store.load_latest().unwrap().unwrap();
        assert_eq!(status.status, "starting");
        assert_eq!(status.started_at_ms, 12345);
        assert_eq!(status.phase.as_deref(), Some("helper_bind"));
        assert_eq!(status.progress, Some(60));
    }

    /// 还没有状态文件时也要能写出阶段，供 manager 立即显示进度。
    #[test]
    fn save_phase_creates_starting_status_when_missing() {
        let dir = temp_dir();
        let store = StatusStore::new(dir.join("nested").join("latest-status.json"));

        store.save_phase("provider_sync", 120).unwrap();

        let status = store.load_latest().unwrap().unwrap();
        assert_eq!(status.status, "starting");
        // 进度上限收敛到 100，避免越界值传到 UI。
        assert_eq!(status.progress, Some(100));
    }

    #[test]
    fn status_store_load_latest_missing_file_returns_none() {
        let dir = temp_dir();
        let store = StatusStore::new(dir.join("latest-status.json"));

        assert_eq!(store.load_latest().unwrap(), None);
    }

    #[test]
    fn status_store_load_latest_bad_json_returns_none() {
        let dir = temp_dir();
        let path = dir.join("latest-status.json");
        std::fs::write(&path, "{bad json").unwrap();
        let store = StatusStore::new(path);

        assert_eq!(store.load_latest().unwrap(), None);
    }
}
