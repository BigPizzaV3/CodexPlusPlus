//! 产品变体：同一份代码构建出 Codex++ 与 ClaudeMom 两个程序。
//!
//! ClaudeMom 只做 Claude Desktop 完整功能模式，由单独的二进制 `claude-mom`
//! （`src/claude_mom.rs`）在进程最开始调用 [`init_claude_mom`] 标记出来；
//! Codex++ 的 `codex-plus-plus` 不调用任何初始化，行为与以前完全一致。
//!
//! ClaudeMom 与 Codex++ 完全独立，可以同时运行：
//! - 状态目录默认 `~/.claude-mom`（设置、上游列表、Claude 本地服务都在里面）；
//! - 界面是自己的一页（前端 `claude-mom.html`），不经 Codex++ 网关，直连用户配置的 Base URL + Key；
//! - 单实例锁、启动器锁端口在 Codex++ 的基础上平移 [`PORT_SHIFT`]；
//! - 本地协议代理首次启动时定在 [`CLAUDE_MOM_PROTOCOL_PROXY_PORT`]，之后照常持久化与迁移。
//!
//! 已经显式设置的环境变量（本地开发 bundle 的 `LSEnvironment`、多开 worker 等）一律优先，不覆盖。

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    CodexPlusPlus,
    ClaudeMom,
}

static FLAVOR: OnceLock<Flavor> = OnceLock::new();

/// ClaudeMom 默认状态目录（相对家目录）。
pub const CLAUDE_MOM_STATE_DIR: &str = ".claude-mom";
/// ClaudeMom 的各个固定端口相对 Codex++ 的平移量。
pub const PORT_SHIFT: u16 = 100;
/// ClaudeMom 首次启动时使用的本地协议代理端口（Codex++ 是 57321）。
pub const CLAUDE_MOM_PROTOCOL_PROXY_PORT: u16 =
    codex_plus_core::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT + PORT_SHIFT;

/// 当前进程的产品变体；没初始化过就是 Codex++。
pub fn current() -> Flavor {
    FLAVOR.get().copied().unwrap_or(Flavor::CodexPlusPlus)
}

pub fn is_claude_mom() -> bool {
    current() == Flavor::ClaudeMom
}

/// 窗口标题、托盘提示等处显示的程序名。
pub fn app_name() -> &'static str {
    match current() {
        Flavor::CodexPlusPlus => "Codex++",
        Flavor::ClaudeMom => "ClaudeMom",
    }
}

/// 主窗口：加载的页面、默认尺寸、最小尺寸。ClaudeMom 是自己的小工具页面。
pub struct MainWindowSpec {
    pub url: &'static str,
    pub size: (f64, f64),
    pub min_size: (f64, f64),
}

pub fn main_window() -> MainWindowSpec {
    match current() {
        Flavor::CodexPlusPlus => MainWindowSpec {
            url: "/index.html",
            size: (1180.0, 820.0),
            min_size: (960.0, 720.0),
        },
        Flavor::ClaudeMom => MainWindowSpec {
            url: "/claude-mom.html",
            size: (860.0, 600.0),
            min_size: (720.0, 520.0),
        },
    }
}

/// 只能在 `main` 最开始、还没有起任何线程时调用（要改进程环境变量）。
pub fn init_claude_mom() {
    let _ = FLAVOR.set(Flavor::ClaudeMom);
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    for (name, value) in claude_mom_env_defaults(&|name| std::env::var_os(name).is_some(), home) {
        // SAFETY: 进程入口处调用，此时只有主线程。
        unsafe { std::env::set_var(name, value) };
    }
    // 状态目录已经定下来，协议代理端口按它的持久化文件走；显式指定了端口就不用种。
    if std::env::var_os("CODEX_PLUS_PROTOCOL_PROXY_PORT").is_none()
        && let Err(error) = codex_plus_core::protocol_proxy::seed_default_protocol_proxy_port(
            CLAUDE_MOM_PROTOCOL_PROXY_PORT,
        )
    {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "claude_mom.seed_proxy_port_failed",
            serde_json::json!({ "message": format!("{error:#}") }),
        );
    }
}

