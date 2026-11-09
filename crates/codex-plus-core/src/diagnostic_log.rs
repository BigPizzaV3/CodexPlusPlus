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
/// 压缩失败后的首次退避，之后每次翻倍，封顶 `COMPACT_RETRY_MAX_DELAY`。
/// 失败了还每条日志都重试，等于每条日志都读 6 MB、写 6 MB 再等一轮替换重试（#2935）。
const COMPACT_RETRY_BASE_DELAY: Duration = Duration::from_secs(30);
const COMPACT_RETRY_MAX_DELAY: Duration = Duration::from_secs(10 * 60);
/// 旧版本用 `settings::atomic_write` 压缩，临时文件固定叫 `codex-plus.log.tmp`；
/// 进程在替换重试中途被结束时会留下 6 MB 的残留。修改时间超过这个值才清，
/// 避免和仍在运行的旧版本进程抢同一个文件。
const LEGACY_TEMP_STALE_AFTER: Duration = Duration::from_secs(60);

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
    /// 连续压缩失败的次数，决定下次再试前的退避时长；成功或换路径后清零。
    compact_failures: u32,
    max_bytes: u64,
    compacted_bytes: u64,
}

impl LogSink {
    fn new() -> Self {
        Self::with_limits(MAX_DIAGNOSTIC_LOG_BYTES, COMPACTED_DIAGNOSTIC_LOG_BYTES)
    }

