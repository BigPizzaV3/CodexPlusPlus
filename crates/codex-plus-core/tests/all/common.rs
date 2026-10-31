//! 各测试模块共享的辅助。

use std::sync::{Mutex, MutexGuard};

static GLOBAL_STATE: Mutex<()> = Mutex::new(());

/// 串行化会改进程级状态的用例：设置路径覆盖、诊断日志路径、环境变量等。
///
/// 合并前这些用例分散在不同测试程序里，靠进程隔离互不干扰；合并后同一进程内并行，
/// 各文件必须共用这一把锁。锁不可重入，同一用例里不要嵌套获取。
/// 中毒容忍：一个用例失败不应把后续用例一起带崩。
pub fn global_state_lock() -> MutexGuard<'static, ()> {
    GLOBAL_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct IsolatedTestState {
    _paths: IsolatedTestPaths,
    _lock: MutexGuard<'static, ()>,
}

impl IsolatedTestState {
    pub fn new() -> Self {
        let lock = global_state_lock();
        Self {
            _paths: IsolatedTestPaths::new(),
            _lock: lock,
        }
    }
}

struct IsolatedTestPaths {
    previous_settings: Option<std::path::PathBuf>,
    previous_log: Option<std::path::PathBuf>,
    _directory: tempfile::TempDir,
}

impl IsolatedTestPaths {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let previous_settings = codex_plus_core::paths::set_settings_path_for_tests(Some(
            directory.path().join("settings.json"),
        ));
        let previous_log = codex_plus_core::diagnostic_log::swap_diagnostic_log_path_for_tests(
            Some(directory.path().join("codex-plus.log")),
        );
        Self {
            previous_settings,
            previous_log,
            _directory: directory,
        }
    }
}

impl Drop for IsolatedTestPaths {
    fn drop(&mut self) {
        codex_plus_core::paths::set_settings_path_for_tests(self.previous_settings.take());
        codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(
            self.previous_log.take(),
        );
    }
}

#[test]
fn isolated_paths_restore_default_log_resolution() {
    let _lock = global_state_lock();
    let _outer = IsolatedTestPaths::new();
    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(None);
    {
        let isolated = IsolatedTestPaths::new();
        assert_eq!(
            codex_plus_core::diagnostic_log::diagnostic_log_path(),
            isolated._directory.path().join("codex-plus.log")
        );
    }
    assert_eq!(
        codex_plus_core::diagnostic_log::swap_diagnostic_log_path_for_tests(None),
        None
    );
    assert_eq!(
        codex_plus_core::diagnostic_log::diagnostic_log_path(),
        codex_plus_core::paths::default_diagnostic_log_path()
    );
}

#[test]
fn isolated_paths_restore_explicit_log_path() {
    let _lock = global_state_lock();
    let outer = IsolatedTestPaths::new();
    let previous_settings = codex_plus_core::paths::default_settings_path();
    let previous_log = codex_plus_core::diagnostic_log::diagnostic_log_path();
    {
        let isolated = IsolatedTestPaths::new();
        assert_ne!(
            codex_plus_core::diagnostic_log::diagnostic_log_path(),
            previous_log
        );
        assert_eq!(
            codex_plus_core::paths::default_settings_path(),
            isolated._directory.path().join("settings.json")
        );
    }
    assert_eq!(
        codex_plus_core::paths::default_settings_path(),
        previous_settings
    );
    assert_eq!(
        codex_plus_core::diagnostic_log::diagnostic_log_path(),
        previous_log
    );
    drop(outer);
}
