use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const APP_STATE_DIR: &str = ".codex-session-delete";
const JIYI_CODEX_HOME_DIR: &str = "codex-home";
const JIYI_UNIX_HOME_DIR: &str = "home";
const JIYI_BROWSER_USER_DATA_DIR: &str = "codex-client-user-data";
const SETTINGS_FILE: &str = "settings.json";
const SMS_PROVIDER_SETTINGS_FILE: &str = "sms-provider.json";
const LATEST_STATUS_FILE: &str = "latest-status.json";
const DIAGNOSTIC_LOG_FILE: &str = "codex-plus.log";

pub fn default_app_state_dir() -> PathBuf {
    if let Some(home_dir) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        return home_dir.join(APP_STATE_DIR);
    }

    PathBuf::from(APP_STATE_DIR)
}

pub fn default_jiyi_codex_home_dir() -> PathBuf {
    default_app_state_dir().join(JIYI_CODEX_HOME_DIR)
}

pub fn default_jiyi_unix_home_dir() -> PathBuf {
    default_app_state_dir().join(JIYI_UNIX_HOME_DIR)
}

pub fn default_jiyi_browser_user_data_dir() -> PathBuf {
    default_app_state_dir().join(JIYI_BROWSER_USER_DATA_DIR)
}

pub fn default_official_codex_home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

pub fn default_settings_path() -> PathBuf {
    if let Some(path) = settings_path_for_tests() {
        return path;
    }
    default_app_state_dir().join(SETTINGS_FILE)
}

pub fn default_sms_provider_settings_path() -> PathBuf {
    default_app_state_dir().join(SMS_PROVIDER_SETTINGS_FILE)
}

pub fn default_latest_status_path() -> PathBuf {
    default_app_state_dir().join(LATEST_STATUS_FILE)
}

pub fn default_diagnostic_log_path() -> PathBuf {
    default_app_state_dir().join(DIAGNOSTIC_LOG_FILE)
}

fn settings_path_for_tests() -> Option<PathBuf> {
    SETTINGS_PATH_FOR_TESTS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|path| path.clone())
}

static SETTINGS_PATH_FOR_TESTS: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

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
        let path = default_settings_path();

        assert!(path.ends_with(".codex-session-delete/settings.json"));
    }

    #[test]
    fn default_sms_provider_settings_path_uses_app_state_directory() {
        let path = default_sms_provider_settings_path();

        assert!(path.ends_with(".codex-session-delete/sms-provider.json"));
    }

    #[test]
    fn default_jiyi_codex_home_dir_is_separate_from_official_codex() {
        let path = default_jiyi_codex_home_dir();

        assert!(path.ends_with(".codex-session-delete/codex-home"));
        assert_ne!(path, default_official_codex_home_dir());
    }

    #[test]
    fn default_jiyi_unix_home_dir_is_separate_from_real_home() {
        let path = default_jiyi_unix_home_dir();

        assert!(path.ends_with(".codex-session-delete/home"));
        assert_ne!(path, default_official_codex_home_dir());
    }

    #[test]
    fn default_jiyi_browser_user_data_dir_is_separate_from_official_app_support() {
        let path = default_jiyi_browser_user_data_dir();

        assert!(path.ends_with(".codex-session-delete/codex-client-user-data"));
        assert!(!path.to_string_lossy().contains("Application Support/Codex"));
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
}
