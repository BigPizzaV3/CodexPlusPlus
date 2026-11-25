//! 用户目录（家目录、配置 / 数据目录、桌面）的唯一入口：代码里不直接用 `directories`。
//!
//! 平常就是 `directories` 的结果。测试沙箱里（见 [`crate::test_sandbox`]）改按沙箱设的环境变量推导：
//! Windows 上 `directories` 走系统的已知文件夹接口（`SHGetKnownFolderPath`），完全不看 `HOME` /
//! `USERPROFILE` / `APPDATA`，不走这里的话测试照样会读写用户真实的家目录。
//! 契约测试 `test_sandbox::nothing_bypasses_user_dirs` 会查有没有绕过这里的。

use std::path::{Path, PathBuf};

/// 与 `directories::BaseDirs` 同名同用法（只带本仓库用到的几个目录）。
#[derive(Debug, Clone)]
pub struct BaseDirs {
    home: PathBuf,
    config: PathBuf,
    data: PathBuf,
    data_local: PathBuf,
}

impl BaseDirs {
    pub fn new() -> Option<Self> {
        if let Some(home) = crate::test_sandbox::active_home() {
            return Some(Self::sandboxed(home));
        }
        let dirs = directories::BaseDirs::new()?;
        Some(Self {
            home: dirs.home_dir().to_path_buf(),
            config: dirs.config_dir().to_path_buf(),
            data: dirs.data_dir().to_path_buf(),
            data_local: dirs.data_local_dir().to_path_buf(),
        })
    }

    /// 沙箱里各平台的默认布局，全按沙箱的家目录 / 环境变量推导。
    fn sandboxed(home: PathBuf) -> Self {
        let env_dir = |name: &str, fallback: PathBuf| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or(fallback)
        };
        if cfg!(windows) {
            let roaming = env_dir("APPDATA", home.join("AppData").join("Roaming"));
            let local = env_dir("LOCALAPPDATA", home.join("AppData").join("Local"));
            Self { config: roaming.clone(), data: roaming, data_local: local, home }
        } else if cfg!(target_os = "macos") {
            let support = home.join("Library").join("Application Support");
            Self { config: support.clone(), data: support.clone(), data_local: support, home }
        } else {
            let config = env_dir("XDG_CONFIG_HOME", home.join(".config"));
            let data = env_dir("XDG_DATA_HOME", home.join(".local").join("share"));
            Self { config, data: data.clone(), data_local: data, home }
        }
    }

    pub fn home_dir(&self) -> &Path {
        &self.home
    }

    pub fn config_dir(&self) -> &Path {
        &self.config
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn data_local_dir(&self) -> &Path {
        &self.data_local
    }
}

/// 用户的桌面目录（Windows 先问系统的已知文件夹，再退回 `directories`）。沙箱里是沙箱家目录下的 `Desktop`。
pub fn desktop_dir() -> Option<PathBuf> {
    if let Some(home) = crate::test_sandbox::active_home() {
        return Some(home.join("Desktop"));
    }
    #[cfg(windows)]
    let native = crate::windows_integration::desktop_dir();
    #[cfg(not(windows))]
    let native: Option<PathBuf> = None;
    native.or_else(|| directories::UserDirs::new().and_then(|dirs| dirs.desktop_dir().map(PathBuf::from)))
}
