use std::path::{Path, PathBuf};

pub fn default_codex_home_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .filter(|path| codex_home_env_dir_is_valid(path))
        .unwrap_or_else(default_user_codex_home_dir)
}

fn codex_home_env_dir_is_valid(path: &PathBuf) -> bool {
    !path.as_os_str().is_empty() && !path.to_string_lossy().trim().is_empty() && path.is_dir()
}

/// 递归删除的守卫：**绝不允许**删掉 `CODEX_HOME` 本身、它的任何一个祖先目录，
/// 或者文件系统根。
///
/// 起因是 #2146：用户在一次 Codex++ 自更新 + Codex 桌面版自动更新的过程中，整个
/// `%USERPROFILE%\.codex`（454 MB 会话历史）被永久删除，只剩 1.1% 可恢复。我审计了
/// 全部 `remove_dir_all` 调用点，没有找到一条能删到 `.codex` 的既定路径——也就是说
/// 现有代码在正常输入下是安全的。但这也意味着：**一旦某个上游值（环境变量、被解析
/// 坏的路径、更新后失效的目录）指向了 home 本身，就没有任何东西拦得住它。**
/// 数据丢失不可逆，所以这里加一道与具体触发源无关的兜底。
///
/// 判定在**规范化之后**做，`..`、符号链接、大小写差异都拦得住；同时容忍路径尚不存在
/// （清理临时目录的常见情形，此时用词法规范化比较）。
pub fn ensure_safe_recursive_removal(target: &Path, codex_home: &Path) -> anyhow::Result<()> {
    let target = normalize_for_comparison(target);
    let home = normalize_for_comparison(codex_home);

    if target.as_os_str().is_empty() || target == Path::new("/") {
        anyhow::bail!("拒绝删除文件系统根目录：{}", target.display());
    }
    if target == home {
        anyhow::bail!(
            "拒绝递归删除 CODEX_HOME 本身（{}）——这会连同全部会话历史一起丢失",
            target.display()
        );
    }
    if home.starts_with(&target) {
        anyhow::bail!(
            "拒绝删除 CODEX_HOME 的祖先目录 {}（CODEX_HOME = {}）",
            target.display(),
            home.display()
        );
    }
    Ok(())
}

/// 规范化到可比较的形态。
///
/// 关键点：**必须两边用同一种方式规范化**。如果待删路径能 canonicalize、而 home
/// 不存在只能走词法，两边就会一个带 `/private` 前缀一个不带（macOS 的 `/var` →
/// `/private/var` 就是这种），比较直接失效、守卫形同虚设。
///
/// 所以先 canonicalize **最深的已存在祖先**，再把剩下还不存在的部分按词法拼回去。
/// 这样无论路径存在与否，结果都稳定在同一形态；`..` 也在词法阶段消掉。
fn normalize_for_comparison(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let lexical = lexically_normalize(&absolute);

    // 找到最深的、真实存在的祖先，用它拿到 canonical 前缀
    let mut ancestor = lexical.as_path();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canonical) = ancestor.canonicalize() {
            let mut result = canonical;
            for component in tail.iter().rev() {
                result.push(component);
            }
            return result;
        }
        let Some(name) = ancestor.file_name() else {
            return lexical;
        };
        tail.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            return lexical;
        };
        ancestor = parent;
    }
}

