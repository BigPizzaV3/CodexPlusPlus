use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const APP_STATE_DIR: &str = ".codex-session-delete";
const SETTINGS_FILE: &str = "settings.json";
const SETTINGS_PATH_POINTER_FILE: &str = "settings.path";
const LATEST_STATUS_FILE: &str = "latest-status.json";
const DIAGNOSTIC_LOG_FILE: &str = "codex-plus.log";
const PENDING_PROVIDER_IMPORT_FILE: &str = "pending-provider-import.json";
const PENDING_SESSION_SHARE_FILE: &str = "pending-session-share.txt";
const PENDING_REMOTE_CONTROL_RECOVERY_FILE: &str = "pending-remote-control-recovery.json";
const SKILLS_STATE_FILE: &str = "skills.json";
const SKILLS_DIR: &str = "skills";
const SKILL_BACKUPS_DIR: &str = "skill-backups";
const PENDING_MANAGER_NAVIGATION_FILE: &str = "pending-manager-navigation.json";

/// 显式指定状态目录的环境变量（Windows 多开隔离的必需项）。
///
/// 多开实例各自需要独立的 `settings.json` / `latest-status.json` / 诊断日志，
/// 早先靠给 worker 换 `HOME` 实现。该做法只在 Unix 成立：`directories` 在
/// Windows 上走 `SHGetKnownFolderPath(FOLDERID_Profile)`，**完全不读 `HOME`**
/// （见 `directories-6.0.0/src/win.rs`），于是 worker 读写的仍是主实例的状态目录，
/// 多开专用设置（含 `--user-data-dir`）从来没有被读取过。
///
/// 因此这里提供平台无关的显式覆盖：设置后该进程（及其子进程）的状态目录整体
/// 挪到指定位置，不依赖任何平台特有的家目录语义。
pub const STATE_DIR_ENV: &str = "CODEX_PLUS_STATE_DIR";

/// 解析显式状态目录覆盖；未设置或设置为空时返回 `None`。
pub fn state_dir_override_from(value: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let value = value?;
    let text = value.to_string_lossy();
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

pub fn state_dir_override() -> Option<PathBuf> {
    state_dir_override_from(std::env::var_os(STATE_DIR_ENV).as_deref())
}

pub fn default_app_state_dir() -> PathBuf {
    // 显式覆盖优先于家目录推导：多开实例与宿主 set 的路径必须权威，
    // 否则 Windows 上 HOME 失效会把实例状态写回主实例目录。
    if let Some(state_dir) = state_dir_override() {
        return state_dir;
    }

    default_state_base_dir()
}

fn default_state_base_dir() -> PathBuf {
    if let Some(home_dir) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        return home_dir.join(APP_STATE_DIR);
    }
    PathBuf::from(APP_STATE_DIR)
}

pub fn settings_path_pointer_path() -> PathBuf {
    default_state_base_dir().with_file_name(SETTINGS_PATH_POINTER_FILE)
}

fn read_settings_path_pointer() -> Option<PathBuf> {
    let path = settings_path_pointer_path();
    let contents = fs::read_to_string(path).ok()?;
    let target = PathBuf::from(contents.trim());
    target.is_absolute().then_some(target)
}

pub fn set_settings_path_pointer(target: Option<&Path>) -> std::io::Result<()> {
    // 指针文件是全局的，而设置了状态目录覆盖的多开实例读取时会忽略它：
    // 在多开实例里写入只会改掉主实例的配置路径，自己却不生效。
    if state_dir_override().is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "多开实例使用独立的状态目录，不支持修改配置文件路径；请在主实例中修改",
        ));
    }
    let pointer = settings_path_pointer_path();
    match target {
        Some(target) => {
            if let Some(parent) = pointer.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(pointer, target.to_string_lossy().as_bytes())
        }
        None => match fs::remove_file(pointer) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
    }
}

pub fn default_settings_path() -> PathBuf {
    if let Some(path) = settings_path_for_tests() {
        return path;
    }
    if state_dir_override().is_none()
        && let Some(path) = read_settings_path_pointer()
    {
        return path;
    }
    default_app_state_dir().join(SETTINGS_FILE)
}

pub fn default_latest_status_path() -> PathBuf {
    default_app_state_dir().join(LATEST_STATUS_FILE)
}

pub fn default_diagnostic_log_path() -> PathBuf {
    default_app_state_dir().join(DIAGNOSTIC_LOG_FILE)
}

pub fn default_pending_provider_import_path() -> PathBuf {
    default_app_state_dir().join(PENDING_PROVIDER_IMPORT_FILE)
}

pub fn default_pending_session_share_path() -> PathBuf {
    default_app_state_dir().join(PENDING_SESSION_SHARE_FILE)
}

pub fn default_pending_remote_control_recovery_path() -> PathBuf {
    default_app_state_dir().join(PENDING_REMOTE_CONTROL_RECOVERY_FILE)
}

/// Skills 的「单一事实来源」目录。已安装的 skill 目录都放这里，
/// 启用时再软链到 `$CODEX_HOME/skills/<id>`，停用只删链接、源目录留着。
pub fn default_skills_source_dir() -> PathBuf {
    default_app_state_dir().join(SKILLS_DIR)
}

pub fn default_skills_state_path() -> PathBuf {
    default_app_state_dir().join(SKILLS_STATE_FILE)
}

/// 卸载 skill 时把源目录整体移到这里，方便反悔。不自动轮转删除。
pub fn default_skill_backups_dir() -> PathBuf {
    default_app_state_dir().join(SKILL_BACKUPS_DIR)
}