    fn with_limits(max_bytes: u64, compacted_bytes: u64) -> Self {
        Self {
            path: PathBuf::new(),
            file: None,
            next_size_check: Instant::now(),
            compact_failures: 0,
            max_bytes,
            compacted_bytes,
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
        let path_changed = self.file.is_none() || self.path != path;
        if path_changed || Instant::now() >= self.next_size_check {
            self.reopen(path, path_changed)?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("诊断日志句柄未就绪"))?;
        // 一次 write_all 就是一次追加写：不做用户态缓冲，进程随时退出都不丢最近的记录。
        file.write_all(line)
    }

    /// 关句柄 → 视体积压缩 → 重开。
    ///
    /// 压缩会替换整个文件，自己的追加句柄必须先关：Windows 上替换一个仍被本进程打开的
    /// 文件会以 ERROR_ACCESS_DENIED 失败（#2935）。压缩失败只记一笔、按退避推迟下次检查，
    /// 不能挡住这条日志和之后的日志——以前失败会提前返回、不重开也不推迟检查，
    /// 之后每条日志都在全局锁里重做一遍读尾部 + 写临时文件 + 替换重试，拖住所有写日志的线程。
    fn reopen(&mut self, path: &Path, path_changed: bool) -> std::io::Result<()> {
        // 句柄已经被 take 走，flush 失败也没有可补救的。
        let _ = self.close();
        if path_changed {
            self.compact_failures = 0;
            remove_stale_legacy_temp(path);
        }
        let compact_result =
            compact_diagnostic_log(path, self.max_bytes, self.compacted_bytes);
        self.open(path)?;
        match compact_result {
            Ok(()) => self.compact_failures = 0,
            Err(error) => {
                self.compact_failures = self.compact_failures.saturating_add(1);
                let delay = compact_retry_delay(self.compact_failures);
                self.next_size_check = Instant::now() + delay;
                let record = DiagnosticRecord {
                    timestamp_ms: now_ms(),
                    pid: std::process::id(),
                    event: "diagnostic_log.compact_failed".to_string(),
                    detail: json!({
                        "error": error.to_string(),
                        "consecutive_failures": self.compact_failures,
                        "retry_in_ms": delay.as_millis() as u64,
                    }),
                };
                if let (Some(file), Ok(mut line)) =
                    (self.file.as_mut(), serde_json::to_string(&record))
                {
                    line.push('\n');
                    let _ = file.write_all(line.as_bytes());
                }
            }
        }
        Ok(())
    }
}

/// 第 n 次连续失败后的退避：30 秒起步，每次翻倍，封顶 10 分钟。
fn compact_retry_delay(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(16);
    COMPACT_RETRY_BASE_DELAY
        .saturating_mul(1u32 << shift)
        .min(COMPACT_RETRY_MAX_DELAY)
}

/// 清掉旧版本压缩中途留下的 `codex-plus.log.tmp`。只清确实放了一阵子的，失败不报错。
fn remove_stale_legacy_temp(path: &Path) {
    let Some(name) = path.file_name() else {
        return;
    };
    let mut legacy_name = name.to_os_string();
    legacy_name.push(".tmp");
    let legacy = path.with_file_name(legacy_name);
    let stale = std::fs::metadata(&legacy)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age >= LEGACY_TEMP_STALE_AFTER);
    if stale {
        let _ = std::fs::remove_file(&legacy);
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

fn compact_diagnostic_log(
    path: &Path,
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

    replace_log_contents(path, &tail)
}

/// 用 `tail` 整体替换日志。
///
/// 不走 `settings::atomic_write`：那条路径在 Windows 上用 `MoveFileExW` 覆盖，目标只要还被
/// 任何进程（manager、launcher、helper 各自常开一个追加句柄）打开就会 ERROR_ACCESS_DENIED，
/// 失败还要在锁里睡 1 秒多重试。`std::fs::rename` 在 Windows 上优先用 POSIX 语义的覆盖改名
/// （FILE_RENAME_FLAG_POSIX_SEMANTICS），别的进程以 FILE_SHARE_DELETE 打开着也能替换，
/// 它们的旧句柄写进被替换掉的文件，下一次定时重开（最多 5 秒）就接上新文件。
/// 临时文件名带 pid，多个进程同时压缩不会互相覆盖；日志不值得为断电再付一次 fsync。
fn replace_log_contents(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut temp_name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    temp_name.push(format!(".{}.compact.tmp", std::process::id()));
    let temp_path = path.with_file_name(temp_name);
    let result =
        std::fs::write(&temp_path, contents).and_then(|()| std::fs::rename(&temp_path, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
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

    /// 写一份超过上限的日志，内容是一行行可解析的记录。
    fn write_oversized_log(path: &Path, lines: usize) {
        let body = (0..lines)
            .map(|index| format!("{{\"old\":{index}}}\n"))
            .collect::<String>();
        std::fs::write(path, body).unwrap();
    }

    fn compact_temp_path(path: &Path) -> PathBuf {
        let mut name = path.file_name().unwrap().to_os_string();
        name.push(format!(".{}.compact.tmp", std::process::id()));
        path.with_file_name(name)
    }

    #[test]
    fn log_sink_backs_off_when_compaction_fails_and_keeps_appending() {
        // #2935：压缩失败时以前直接返回错误、不重开句柄也不推迟检查，于是每条日志都在全局锁里
        // 重做一遍读尾部 + 写临时文件 + 替换重试，主日志也不再前进。
        // 这里用「临时文件路径被一个目录占着」模拟替换失败。
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("codex-plus.log");
        write_oversized_log(&path, 20);
        std::fs::create_dir(compact_temp_path(&path)).unwrap();
        let mut sink = LogSink::with_limits(64, 32);

        let before = Instant::now();
        for index in 0..5 {
            sink.write_line(&path, format!("{{\"new\":{index}}}\n").as_bytes())
                .unwrap();
        }

        let contents = std::fs::read_to_string(&path).unwrap();
        // 压缩失败不能吞掉日志：原内容还在，新记录都追加上了。
        assert!(contents.starts_with("{\"old\":0}\n"), "{contents}");
        for index in 0..5 {
            assert!(contents.contains(&format!("{{\"new\":{index}}}")), "{contents}");
        }
        // 只试了一次，失败本身留了一条记录，下次检查推到退避之后。
        assert_eq!(contents.matches("diagnostic_log.compact_failed").count(), 1, "{contents}");
        assert_eq!(sink.compact_failures, 1);
        assert!(sink.next_size_check >= before + COMPACT_RETRY_BASE_DELAY);

        // 到期再试仍失败：退避翻倍。
        sink.next_size_check = Instant::now();
        let before = Instant::now();
        sink.write_line(&path, b"{\"new\":5}\n").unwrap();
        assert_eq!(sink.compact_failures, 2);
        assert!(sink.next_size_check >= before + COMPACT_RETRY_BASE_DELAY * 2);

        // 障碍消失后下一次检查就能压缩成功，计数清零。
        std::fs::remove_dir(compact_temp_path(&path)).unwrap();
        sink.next_size_check = Instant::now();
        sink.write_line(&path, b"{\"new\":6}\n").unwrap();
        assert_eq!(sink.compact_failures, 0);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("{\"old\":0}"), "{contents}");
        assert!(contents.ends_with("{\"new\":6}\n"), "{contents}");
    }

    #[test]
    fn compact_retry_delay_doubles_and_caps() {
        assert_eq!(compact_retry_delay(1), COMPACT_RETRY_BASE_DELAY);
        assert_eq!(compact_retry_delay(2), COMPACT_RETRY_BASE_DELAY * 2);
        assert_eq!(compact_retry_delay(3), COMPACT_RETRY_BASE_DELAY * 4);
        assert_eq!(compact_retry_delay(100), COMPACT_RETRY_MAX_DELAY);
        assert_eq!(compact_retry_delay(u32::MAX), COMPACT_RETRY_MAX_DELAY);
    }

    #[test]
    fn log_sink_periodic_compaction_rewrites_log_and_keeps_writing() {
        // 定时检查到期、日志超限：句柄先关、压缩、重开，之后的记录写进压缩后的新文件。
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("codex-plus.log");
        let mut sink = LogSink::with_limits(64, 32);
        sink.write_line(&path, b"{\"first\":0}\n").unwrap();
        write_oversized_log(&path, 20);

        sink.next_size_check = Instant::now();
        sink.write_line(&path, b"{\"after\":1}\n").unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.len() < 64, "{contents}");
        assert!(!contents.contains("{\"old\":0}"), "{contents}");
        assert!(contents.ends_with("{\"after\":1}\n"), "{contents}");
        assert!(!compact_temp_path(&path).exists());
    }

    #[test]
    fn compaction_replaces_log_while_another_writer_keeps_it_open() {
        // 同一份日志还被别的进程（manager / launcher / helper）常开着追加句柄。
        // Windows 上 MoveFileExW 覆盖这种文件稳定返回 ERROR_ACCESS_DENIED（#2935），
        // 压缩必须照样成功。macOS / Linux 上本来就成立，这条主要守 Windows CI。
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("codex-plus.log");
        write_oversized_log(&path, 20);
        let mut other_writer = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        other_writer.write_all(b"{\"other\":0}\n").unwrap();

        compact_diagnostic_log(&path, 64, 32).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.len() <= 32, "{contents}");
        assert!(contents.ends_with("{\"other\":0}\n"), "{contents}");
        assert!(!compact_temp_path(&path).exists());
        // 旧句柄写进被替换掉的文件，不报错也不污染新文件；定时重开后才接上新文件。
        other_writer.write_all(b"{\"other\":1}\n").unwrap();
    }

    #[test]
    fn log_sink_removes_stale_legacy_temp_on_open() {
        // 旧版本压缩中途被结束会留下 6 MB 的 codex-plus.log.tmp；放了一阵子的要清掉，
        // 刚写的（可能是仍在运行的旧版本进程在用）不动。
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("codex-plus.log");
        let legacy = temp.path().join("codex-plus.log.tmp");
        std::fs::write(&legacy, b"stale").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&legacy)
            .unwrap()
            .set_modified(SystemTime::now() - LEGACY_TEMP_STALE_AFTER * 2)
            .unwrap();

        LogSink::new().write_line(&path, b"{\"a\":1}\n").unwrap();
        assert!(!legacy.exists());

        std::fs::write(&legacy, b"fresh").unwrap();
        LogSink::new().write_line(&path, b"{\"b\":2}\n").unwrap();
        assert!(legacy.exists());
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
