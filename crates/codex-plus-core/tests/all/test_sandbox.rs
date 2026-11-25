//! 测试不许碰用户的真实目录：每个测试程序都要在 main 之前进家目录沙箱（`codex_plus_core::test_sandbox`）。

use std::path::{Path, PathBuf};

use crate::common::global_state_lock;

#[test]
fn this_process_runs_inside_the_home_sandbox() {
    let _guard = global_state_lock();
    let root = codex_plus_core::test_sandbox::root().expect("测试程序没有进家目录沙箱");
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    assert!(home.starts_with(root), "HOME 没有指到沙箱：{}", home.display());
    // 走默认路径的几处都落在沙箱里（没有测试临时改环境变量时）。
    if std::env::var_os("CLAUDE_CONFIG_DIR").is_none() {
        assert!(codex_plus_core::tools::default_claude_home_dir().starts_with(root));
    }
    if std::env::var_os("CODEX_HOME").is_none() {
        assert!(codex_plus_core::relay_config::default_codex_home_dir().starts_with(root));
    }
    if std::env::var_os("CODEX_PLUS_STATE_DIR").is_none() {
        assert!(codex_plus_core::paths::default_app_state_dir().starts_with(root));
    }
    // 用户目录统一走 user_dirs：沙箱里按沙箱推导（Windows 上 directories 不看环境变量，只能靠这里挡住）。
    let dirs = codex_plus_core::user_dirs::BaseDirs::new().expect("user_dirs");
    for path in [dirs.home_dir(), dirs.config_dir(), dirs.data_dir(), dirs.data_local_dir()] {
        assert!(path.starts_with(root), "用户目录没落在沙箱里：{}", path.display());
    }
    assert!(codex_plus_core::user_dirs::desktop_dir().expect("desktop").starts_with(root));
}

/// 每个测试程序的根文件：工作区成员的 `tests/*.rs`、`tests/*/main.rs`，以及带单元测试的 `src/lib.rs` / `src/main.rs`。
fn test_roots(workspace: &Path) -> Vec<PathBuf> {
    let members = ["crates/codex-plus-core", "crates/codex-plus-data", "apps/codex-plus-launcher", "apps/codex-plus-manager/src-tauri"];
    let mut roots = Vec::new();
    for member in members {
        let dir = workspace.join(member);
        for name in ["src/lib.rs", "src/main.rs"] {
            let path = dir.join(name);
            if std::fs::read_to_string(&path).is_ok_and(|text| text.contains("#[cfg(test)]") || text.contains("#[test]")) {
                roots.push(path);
            }
        }
        let Ok(entries) = std::fs::read_dir(dir.join("tests")) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                roots.push(path);
            } else if path.join("main.rs").is_file() {
                roots.push(path.join("main.rs"));
            }
        }
    }
    roots
}

#[test]
fn every_test_binary_enters_the_home_sandbox() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let roots = test_roots(&workspace);
    assert!(roots.len() >= 10, "没找到测试程序：{roots:?}");
    let missing: Vec<_> = roots
        .iter()
        .filter(|path| !std::fs::read_to_string(path).unwrap_or_default().contains("test_sandbox::enter()"))
        .map(|path| path.strip_prefix(&workspace).unwrap_or(path).display().to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "这些测试程序没进家目录沙箱，可能改到用户真实的 ~/.claude、~/.codex：{missing:?}\n照 tests/all/main.rs 末尾加一个 #[ctor::ctor] 调 test_sandbox::enter()"
    );
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Windows 上 `directories` / 已知文件夹接口不看环境变量，沙箱挡不住：代码里的用户目录只能经 `user_dirs` 取。
#[test]
fn nothing_bypasses_user_dirs() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for member in ["crates/codex-plus-core", "crates/codex-plus-data", "apps/codex-plus-launcher", "apps/codex-plus-manager/src-tauri"] {
        for sub in ["src", "examples", "tests"] {
            rust_files(&workspace.join(member).join(sub), &mut files);
        }
    }
    assert!(files.len() > 50, "没找到源文件");
    let allowed = |path: &Path| path.ends_with("src/user_dirs.rs") || path.ends_with("src/windows_integration.rs") || path.ends_with("tests/all/test_sandbox.rs");
    let offenders: Vec<_> = files
        .iter()
        .filter(|path| !allowed(path))
        .filter(|path| {
            // 注释里提到不算。
            std::fs::read_to_string(path).unwrap_or_default().lines().any(|line| {
                let code = line.trim_start();
                !code.starts_with("//") && (code.contains("directories::") || code.contains("SHGetKnownFolderPath("))
            })
        })
        .map(|path| path.strip_prefix(&workspace).unwrap_or(path).display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "这些文件绕过了 codex_plus_core::user_dirs 直接取用户目录，Windows 上测试沙箱挡不住：{offenders:?}"
    );
}
