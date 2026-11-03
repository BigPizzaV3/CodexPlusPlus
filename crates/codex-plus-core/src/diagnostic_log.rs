use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};

static TEST_LOG_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

// 上限压到 24 MB：桥与请求的高频事件叠一天就能写到 39 MB，超过这个量级之后再往下翻
// 也找不到有用的线索，只会让每次打开日志都变慢。
const MAX_DIAGNOSTIC_LOG_BYTES: u64 = 24 * 1024 * 1024;
const COMPACTED_DIAGNOSTIC_LOG_BYTES: u64 = 6 * 1024 * 1024;
/// 文件体积只需要隔一会儿看一次，不必每行 stat。
const LOG_SIZE_CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
struct DiagnosticRecord {
    timestamp_ms: u64,
    pid: u32,
    event: String,
    detail: Value,
}

/// 进程内复用的写句柄。
///
/// 原来每条日志都 open + write + close 一次，峰值 26 行/秒时等于每秒上百次系统调用，
/// 全靠内核扛着；日志本身也会长到 39 MB。这里把句柄留在进程里，按块刷盘：
/// 一次 write 落一整批行，多进程同时 O_APPEND 也不会把两行挤在一起（整块原子追加）。
struct LogSink {
    path: PathBuf,
    file: Option<std::fs::File>,
    next_size_check: Instant,
}

impl LogSink {
    fn new() -> Self {
        Self {
            path: PathBuf::new(),
            file: None,
            next_size_check: Instant::now(),
        }
    }

    /// 关掉当前句柄。换路径、清空、压缩前都要走这里。
    fn close(&mut self) -> std::io::Result<()> {
        self.path = PathBuf::new();
        match self.file.take() {
            Some(mut file) => file.flush(),
            None => Ok(()),
        }
    }

    /// 句柄常开、写入直落内核页缓存，没有需要单独刷的缓冲区；保留这个入口是为了让
    /// 调用方（清空日志、切换测试路径、进程退出前）有统一的语义。
    fn flush_now(&mut self) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }

    fn open(&mut self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        self.file = Some(file);
        self.path = path.to_path_buf();
        self.next_size_check = Instant::now() + LOG_SIZE_CHECK_INTERVAL;
        Ok(())
    }

    /// 追加一行（含结尾换行）。`line` 必须带 `\n`。
    fn write_line(&mut self, path: &Path, line: &[u8]) -> std::io::Result<()> {
        let now = Instant::now();
        if self.file.is_none() || self.path != path {
            self.close()?;
            let owned = path.to_path_buf();
            compact_diagnostic_log_if_needed(&owned)?;
            self.open(path)?;
        } else if now >= self.next_size_check {
            // 压缩会重写整个文件，句柄必须先落下再重开。
            self.flush_now()?;
            let owned = path.to_path_buf();
            compact_diagnostic_log_if_needed(&owned)?;
            self.open(path)?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("诊断日志句柄未就绪"))?;
        // 一次 write_all 就是一次追加写：不做用户态缓冲，进程随时退出都不丢最近的记录。
        file.write_all(line)
    }
}

fn log_sink() -> &'static Mutex<LogSink> {
    LOG_SINK.get_or_init(|| Mutex::new(LogSink::new()))
}

static LOG_SINK: OnceLock<Mutex<LogSink>> = OnceLock::new();

pub fn append_diagnostic_log(event: &str, detail: impl Serialize) -> std::io::Result<()> {
    let path = diagnostic_log_path();
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
    // 记录和换行在同一次写入里：`writeln!` 会分两次写，并发写日志时别的记录会插进来，
    // 两条挤成一行、谁都解析不了。
    let mut buffer = String::with_capacity(line.len() + 1);
    buffer.push_str(&line);
    buffer.push('\n');
    let mut sink = log_sink()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    sink.write_line(&path, buffer.as_bytes())
}

