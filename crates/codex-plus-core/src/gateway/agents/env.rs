//! 找 Agent 配置文件用的环境：用户目录、环境变量、各系统的应用数据目录。
//!
//! 命令层用 [`AgentEnv::current`]（真实的用户目录与进程环境变量）；测试用 [`AgentEnv::at`]
//! 指向临时目录，环境变量只认显式给的，不会读到开发机上的真实配置。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

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
    /// 命令搜索目录的文件名索引：第一次 [`AgentEnv::has_binary`] 时建，克隆共用。
    /// 一次 Agent 列表要探测几十个命令，逐个 stat「目录 × 扩展名」在 Windows 上
    /// 是几万次失败的 CreateFile（每 10 秒的界面轮询都会触发），这里改成每个目录只列一次。
    binaries: Arc<OnceLock<BinaryIndex>>,
}

/// 搜索目录里的可执行文件名（Windows / macOS 按小写比较，与文件系统一致）。
#[derive(Debug, Default)]
struct BinaryIndex {
    /// 普通文件。
    files: HashSet<String>,
    /// 符号链接：查到时再跟随链接确认指向文件，避免建索引时 stat 整个 bin 目录的链接。
    links: HashMap<String, Vec<PathBuf>>,
    /// 列不出内容（无读权限等）但可能可以直接访问的目录：退回逐个 stat。
    unlisted: Vec<PathBuf>,
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
            binaries: Arc::default(),
        }
    }

    /// 测试环境：用户目录指向 `home`，只认 [`AgentEnv::with_var`] 给的环境变量。
    pub fn at(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            os: Os::current(),
            vars: BTreeMap::new(),
            process_env: false,
            binaries: Arc::default(),
        }
    }

    pub fn with_var(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.to_string(), value.to_string());
        // PATH 等可能变了，旧索引作废。
        self.binaries = Arc::default();
        self
    }

    pub fn with_os(mut self, os: Os) -> Self {
        self.os = os;
        self.binaries = Arc::default();
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
    ///
    /// 同一个环境里第一次调用时把各搜索目录各列一次建索引，之后的查询只查内存；
    /// 索引是那一刻的快照，要看到新装的命令就用新的 [`AgentEnv`]（命令层每次都是新建的）。
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
        let index = self.binaries.get_or_init(|| self.build_binary_index());
        names.iter().any(|name| {
            let key = self.binary_key(name);
            index.files.contains(&key)
                || index
                    .links
                    .get(&key)
                    .is_some_and(|paths| paths.iter().any(|path| is_file(path)))
                || index.unlisted.iter().any(|dir| is_file(&dir.join(name)))
        })
    }

    /// 文件名在索引里的写法：Windows / macOS 文件系统不区分大小写，按小写比较。
    fn binary_key(&self, name: &str) -> String {
        if self.os == Os::Linux {
            name.to_string()
        } else {
            name.to_lowercase()
        }
    }

    /// 把各搜索目录各列一次：普通文件记名字，符号链接留到查询时再跟随，
    /// 列不出内容的目录退回逐个 stat（与原先逐个探测的结果一致）。
    fn build_binary_index(&self) -> BinaryIndex {
        let mut index = BinaryIndex::default();
        let mut seen = HashSet::new();
        for dir in self.binary_search_dirs() {
            // PATH 里常有重复目录，同一个目录只列一次。
            if !seen.insert(self.binary_key(&dir.to_string_lossy())) {
                continue;
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                // 不存在或不是目录：逐个 stat 也找不到命令。
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    continue;
                }
                Err(_) => {
                    index.unlisted.push(dir);
                    continue;
                }
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let key = self.binary_key(&entry.file_name().to_string_lossy());
                if kind.is_file() {
                    index.files.insert(key);
                } else if kind.is_symlink() {
                    index.links.entry(key).or_default().push(entry.path());
                }
            }
        }
        index
    }

    /// 找命令的目录：PATH，加上从访达启动时拿不到 shell PATH 的常见安装位置。
    fn binary_search_dirs(&self) -> Vec<PathBuf> {
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
        dirs
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

    /// 命令放在 `~/.local/bin`：不经过 PATH 分隔符，各平台都能跑。
    fn local_bin(home: &Path) -> PathBuf {
        let dir = home.join(".local").join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn has_binary_finds_files_but_not_directories() {
        let temp = tempfile::tempdir().unwrap();
        let bin = local_bin(temp.path());
        std::fs::write(bin.join("agent-a"), "").unwrap();
        std::fs::create_dir(bin.join("agent-dir")).unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        assert!(env.has_binary("agent-a"));
        assert!(!env.has_binary("agent-dir"));
        assert!(!env.has_binary("missing"));
        assert!(!env.has_binary(""));
    }

    #[test]
    fn has_binary_on_windows_tries_extensions_case_insensitively() {
        let temp = tempfile::tempdir().unwrap();
        let bin = local_bin(temp.path());
        std::fs::write(bin.join("Gemini.CMD"), "").unwrap();
        std::fs::write(bin.join("qwen.exe"), "").unwrap();
        let windows = AgentEnv::at(temp.path()).with_os(Os::Windows);
        assert!(windows.has_binary("gemini"));
        assert!(windows.has_binary("qwen"));
        assert!(!windows.has_binary("kimi"));
        // 其它系统只认命令原名，不补扩展名。
        let linux = AgentEnv::at(temp.path()).with_os(Os::Linux);
        assert!(!linux.has_binary("qwen"));
    }

    #[test]
    fn has_binary_reads_path_entries() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let file = if cfg!(windows) { "tool.exe" } else { "tool" };
        std::fs::write(second.join(file), "").unwrap();
        // 不存在的目录和重复的目录都不影响结果。
        let missing = temp.path().join("missing");
        let path = std::env::join_paths([
            first.as_path(),
            missing.as_path(),
            second.as_path(),
            second.as_path(),
        ])
        .unwrap();
        let env = AgentEnv::at(temp.path()).with_var("PATH", path.to_str().unwrap());
        assert!(env.has_binary("tool"));
        assert!(!env.has_binary("other"));
    }

    #[test]
    fn has_binary_index_is_a_snapshot_per_env() {
        let temp = tempfile::tempdir().unwrap();
        let bin = local_bin(temp.path());
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        assert!(!env.has_binary("late"));
        std::fs::write(bin.join("late"), "").unwrap();
        // 同一个环境（及其克隆）沿用第一次建的索引，不再逐个探测文件系统。
        assert!(!env.has_binary("late"));
        assert!(!env.clone().has_binary("late"));
        // 新环境、或改了环境变量 / 系统之后重新建索引。
        assert!(AgentEnv::at(temp.path()).with_os(Os::Linux).has_binary("late"));
        assert!(env.clone().with_var("UNRELATED", "1").has_binary("late"));
        assert!(env.clone().with_os(Os::Linux).has_binary("late"));
    }

    #[cfg(unix)]
    #[test]
    fn has_binary_follows_symlinks_to_files_only() {
        let temp = tempfile::tempdir().unwrap();
        let bin = local_bin(temp.path());
        let package = temp.path().join("pkg");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("real"), "").unwrap();
        std::os::unix::fs::symlink(package.join("real"), bin.join("linked")).unwrap();
        std::os::unix::fs::symlink(&package, bin.join("dir-link")).unwrap();
        std::os::unix::fs::symlink(temp.path().join("gone"), bin.join("dangling")).unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        assert!(env.has_binary("linked"));
        assert!(!env.has_binary("dir-link"));
        assert!(!env.has_binary("dangling"));
    }

    #[cfg(unix)]
    #[test]
    fn has_binary_falls_back_to_stat_for_unlistable_dirs() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let bin = local_bin(temp.path());
        std::fs::write(bin.join("hidden"), "").unwrap();
        // 只能进、不能列的目录：列不出内容，但按名字仍然访问得到。
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o311)).unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        let found = env.has_binary("hidden");
        let missing = env.has_binary("absent");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(found);
        assert!(!missing);
    }
}
