//! 排队任务持有进程的存活标记（#2772）。
//!
//! 排队任务由提交它的进程定期续租，但电脑睡眠或进程卡顿超过租约后，只看时间会把仍存活进程
//! 自己的排队任务误判为失联。这里让持有进程在协作状态文件旁的目录里，独占锁住一个以本进程
//! 随机 id 命名的标记文件，整个进程生命周期都不释放：进程退出（包括崩溃、被强杀）时由操作系统
//! 释放锁，其他进程据此判断持有者是否还在，不受睡眠、卡顿和 pid 复用的影响。

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use fs2::FileExt;

const LOCK_EXTENSION: &str = "lock";

/// 协作状态文件对应的持有者标记目录（与状态文件同级）。
pub(super) fn owners_dir(store_path: &Path) -> PathBuf {
    let name = store_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "agent-collaboration".to_string());
    store_path.with_file_name(format!("{name}.owners"))
}

/// 本进程的持有者 id，进程内固定。
fn process_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// 本进程已锁住的标记文件（按标记目录区分），进程退出前一直持有。
fn held() -> &'static Mutex<HashMap<PathBuf, File>> {
    static HELD: OnceLock<Mutex<HashMap<PathBuf, File>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 本进程在 `dir` 下锁住标记文件（已锁住时直接返回），返回本进程的持有者 id。
/// 创建或加锁失败时返回 None：调用方不记录持有者，排队任务退回只按租约判断。
pub(super) fn acquire(dir: &Path) -> Option<String> {
    let mut held = held().lock().unwrap_or_else(|error| error.into_inner());
    if held.contains_key(dir) {
        return Some(process_token().to_string());
    }
    let file = create_locked(dir).ok()?;
    held.insert(dir.to_path_buf(), file);
    drop(held);
    // 首次登记时顺手清掉已退出进程留下的标记文件，避免越积越多。
    sweep_stale(dir);
    Some(process_token().to_string())
}

fn create_locked(dir: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(dir)?;
    let token = process_token();
    // 先在临时名下锁住再改名：其他进程只检查 .lock 文件，看到它时它一定已被锁住，
    // 不会在「刚创建、还没加锁」的瞬间被当成已退出的持有者删掉。
    let temporary = dir.join(format!("{token}.tmp"));
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&temporary)?;
    file.try_lock_exclusive()?;
    std::fs::rename(&temporary, dir.join(format!("{token}.{LOCK_EXTENSION}")))?;
    Ok(file)
}

/// `token` 对应的持有进程是否还在。标记文件还被锁着即仍存活；文件不存在或能被锁住即已退出
/// （顺手删掉该文件）。`token` 不是本模块生成的 id 或无法判断时返回 false，退回只按租约判断。
pub(super) fn is_alive(dir: &Path, token: &str) -> bool {
    // 只接受规范的 uuid，防止状态文件里的内容拼出目录外的路径。
    if uuid::Uuid::parse_str(token).map(|id| id.hyphenated().to_string()) != Ok(token.to_string()) {
        return false;
    }
    if token == process_token()
        && held()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(dir)
    {
        return true;
    }
    let path = dir.join(format!("{token}.{LOCK_EXTENSION}"));
    let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            drop(file);
            let _ = std::fs::remove_file(&path);
            false
        }
        Err(error) => error.kind() == fs2::lock_contended_error().kind(),
    }
}

fn sweep_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension() != Some(OsStr::new(LOCK_EXTENSION)) {
            continue;
        }
        if let Some(token) = path.file_stem().and_then(OsStr::to_str) {
            // 已退出进程的标记文件会在检查时删掉；仍存活的不动。
            let _ = is_alive(dir, token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 另一个进程持有的标记：锁着时算存活，锁释放（进程退出）后算已退出并删掉标记文件。
    #[test]
    fn foreign_owner_is_alive_only_while_its_marker_is_locked() {
        let temp = tempfile::tempdir().unwrap();
        let dir = owners_dir(&temp.path().join("collaboration.sqlite3"));
        std::fs::create_dir_all(&dir).unwrap();
        let token = uuid::Uuid::new_v4().to_string();
        let path = dir.join(format!("{token}.{LOCK_EXTENSION}"));
        let marker = File::create(&path).unwrap();
        marker.try_lock_exclusive().unwrap();
        assert!(is_alive(&dir, &token));
        assert!(path.exists());

        drop(marker);
        assert!(!is_alive(&dir, &token));
        assert!(!path.exists(), "已退出持有者的标记文件应被清掉");
        assert!(!is_alive(&dir, &token), "标记文件不存在时同样算已退出");
    }

    #[test]
    fn own_marker_stays_alive_and_invalid_tokens_are_never_alive() {
        let temp = tempfile::tempdir().unwrap();
        let dir = owners_dir(&temp.path().join("collaboration.sqlite3"));
        // 已退出进程留下的标记在本进程首次登记时被清掉。
        std::fs::create_dir_all(&dir).unwrap();
        let stale = dir.join(format!("{}.{LOCK_EXTENSION}", uuid::Uuid::new_v4()));
        File::create(&stale).unwrap();

        let token = acquire(&dir).unwrap();
        assert_eq!(acquire(&dir).as_deref(), Some(token.as_str()));
        assert!(is_alive(&dir, &token));
        assert!(dir.join(format!("{token}.{LOCK_EXTENSION}")).exists());
        assert!(!stale.exists());

        for invalid in ["", "../escape", "not-a-uuid"] {
            assert!(!is_alive(&dir, invalid), "{invalid}");
        }
        let braced = format!("{{{token}}}");
        assert!(!is_alive(&dir, &braced));
    }
}