/// 纯词法规范化：消掉 `.` 与 `..`、重复分隔符，不做任何文件系统访问。
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn default_user_codex_home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::Mutex;

    // ---- #2146 递归删除守卫 ----

    #[test]
    fn removal_guard_rejects_codex_home_itself() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        std::fs::create_dir_all(&home).unwrap();
        let error = ensure_safe_recursive_removal(&home, &home).unwrap_err();
        assert!(error.to_string().contains("CODEX_HOME 本身"), "{error}");
    }

    #[test]
    fn removal_guard_rejects_ancestors_of_codex_home() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("a").join("b").join(".codex");
        std::fs::create_dir_all(&home).unwrap();
        // 用户主目录、家目录的上级……任何包含 home 的目录都该被拒绝
        for candidate in [
            temp.path().to_path_buf(),
            temp.path().join("a"),
            temp.path().join("a").join("b"),
        ] {
            let error = ensure_safe_recursive_removal(&candidate, &home).unwrap_err();
            assert!(
                error.to_string().contains("祖先目录"),
                "{candidate:?} => {error}"
            );
        }
    }

    #[test]
    fn removal_guard_rejects_filesystem_root() {
        let home = PathBuf::from("/somewhere/.codex");
        let error = ensure_safe_recursive_removal(Path::new("/"), &home).unwrap_err();
        assert!(error.to_string().contains("文件系统根"), "{error}");
    }

    #[test]
    fn removal_guard_allows_normal_children() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        std::fs::create_dir_all(&home).unwrap();
        for candidate in [
            home.join("backups_state").join("provider-sync"),
            home.join("model-catalogs"),
            home.join("skills").join("x"),
        ] {
            std::fs::create_dir_all(&candidate).unwrap();
            ensure_safe_recursive_removal(&candidate, &home)
                .unwrap_or_else(|e| panic!("{candidate:?} 不该被拒绝：{e}"));
        }
        // home 之外的正常目录也不受影响
        let outside = temp.path().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        ensure_safe_recursive_removal(&outside, &home).unwrap();
    }

    /// `..` 与末尾斜杠不能成为绕过守卫的口子。
    #[test]
    fn removal_guard_sees_through_traversal_and_trailing_slash() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        std::fs::create_dir_all(&home).unwrap();

        let via_parent = home.join("..");
        assert!(
            ensure_safe_recursive_removal(&via_parent, &home).is_err(),
            "`..` 绕过"
        );

        let mut with_slash = home.clone().into_os_string();
        with_slash.push("/");
        assert!(
            ensure_safe_recursive_removal(Path::new(&with_slash), &home).is_err(),
            "末尾斜杠绕过"
        );
    }

    /// home 尚不存在时（首次运行）也不能因为 canonicalize 失败而漏判。
    #[test]
    fn removal_guard_handles_nonexistent_paths() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("not-yet").join(".codex");
        // 祖先同样不存在
        assert!(ensure_safe_recursive_removal(temp.path(), &home).is_err());
        assert!(ensure_safe_recursive_removal(&home, &home).is_err());
    }

    static CODEX_HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct CodexHomeEnvGuard {
        previous: Option<OsString>,
    }

    impl CodexHomeEnvGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::var_os("CODEX_HOME");
            unsafe {
                std::env::set_var("CODEX_HOME", path);
            }
            Self { previous }
        }

        fn set_raw(value: &str) -> Self {
            let previous = std::env::var_os("CODEX_HOME");
            unsafe {
                std::env::set_var("CODEX_HOME", value);
            }
            Self { previous }
        }
    }

    impl Drop for CodexHomeEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var("CODEX_HOME", value),
                    None => std::env::remove_var("CODEX_HOME"),
                }
            }
        }
    }

    #[test]
    fn default_codex_home_dir_uses_existing_codex_home_env_dir() {
        let _lock = CODEX_HOME_ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let codex_home = temp.path().join("custom-codex-home");
        std::fs::create_dir_all(&codex_home).unwrap();
        let _guard = CodexHomeEnvGuard::set(&codex_home);

        assert_eq!(default_codex_home_dir(), codex_home);
        assert_eq!(crate::relay_config::default_codex_home_dir(), codex_home);
        assert_eq!(crate::codex_sqlite::default_codex_home_dir(), codex_home);
    }

    #[test]
    fn default_codex_home_dir_ignores_empty_or_missing_codex_home_env() {
        let _lock = CODEX_HOME_ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing-codex-home");
        let expected = default_user_codex_home_dir();

        {
            let _guard = CodexHomeEnvGuard::set_raw("   ");
            assert_eq!(default_codex_home_dir(), expected);
            assert_eq!(crate::relay_config::default_codex_home_dir(), expected);
            assert_eq!(crate::codex_sqlite::default_codex_home_dir(), expected);
        }

        {
            let _guard = CodexHomeEnvGuard::set(&missing);
            assert_eq!(default_codex_home_dir(), expected);
            assert_eq!(crate::relay_config::default_codex_home_dir(), expected);
            assert_eq!(crate::codex_sqlite::default_codex_home_dir(), expected);
        }
    }
}