/// ClaudeMom 要补上的环境变量：只补没设置过的。拆出来便于测试，不读写真实环境。
fn claude_mom_env_defaults(
    is_set: &dyn Fn(&str) -> bool,
    home: Option<std::path::PathBuf>,
) -> Vec<(&'static str, std::ffi::OsString)> {
    let mut defaults = Vec::new();
    if !is_set(codex_plus_core::paths::STATE_DIR_ENV)
        && let Some(home) = home
    {
        defaults.push((
            codex_plus_core::paths::STATE_DIR_ENV,
            home.join(CLAUDE_MOM_STATE_DIR).into_os_string(),
        ));
    }
    // CODEX_PLUS_GUARD_PORT 同时覆盖两个锁端口，设置了就整体尊重。
    if !is_set("CODEX_PLUS_GUARD_PORT") {
        if !is_set("CODEX_PLUS_MANAGER_GUARD_PORT") {
            let port = codex_plus_core::ports::manager_guard_port().saturating_add(PORT_SHIFT);
            defaults.push(("CODEX_PLUS_MANAGER_GUARD_PORT", port.to_string().into()));
        }
        if !is_set("CODEX_PLUS_LAUNCHER_GUARD_PORT") {
            let port = codex_plus_core::ports::launcher_guard_port().saturating_add(PORT_SHIFT);
            defaults.push(("CODEX_PLUS_LAUNCHER_GUARD_PORT", port.to_string().into()));
        }
    }
    defaults
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(defaults: &[(&'static str, std::ffi::OsString)]) -> Vec<&'static str> {
        defaults.iter().map(|(name, _)| *name).collect()
    }

    #[test]
    fn claude_mom_defaults_isolate_state_dir_and_guard_ports() {
        let home = std::path::PathBuf::from("/home/someone");
        let defaults = claude_mom_env_defaults(&|_| false, Some(home.clone()));
        assert_eq!(
            names(&defaults),
            [
                "CODEX_PLUS_STATE_DIR",
                "CODEX_PLUS_MANAGER_GUARD_PORT",
                "CODEX_PLUS_LAUNCHER_GUARD_PORT"
            ]
        );
        assert_eq!(defaults[0].1, home.join(".claude-mom").into_os_string());
        // 端口在 Codex++ 的基础上平移，不能和它撞上。
        let manager: u16 = defaults[1].1.to_str().unwrap().parse().unwrap();
        let launcher: u16 = defaults[2].1.to_str().unwrap().parse().unwrap();
        assert_ne!(manager, codex_plus_core::ports::MANAGER_GUARD_PORT_BASE);
        assert_ne!(launcher, codex_plus_core::ports::LAUNCHER_GUARD_PORT_BASE);
        assert_ne!(manager, launcher);
        assert_ne!(
            CLAUDE_MOM_PROTOCOL_PROXY_PORT,
            codex_plus_core::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT
        );
    }

    #[test]
    fn explicit_environment_wins_over_claude_mom_defaults() {
        let all_set = claude_mom_env_defaults(&|_| true, Some("/home/someone".into()));
        assert!(all_set.is_empty());
        // 统一锁端口覆盖两个锁端口；状态目录照常补。
        let shared_guard =
            claude_mom_env_defaults(&|name| name == "CODEX_PLUS_GUARD_PORT", Some("/h".into()));
        assert_eq!(names(&shared_guard), ["CODEX_PLUS_STATE_DIR"]);
        // 拿不到家目录时不瞎猜状态目录。
        let no_home = claude_mom_env_defaults(&|_| false, None);
        assert!(!names(&no_home).contains(&"CODEX_PLUS_STATE_DIR"));
    }

    #[test]
    fn codex_plus_plus_is_the_default_flavor_and_keeps_its_window() {
        // 测试进程里没人调用 init_claude_mom。
        assert_eq!(current(), Flavor::CodexPlusPlus);
        assert_eq!(app_name(), "Codex++");
        let window = main_window();
        assert_eq!(window.url, "/index.html");
        assert_eq!(window.size, (1180.0, 820.0));
    }
}