/// 把缓冲区里的日志立刻落盘。清空日志、切换测试路径、进程退出前用。
pub fn flush_diagnostic_log() -> std::io::Result<()> {
    let mut sink = log_sink()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    sink.flush_now()
}

pub fn clear_diagnostic_log() -> std::io::Result<()> {
    let path = diagnostic_log_path();
    clear_diagnostic_log_path(&path)
}

fn clear_diagnostic_log_path(path: &Path) -> std::io::Result<()> {
    // 句柄不关掉就只能删到一个还开着的 inode，之后的日志会写进黑洞。
    {
        let mut sink = log_sink()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sink.close()?;
    }
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
    swap_diagnostic_log_path_for_tests(path);
}

#[doc(hidden)]
pub fn swap_diagnostic_log_path_for_tests(path: Option<PathBuf>) -> Option<PathBuf> {
    let test_path = TEST_LOG_PATH.get_or_init(|| Mutex::new(None));
    let replaced = {
        let mut guard = test_path.lock().expect("test log path lock poisoned");
        std::mem::replace(&mut *guard, path.clone())
    };
    // 旧句柄先关掉，测试随后读新路径时才看得到完整内容。
    let mut sink = log_sink()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = sink.close();
    replaced
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn compact_diagnostic_log_if_needed(path: &PathBuf) -> std::io::Result<()> {
    compact_diagnostic_log(
        path,
        MAX_DIAGNOSTIC_LOG_BYTES,
        COMPACTED_DIAGNOSTIC_LOG_BYTES,
    )
}

fn compact_diagnostic_log(
    path: &PathBuf,
    max_bytes: u64,
    compacted_bytes: u64,
) -> std::io::Result<()> {
    let len = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if len <= max_bytes {
        return Ok(());
    }

    let keep = compacted_bytes.min(len);
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(len - keep))?;
    let mut tail = Vec::with_capacity(keep as usize);
    file.read_to_end(&mut tail)?;
    drop(file);
    if len > keep {
        if let Some(pos) = tail.iter().position(|byte| *byte == b'\n') {
            tail.drain(..=pos);
        }
    }

    crate::settings::atomic_write(path, &tail).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_diagnostic_log_keeps_tail_and_drops_partial_first_line() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("codex-plus.log");
        std::fs::write(&path, "line-1\nline-2\nline-3\nline-4\n").unwrap();

        compact_diagnostic_log(&path, 12, 16).unwrap();

        let contents = std::fs::read_to_string(path).unwrap();
        assert_eq!(contents, "line-3\nline-4\n");
    }

    #[test]
    fn clear_diagnostic_log_ignores_missing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing.log");

        clear_diagnostic_log_path(&path).unwrap();
    }

    #[test]
    fn log_sink_keeps_each_record_on_its_own_line() {
        // 句柄复用之后仍然是一条记录一次追加写：行不能被攒在一起、也不能被拆开，
        // 否则并发写日志时会拼出谁都不认识的行。
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("codex-plus.log");
        let mut sink = LogSink::new();
        for index in 0..10 {
            sink.write_line(&path, format!("{{\"i\":{index}}}\n").as_bytes())
                .unwrap();
        }

        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 10);
        assert!(
            contents
                .lines()
                .all(|line| serde_json::from_str::<Value>(line).is_ok()),
            "{contents}"
        );
    }

    #[test]
    fn log_sink_reopens_when_path_changes() {
        // 测试覆盖路径来回切换时，旧句柄必须落下，新路径要能立刻写进去。
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.log");
        let second = temp.path().join("second.log");
        let mut sink = LogSink::new();
        sink.write_line(&first, b"{\"a\":1}\n").unwrap();
        sink.write_line(&second, b"{\"b\":2}\n").unwrap();

        assert_eq!(std::fs::read_to_string(&first).unwrap(), "{\"a\":1}\n");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "{\"b\":2}\n");
    }
}