pub fn default_pending_manager_navigation_path() -> PathBuf {
    default_app_state_dir().join(PENDING_MANAGER_NAVIGATION_FILE)
}

fn settings_path_for_tests() -> Option<PathBuf> {
    SETTINGS_PATH_FOR_TESTS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|path| path.clone())
}

static SETTINGS_PATH_FOR_TESTS: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

#[cfg(test)]
static SETTINGS_PATH_TEST_GUARD: OnceLock<Mutex<()>> = OnceLock::new();

#[cfg(test)]
pub(crate) fn settings_path_test_guard() -> std::sync::MutexGuard<'static, ()> {
    SETTINGS_PATH_TEST_GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap()
}

pub fn set_settings_path_for_tests(path: Option<PathBuf>) -> Option<PathBuf> {
    SETTINGS_PATH_FOR_TESTS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|mut current| std::mem::replace(&mut *current, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_path_uses_app_state_directory() {
        let _guard = settings_path_test_guard();
        let path = default_settings_path();

        assert!(path.ends_with(".codex-session-delete/settings.json"));
    }

    #[test]
    fn settings_path_pointer_accepts_only_absolute_targets() {
        let dir = tempfile::tempdir().unwrap();
        let pointer = dir.path().join("settings.path");
        // 用 tempdir 生成绝对路径；写死 /tmp/... 在 Windows 上没有盘符，不算绝对路径
        let target = dir.path().join("codex-settings.json");
        fs::write(&pointer, format!("{}\n", target.display())).unwrap();
        let absolute = fs::read_to_string(&pointer).unwrap();
        assert!(PathBuf::from(absolute.trim()).is_absolute());

        fs::write(&pointer, "relative/settings.json\n").unwrap();
        assert!(!PathBuf::from(fs::read_to_string(pointer).unwrap().trim()).is_absolute());
    }

    #[test]
    fn default_latest_status_path_uses_app_state_directory() {
        let path = default_latest_status_path();

        assert!(path.ends_with(".codex-session-delete/latest-status.json"));
    }

    #[test]
    fn default_diagnostic_log_path_uses_app_state_directory() {
        let path = default_diagnostic_log_path();

        assert!(path.ends_with(".codex-session-delete/codex-plus.log"));
    }

    #[test]
    fn default_pending_provider_import_path_uses_app_state_directory() {
        let path = default_pending_provider_import_path();

        assert!(path.ends_with(".codex-session-delete/pending-provider-import.json"));
    }

    #[test]
    fn default_pending_session_share_path_uses_app_state_directory() {
        let path = default_pending_session_share_path();

        assert!(path.ends_with(".codex-session-delete/pending-session-share.txt"));
    }

    #[test]
    fn default_pending_remote_control_recovery_path_uses_app_state_directory() {
        let path = default_pending_remote_control_recovery_path();

        assert!(path.ends_with(".codex-session-delete/pending-remote-control-recovery.json"));
    }

    #[test]
    fn default_pending_manager_navigation_path_uses_app_state_directory() {
        let path = default_pending_manager_navigation_path();

        assert!(path.ends_with(".codex-session-delete/pending-manager-navigation.json"));
    }

    // 显式状态目录覆盖是 Windows 多开隔离的前提：平台无关的环境变量，
    // 不依赖 HOME（Windows 上 directories 走 SHGetKnownFolderPath，不读 HOME）。
    #[test]
    fn state_dir_override_accepts_non_empty_paths_only() {
        assert_eq!(
            state_dir_override_from(Some(std::ffi::OsStr::new("/tmp/instance-state"))),
            Some(PathBuf::from("/tmp/instance-state"))
        );
        assert_eq!(
            state_dir_override_from(Some(std::ffi::OsStr::new("  /tmp/padded  "))),
            Some(PathBuf::from("/tmp/padded"))
        );
        assert_eq!(
            state_dir_override_from(Some(std::ffi::OsStr::new(""))),
            None
        );
        assert_eq!(
            state_dir_override_from(Some(std::ffi::OsStr::new("   "))),
            None
        );
        assert_eq!(state_dir_override_from(None), None);
    }

    #[test]
    fn default_app_state_dir_honours_explicit_override() {
        // `CODEX_PLUS_STATE_DIR` 是进程级环境变量，用作用域守卫保证并行测试看不到它。
        let _guard = settings_path_test_guard();
        let scope = StateDirEnvScope::set("/tmp/codex-plus-instance-state");

        assert_eq!(
            default_app_state_dir(),
            PathBuf::from("/tmp/codex-plus-instance-state")
        );

        drop(scope);
        assert!(default_app_state_dir().ends_with(".codex-session-delete"));
    }

    /// 测试用：临时设置 `CODEX_PLUS_STATE_DIR`，Drop 时恢复原值。
    struct StateDirEnvScope {
        previous: Option<std::ffi::OsString>,
    }

    impl StateDirEnvScope {
        fn set(value: &str) -> Self {
            let previous = std::env::var_os(STATE_DIR_ENV);
            unsafe { std::env::set_var(STATE_DIR_ENV, value) };
            Self { previous }
        }
    }

    impl Drop for StateDirEnvScope {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(previous) => unsafe { std::env::set_var(STATE_DIR_ENV, previous) },
                None => unsafe { std::env::remove_var(STATE_DIR_ENV) },
            }
        }
    }
}
