use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};

static TEST_LOG_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
static DIAGNOSTIC_LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

const MAX_DIAGNOSTIC_LOG_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DIAGNOSTIC_LOG_ARCHIVES: usize = 3;

#[derive(Debug, Clone, Serialize)]
struct DiagnosticRecord {
    timestamp_ms: u64,
    pid: u32,
    event: String,
    detail: Value,
}

pub fn append_diagnostic_log(event: &str, detail: impl Serialize) -> std::io::Result<()> {
    if !should_persist_event(event) {
        return Ok(());
    }
    let _guard = DIAGNOSTIC_LOG_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| std::io::Error::other("diagnostic log lock poisoned"))?;
    let path = diagnostic_log_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let detail = serde_json::to_value(detail).unwrap_or_else(|error| {
        json!({
            "serialization_error": error.to_string()
        })
    });
    let record = DiagnosticRecord {
        timestamp_ms: now_ms(),
        pid: std::process::id(),
        event: event.to_string(),
        detail,
    };
    let line = serde_json::to_string(&record).unwrap_or_else(|error| {
        json!({
            "timestamp_ms": now_ms(),
            "pid": std::process::id(),
            "event": "diagnostic_log.serialization_failed",
            "detail": {
                "message": error.to_string()
            }
        })
        .to_string()
    });

    rotate_diagnostic_log(&path, MAX_DIAGNOSTIC_LOG_BYTES, MAX_DIAGNOSTIC_LOG_ARCHIVES)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

pub fn clear_diagnostic_log() -> std::io::Result<()> {
    let _guard = DIAGNOSTIC_LOG_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| std::io::Error::other("diagnostic log lock poisoned"))?;
    let path = diagnostic_log_path();
    clear_diagnostic_log_path(&path)?;
    for index in 1..=MAX_DIAGNOSTIC_LOG_ARCHIVES {
        clear_diagnostic_log_path(&diagnostic_log_archive_path(&path, index))?;
    }
    Ok(())
}

fn clear_diagnostic_log_path(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn diagnostic_log_path() -> PathBuf {
    if let Some(lock) = TEST_LOG_PATH.get() {
        if let Ok(guard) = lock.lock() {
            if let Some(path) = &*guard {
                return path.clone();
            }
        }
    }
    crate::paths::default_diagnostic_log_path()
}

#[doc(hidden)]
pub fn set_diagnostic_log_path_for_tests(path: Option<PathBuf>) {
    let lock = TEST_LOG_PATH.get_or_init(|| Mutex::new(None));
    *lock.lock().expect("test log path lock poisoned") = path;
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn should_persist_event(event: &str) -> bool {
    !matches!(
        event,
        "bridge.request" | "bridge.response" | "bridge.resolve_start" | "bridge.resolve_ok"
    )
}

fn rotate_diagnostic_log(path: &Path, max_bytes: u64, archives: usize) -> std::io::Result<()> {
    let len = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if len < max_bytes {
        return Ok(());
    }

    for index in (1..=archives).rev() {
        let destination = diagnostic_log_archive_path(path, index);
        if destination.exists() {
            std::fs::remove_file(&destination)?;
        }
        let source = if index == 1 {
            path.to_path_buf()
        } else {
            diagnostic_log_archive_path(path, index - 1)
        };
        if source.exists() {
            std::fs::rename(source, destination)?;
        }
    }
    Ok(())
}

fn diagnostic_log_archive_path(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_diagnostic_log_renames_files_and_keeps_three_archives() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("codex-plus.log");
        for index in 1..=4 {
            std::fs::write(&path, format!("line-{index}\n")).unwrap();
            rotate_diagnostic_log(&path, 1, 3).unwrap();
        }

        assert_eq!(
            std::fs::read_to_string(format!("{}.1", path.display())).unwrap(),
            "line-4\n"
        );
        assert_eq!(
            std::fs::read_to_string(format!("{}.2", path.display())).unwrap(),
            "line-3\n"
        );
        assert_eq!(
            std::fs::read_to_string(format!("{}.3", path.display())).unwrap(),
            "line-2\n"
        );
        assert!(!std::path::Path::new(&format!("{}.4", path.display())).exists());
    }

    #[test]
    fn high_frequency_success_events_are_not_persisted() {
        for event in [
            "bridge.request",
            "bridge.response",
            "bridge.resolve_start",
            "bridge.resolve_ok",
        ] {
            assert!(!should_persist_event(event));
        }
        assert!(should_persist_event("bridge.resolve_failed"));
        assert!(should_persist_event(
            "protocol_proxy.usage_ledger_write_failed"
        ));
    }

    #[test]
    fn clear_diagnostic_log_ignores_missing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing.log");

        clear_diagnostic_log_path(&path).unwrap();
    }
}
