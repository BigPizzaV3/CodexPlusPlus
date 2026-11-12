use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

// 文件还原可能保留时间戳和大小，命中缓存最多复用 30 秒。
const INDEX_MAX_AGE: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct ThreadNameCache {
    path: PathBuf,
    refreshed_at: Option<Instant>,
    stamp: Option<FileStamp>,
    names: HashMap<String, String>,
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

impl ThreadNameCache {
    /// 读取侧栏名称；相同文件未变化时复用解析结果。
    pub fn names(&mut self, path: &Path) -> &HashMap<String, String> {
        let stamp = match std::fs::metadata(path) {
            Ok(metadata) => Some(FileStamp {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                created: metadata.created().ok(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                eprintln!("无法读取会话名称索引 {}: {error}", path.display());
                None
            }
        };
        let reusable = self.path == path
            && stamp.is_some()
            && stamp == self.stamp
            && self
                .refreshed_at
                .is_some_and(|time| time.elapsed() < INDEX_MAX_AGE);
        if !reusable {
            self.path = path.to_path_buf();
            self.refreshed_at = Some(Instant::now());
            self.names = match stamp.as_ref().map(|_| std::fs::read(path)) {
                Some(Ok(bytes)) => parse_thread_names(&bytes),
                Some(Err(error)) => {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        eprintln!("无法读取会话名称索引 {}: {error}", path.display());
                    }
                    self.names.clear();
                    self.stamp = None;
                    return &self.names;
                }
                None => HashMap::new(),
            };
            self.stamp = stamp;
        }
        &self.names
    }
}

/// Codex 追加写名称，同一 ID 取最后一条非空名称，坏行跳过。
fn parse_thread_names(bytes: &[u8]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(record) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        let (Some(id), Some(name)) = (
            record.get("id").and_then(serde_json::Value::as_str),
            record
                .get("thread_name")
                .and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        let (id, name) = (id.trim(), name.trim());
        if !id.is_empty() && !name.is_empty() {
            names.insert(id.to_string(), name.to_string());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_fingerprint_expires_after_max_age() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("session_index.jsonl");
        std::fs::write(&path, r#"{"id":"a","thread_name":"old"}"#).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut cache = ThreadNameCache::default();
        assert_eq!(cache.names(&path).get("a").unwrap(), "old");

        std::fs::write(&path, r#"{"id":"a","thread_name":"new"}"#).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(cache.names(&path).get("a").unwrap(), "old");
        cache.refreshed_at = Some(Instant::now() - INDEX_MAX_AGE);
        assert_eq!(cache.names(&path).get("a").unwrap(), "new");
    }
}
