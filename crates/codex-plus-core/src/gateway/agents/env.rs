//! 找 Agent 配置文件用的环境：用户目录、环境变量、各系统的应用数据目录。
//!
//! 命令层用 [`AgentEnv::current`]（真实的用户目录与进程环境变量）；测试用 [`AgentEnv::at`]
//! 指向临时目录，环境变量只认显式给的，不会读到开发机上的真实配置。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 当前运行的系统（决定应用数据目录的位置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentEnv {
    pub home: PathBuf,
    pub os: Os,
    /// 显式给的环境变量（测试用）；`process_env` 为 true 时没给的再读进程环境变量。
    vars: BTreeMap<String, String>,
    process_env: bool,
}

impl Default for AgentEnv {
    fn default() -> Self {
        Self::current()
    }
}

impl AgentEnv {
    /// 真实环境：用户目录 + 进程环境变量。
    pub fn current() -> Self {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            home,
            os: Os::current(),
            vars: BTreeMap::new(),
            process_env: true,
        }
    }

    /// 测试环境：用户目录指向 `home`，只认 [`AgentEnv::with_var`] 给的环境变量。
    pub fn at(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            os: Os::current(),
            vars: BTreeMap::new(),
            process_env: false,
        }
    }

    pub fn with_var(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.to_string(), value.to_string());
        self
    }

    pub fn with_os(mut self, os: Os) -> Self {
        self.os = os;
        self
    }

    /// 环境变量；空串当作没设置。
    pub fn var(&self, name: &str) -> Option<String> {
        self.vars
            .get(name)
            .cloned()
            .or_else(|| {
                self.process_env
                    .then(|| std::env::var(name).ok())
                    .flatten()
            })
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    /// 环境变量里的目录（`~` 开头按用户目录展开）。
    pub fn dir_var(&self, name: &str) -> Option<PathBuf> {
        self.var(name).map(|value| self.expand(&value))
    }

    /// 第一个设置了的目录环境变量，都没有时用 `fallback`。
    pub fn dir_or(&self, names: &[&str], fallback: impl FnOnce() -> PathBuf) -> PathBuf {
        names
            .iter()
            .find_map(|name| self.dir_var(name))
            .unwrap_or_else(fallback)
    }

    /// `~/…` 展开成用户目录下的路径；其余原样。
    pub fn expand(&self, value: &str) -> PathBuf {
        if value == "~" {
            return self.home.clone();
        }
        match value.strip_prefix("~/").or_else(|| value.strip_prefix("~\\")) {
            Some(rest) => self.home.join(rest),
            None => PathBuf::from(value),
        }
    }

    /// 用户目录下的路径。
    pub fn home_path(&self, relative: &str) -> PathBuf {
        self.home.join(relative)
    }

    /// XDG 配置目录：`$XDG_CONFIG_HOME`，否则 `~/.config`（多数 CLI 在 macOS 上也用它）。
    pub fn xdg_config(&self) -> PathBuf {
        self.dir_var("XDG_CONFIG_HOME")
            .unwrap_or_else(|| self.home.join(".config"))
    }

    /// XDG 数据目录：`$XDG_DATA_HOME`，否则 `~/.local/share`。
    pub fn xdg_data(&self) -> PathBuf {
        self.dir_var("XDG_DATA_HOME")
            .unwrap_or_else(|| self.home.join(".local").join("share"))
    }

    /// 桌面应用的配置目录：macOS `~/Library/Application Support`、Windows `%APPDATA%`、Linux XDG 配置目录。
    pub fn app_config(&self) -> PathBuf {
        match self.os {
            Os::Mac => self.home.join("Library").join("Application Support"),
            Os::Windows => self
                .dir_var("APPDATA")
                .unwrap_or_else(|| self.home.join("AppData").join("Roaming")),
            Os::Linux => self.xdg_config(),
        }
    }

    /// Windows 的 `%LOCALAPPDATA%`；其它系统同 [`AgentEnv::app_config`]。
    pub fn local_app_data(&self) -> PathBuf {
        match self.os {
            Os::Windows => self
                .dir_var("LOCALAPPDATA")
                .unwrap_or_else(|| self.home.join("AppData").join("Local")),
            _ => self.app_config(),
        }
    }

    /// 命令在 PATH（以及常见的安装目录）里找得到。
    pub fn has_binary(&self, binary: &str) -> bool {
        if binary.is_empty() {
            return false;
        }
        let names: Vec<String> = if self.os == Os::Windows {
            ["", ".exe", ".cmd", ".bat"]
                .iter()
                .map(|ext| format!("{binary}{ext}"))
                .collect()
        } else {
            vec![binary.to_string()]
        };
        let separator = if self.os == Os::Windows { ';' } else { ':' };
        let mut dirs: Vec<PathBuf> = self
            .var("PATH")
            .map(|path| {
                path.split(separator)
                    .filter(|dir| !dir.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        // 从访达启动的 App 拿不到 shell 的 PATH，补上常见的安装位置。
        for extra in [
            ".local/bin",
            ".bun/bin",
            ".npm-global/bin",
            ".cargo/bin",
            ".volta/bin",
            "go/bin",
        ] {
            dirs.push(self.home.join(extra));
        }
        if self.process_env {
            dirs.push(PathBuf::from("/opt/homebrew/bin"));
            dirs.push(PathBuf::from("/usr/local/bin"));
        }
        dirs.iter()
            .any(|dir| names.iter().any(|name| is_file(&dir.join(name))))
    }

    /// 用户目录下写成 `~/…` 的展示路径。
    pub fn display(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if !rest.as_os_str().is_empty() => {
                format!("~/{}", rest.to_string_lossy().replace('\\', "/"))
            }
            _ => path.to_string_lossy().to_string(),
        }
    }
}

fn is_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|meta| meta.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_only_sees_given_vars() {
        let env = AgentEnv::at("/home/u").with_var("FOO_DIR", "~/foo");
        assert_eq!(env.dir_var("FOO_DIR"), Some(PathBuf::from("/home/u/foo")));
        assert_eq!(env.var("PATH"), None);
        assert_eq!(env.var("HOME"), None);
    }

    #[test]
    fn app_config_follows_os() {
        let mac = AgentEnv::at("/h").with_os(Os::Mac);
        assert_eq!(
            mac.app_config(),
            PathBuf::from("/h/Library/Application Support")
        );
        let linux = AgentEnv::at("/h").with_os(Os::Linux);
        assert_eq!(linux.app_config(), PathBuf::from("/h/.config"));
        let windows = AgentEnv::at("/h")
            .with_os(Os::Windows)
            .with_var("APPDATA", "C:/Users/u/AppData/Roaming");
        assert_eq!(
            windows.app_config(),
            PathBuf::from("C:/Users/u/AppData/Roaming")
        );
    }

    #[test]
    fn display_uses_tilde() {
        let env = AgentEnv::at("/h");
        assert_eq!(env.display(Path::new("/h/.foo/a.json")), "~/.foo/a.json");
        assert_eq!(env.display(Path::new("/etc/x")), "/etc/x");
    }
}
