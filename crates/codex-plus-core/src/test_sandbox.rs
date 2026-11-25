//! 测试进程的家目录沙箱：测试不许碰用户的真实目录。
//!
//! 很多代码走默认路径（`~/.claude/settings.json`、`~/.codex`、`~/.codex-session-delete`、
//! `~/.config/Codex++` 等），测试里只要有一处忘了换成临时目录，就会改到用户正在用的配置
//! （出过事：Claude 的配置被改成测试里的 DeepSeek，正在用 Claude 的会话直接断了）。
//!
//! 所以每个测试程序在 `main` 之前（`#[ctor]`，此时还是单线程）调用 [`enter`]：
//! 把 `HOME` 等指向这个进程自己的临时目录，清掉直接指定配置目录的变量（`CLAUDE_CONFIG_DIR`、
//! `CODEX_HOME`、`CODEX_PLUS_STATE_DIR`……），默认路径于是全落在沙箱里。测试自己再设这些变量照常有效。
//!
//! 新增测试程序（`src` 下的单元测试根、`tests/*.rs`）都要接上，`tests/all/test_sandbox.rs` 的契约测试会查。
//! Windows 上 `directories` 走系统的已知文件夹接口、不看环境变量：所以代码里的用户目录统一走
//! [`crate::user_dirs`]，它看到 [`active_home`] 就改按沙箱推导（契约测试也查没有绕过它的）。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static SANDBOX: OnceLock<PathBuf> = OnceLock::new();

/// 沙箱根目录，传给子进程：子进程看到它（且 HOME 在里面）就知道自己已经在沙箱里。
const MARKER: &str = "CODEX_PLUS_TEST_SANDBOX";

/// 直接指定某个配置 / 状态目录的变量：清掉，让它们按沙箱里的 HOME 推导。
const CLEARED: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_USER_DATA_DIR",
    "CLAUDE_MOM_STATE_DIR",
    "CODEX_HOME",
    "CODEX_SQLITE_HOME",
    "CODEX_PLUS_STATE_DIR",
    "CODEX_PLUS_SHARED_SKILLS_DIR",
    "CODEX_ELECTRON_USER_DATA_PATH",
    "GROK_HOME",
    "CURSOR_CONFIG_DIR",
    "FACTORY_HOME_OVERRIDE",
    "CPP_PROVIDER_SCAN_HOME",
    "PI_CONFIG_DIR",
    "STEPCODE_CONFIG_DIR",
    "SNOW_CONFIG_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
];

/// 进入沙箱（幂等）。只该在测试程序的 `#[ctor]` 里调用：改环境变量要求此时没有别的线程。
pub fn enter() -> &'static Path {
    SANDBOX.get_or_init(|| {
        // 测试把自己再起一遍当子进程（带上专门设的 CODEX_PLUS_STATE_DIR 等）时，父进程已经在沙箱里了：
        // 原样继承，不再清变量，否则父进程给子进程设的目录会被清掉。
        if let Some(inherited) = std::env::var_os(MARKER).map(PathBuf::from)
            && std::env::var_os("HOME").is_some_and(|home| Path::new(&home).starts_with(&inherited))
        {
            return inherited;
        }
        let root = std::env::temp_dir().join(format!("codex-plus-test-home-{}", std::process::id()));
        let home = root.join("home");
        let _ = std::fs::create_dir_all(&home);
        // SAFETY: 只在 main 之前的 #[ctor] 里调用，进程里还没有其他线程读写环境变量。
        unsafe {
            // 子进程（cargo、rustup 代理）还要找到真实的工具链：没显式设过就按真实家目录先钉住。
            if let Some(real) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
                let real = PathBuf::from(real);
                if std::env::var_os("RUSTUP_HOME").is_none() {
                    std::env::set_var("RUSTUP_HOME", real.join(".rustup"));
                }
                if std::env::var_os("CARGO_HOME").is_none() {
                    std::env::set_var("CARGO_HOME", real.join(".cargo"));
                }
            }
            for name in CLEARED {
                std::env::remove_var(name);
            }
            std::env::set_var("HOME", &home);
            std::env::set_var(MARKER, &root);
            if cfg!(windows) {
                std::env::set_var("USERPROFILE", &home);
                std::env::set_var("APPDATA", home.join("AppData").join("Roaming"));
                std::env::set_var("LOCALAPPDATA", home.join("AppData").join("Local"));
            }
        }
        root
    })
}

/// 沙箱里的家目录：本进程（或起它的测试进程）进了沙箱、家目录也还在沙箱根下时返回，否则 None。
///
/// 按环境变量判断而不是只看本进程有没有调过 [`enter`]：测试起的子进程（launcher、helper 等非测试程序）
/// 也要落在沙箱里。正式运行时没有这个标记，开销只是读一次环境变量。
pub fn active_home() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os(MARKER)?);
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    home.starts_with(&root).then_some(home)
}

/// 当前进程的沙箱根目录（没进沙箱时为 None）。
pub fn root() -> Option<&'static Path> {
    SANDBOX.get().map(PathBuf::as_path)
}
