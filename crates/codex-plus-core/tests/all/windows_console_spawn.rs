//! 静态检查：Windows 上起子进程必须隐藏控制台窗口（issue #2048 / #2559 / #2778）。
//!
//! 管理器、启动器都是 GUI 子系统，没有自己的控制台。用 `Command::new` 起控制台程序
//! （tasklist、powershell、CLI 等）时不设 `CREATE_NO_WINDOW`，Windows 就会为它新开一个
//! 黑色终端窗口，用户看到的就是「切页面时闪黑框」。这类问题修过好几轮，每次都是新加的
//! 调用点漏了标志，而 macOS 上的 CI 编译不到 `cfg(windows)` 分支，单靠评审拦不住。
//!
//! 这里逐个扫描源码里的 `Command::new(`，每一处必须满足下面之一：
//! - 往后 30 行内设置了 `creation_flags`（`windows_create_no_window()` / `CREATE_NO_WINDOW`）；
//! - 同一行交给了已知会设标志的封装（见 `HIDING_HELPERS`）；
//! - 在 `#[cfg(test)] mod` 里，或最近的 cfg 条件排除了 Windows（macOS / unix / not(windows)）；
//! - 程序是 Unix 绝对路径（`/usr/bin/…`、`/bin/…`），Windows 上根本起不来；
//! - 登记在 `ALLOWED` 里并写明理由（GUI 程序、刻意打开终端、未编译的停用模块等）。
//!
//! 新增调用点被拦下时，优先补 `creation_flags`，不要往白名单里塞。

use std::fs;
use std::path::{Path, PathBuf};

/// 往后看多少行找 `creation_flags`：覆盖「先建 Command、链式配置、cfg(windows) 块里设标志」的写法。
const LOOKAHEAD_LINES: usize = 30;
/// 往前看多少行找 cfg 条件。
const CFG_LOOKBACK_LINES: usize = 8;

/// 内部统一设置了 `CREATE_NO_WINDOW` 的封装：调用点把 `Command::new(..)` 直接传给它们即可。
const HIDING_HELPERS: &[&str] = &[
    "probe_output(",   // claude_localization.rs
    "command_output(", // plugin_market.rs
    "git_output(",     // plugin_market.rs（upstream_worktree 的同名函数在停用模块里）
];

/// (文件路径后缀, 该行包含的片段, 理由)。片段尽量取能唯一定位调用点的字面量。
const ALLOWED: &[(&str, &str, &str)] = &[
    // GUI 程序或系统外壳：没有控制台，不会闪黑框。
    (
        "crates/codex-plus-core/src/windows_integration.rs",
        "Command::new(\"explorer.exe\")",
        "explorer 是 GUI 程序，ShellExecute 失败时用来打开链接",
    ),
    (
        "apps/codex-plus-manager/src-tauri/src/gateway_commands.rs",
        "Command::new(\"explorer\")",
        "explorer 是 GUI 程序，用来在资源管理器里显示文件",
    ),
    // 刻意打开终端：用户点「在终端继续 / 在终端启动」就是要看到窗口。
    (
        "crates/codex-plus-core/src/gateway/sessions.rs",
        "Command::new(\"cmd\")",
        "在新终端里继续会话 / 启动 CLI Agent，窗口是用户要的",
    ),
    // 只构造不执行：返回的 Command 由调用方交给 git_output（其中设置标志）。
    (
        "crates/codex-plus-core/src/plugin_market.rs",
        "let mut command = Command::new(git);",
        "git_command_with_binary 只拼参数，执行统一走 git_output",
    ),
    // macOS 分支：cfg 属性离调用点超过 CFG_LOOKBACK_LINES 行。
    (
        "crates/codex-plus-core/src/gateway/sessions.rs",
        "Command::new(\"open\")",
        "#[cfg(target_os = \"macos\")] 块里用「终端」打开 .command 文件",
    ),
    // 只在 macOS 运行期分支里走到的调用（用 `if cfg!` / 平台判断而非 cfg 属性门控）。
    (
        "crates/codex-plus-core/src/computer_use.rs",
        "Command::new(\"mdfind\")",
        "Computer Use 只支持 macOS，Spotlight 兜底找 ChatGPT.app",
    ),
    (
        "crates/codex-plus-core/src/launcher.rs",
        "let child = Command::new(executable)",
        "macOS 分支：用 open 启动 Codex.app",
    ),
    (
        "crates/codex-plus-core/src/launcher.rs",
        "let _ = Command::new(executable)",
        "macOS 退出清理命令，非 macOS 时命令为空直接返回",
    ),
    (
        "crates/codex-plus-core/src/launcher.rs",
        "Command::new(\"osascript\")",
        "函数开头已 `if !cfg!(target_os = \"macos\")` 返回",
    ),
    (
        "apps/codex-plus-manager/src-tauri/src/computer_use_commands.rs",
        "Command::new(\"open\").arg(&app_path)",
        "Computer Use 只支持 macOS，打开 ChatGPT.app",
    ),
    (
        "crates/codex-plus-core/src/update/macos.rs",
        "Command::new(",
        "模块只在 macOS（及测试）编译",
    ),
    // 停用模块：源文件保留以便回溯，但没有在 lib.rs 里声明，不参与编译。
    (
        "crates/codex-plus-core/src/upstream_worktree/",
        "Command::new(",
        "停用模块，未编译",
    ),
    (
        "crates/codex-plus-core/src/zed_remote",
        "Command::new(",
        "停用模块，未编译",
    ),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("仓库根目录")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// 标出 `#[cfg(test)]` 修饰的 `mod` 块里的行（按花括号配对，够用的近似）。
fn test_module_lines(lines: &[&str]) -> Vec<bool> {
    let mut in_test = vec![false; lines.len()];
    let mut index = 0;
    while index < lines.len() {
        let is_test_mod = lines[index].trim() == "#[cfg(test)]"
            && lines.get(index + 1).is_some_and(|next| {
                next.trim_start().starts_with("mod ") || next.trim_start().starts_with("pub mod ")
            });
        if !is_test_mod {
            index += 1;
            continue;
        }
        let mut depth = 0i32;
        let mut opened = false;
        let mut cursor = index;
        while cursor < lines.len() {
            in_test[cursor] = true;
            for ch in lines[cursor].chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        opened = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            cursor += 1;
            if opened && depth <= 0 {
                break;
            }
        }
        index = cursor;
    }
    in_test
}

/// 最近的 cfg 条件是否把 Windows 排除在外。
fn nearest_cfg_excludes_windows(lines: &[&str], index: usize) -> bool {
    let start = index.saturating_sub(CFG_LOOKBACK_LINES);
    let Some(cfg) = lines[start..=index]
        .iter()
        .rev()
        .find(|line| line.contains("cfg(") || line.contains("cfg!("))
    else {
        return false;
    };
    if cfg.contains("!cfg!(") {
        // `if !cfg!(target_os = "macos")` 这类取反的运行期判断，范围里反而包含 Windows。
        return false;
    }
    let (positive, negated) = split_negations(cfg);
    // `not(windows)`、`not(any(target_os = "macos", target_os = "windows"))`。
    if negated.contains("windows") {
        return true;
    }
    // `target_os = "macos"`、`unix`、`all(unix, not(target_os = "macos"))`、`any(linux, macos)`。
    !positive.contains("windows")
        && (positive.contains("macos") || positive.contains("unix") || positive.contains("linux"))
}

/// 把 cfg 文本拆成「不在 not(...) 里的部分」和「not(...) 里的部分」。
fn split_negations(cfg: &str) -> (String, String) {
    let mut positive = String::new();
    let mut negated = String::new();
    let mut rest = cfg;
    while let Some(start) = rest.find("not(") {
        positive.push_str(&rest[..start]);
        let inner = &rest[start + "not(".len()..];
        let mut depth = 1;
        let mut end = inner.len();
        for (offset, ch) in inner.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        negated.push_str(&inner[..end]);
        rest = inner.get(end + 1..).unwrap_or("");
    }
    positive.push_str(rest);
    (positive, negated)
}

#[test]
fn every_windows_reachable_spawn_hides_its_console_window() {
    let root = repo_root();
    let mut files = Vec::new();
    for dir in [
        "crates/codex-plus-core/src",
        "crates/codex-plus-data/src",
        "apps/codex-plus-manager/src-tauri/src",
        "apps/codex-plus-launcher/src",
        "apps/codex-plus-mobile-relay/src",
    ] {
        rust_files(&root.join(dir), &mut files);
    }
    files.sort();
    assert!(
        files.len() > 50,
        "没扫到源码，路径可能变了：{}",
        root.display()
    );

    let mut used_allowances = vec![false; ALLOWED.len()];
    let mut offenders = Vec::new();
    for file in &files {
        let relative = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(file).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let in_test = test_module_lines(&lines);
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("Command::new(") || line.trim_start().starts_with("//") {
                continue;
            }
            if in_test[index] || relative.ends_with("/tests.rs") {
                continue;
            }
            let end = (index + LOOKAHEAD_LINES).min(lines.len());
            if lines[index..end]
                .iter()
                .any(|line| line.contains("creation_flags"))
            {
                continue;
            }
            // 交给封装执行：`helper(Command::new(..)…)` 可能换行写，`helper(&mut command)` 在后面几行。
            if lines[index.saturating_sub(1)..end]
                .iter()
                .any(|line| HIDING_HELPERS.iter().any(|helper| line.contains(helper)))
            {
                continue;
            }
            if line.contains("Command::new(\"/usr/") || line.contains("Command::new(\"/bin/") {
                continue;
            }
            if nearest_cfg_excludes_windows(&lines, index) {
                continue;
            }
            if let Some(position) = ALLOWED
                .iter()
                .position(|(path, needle, _)| relative.contains(path) && line.contains(needle))
            {
                used_allowances[position] = true;
                continue;
            }
            offenders.push(format!("{relative}:{}: {}", index + 1, line.trim()));
        }
    }
    assert!(
        offenders.is_empty(),
        "这些 Command::new 在 Windows 上会闪黑色终端：请在 #[cfg(windows)] 下设置 \
         creation_flags(windows_create_no_window())，确属例外再登记到 ALLOWED 并写明理由。\n{}",
        offenders.join("\n")
    );
    let stale: Vec<_> = ALLOWED
        .iter()
        .zip(&used_allowances)
        .filter(|(_, used)| !**used)
        .map(|((path, needle, _), _)| format!("{path} / {needle}"))
        .collect();
    assert!(
        stale.is_empty(),
        "ALLOWED 里这些条目已匹配不到调用点，请删掉：\n{}",
        stale.join("\n")
    );
}

#[test]
fn cfg_detection_recognises_platform_gates() {
    let gated = [
        "#[cfg(target_os = \"macos\")]",
        "let x = Command::new(\"ps\")",
    ];
    assert!(nearest_cfg_excludes_windows(&gated, 1));
    let unix = ["#[cfg(unix)]", "Command::new(\"kill\")"];
    assert!(nearest_cfg_excludes_windows(&unix, 1));
    let not_windows = ["#[cfg(not(target_os = \"windows\"))]", "Command::new(path)"];
    assert!(nearest_cfg_excludes_windows(&not_windows, 1));
    let windows = ["#[cfg(windows)]", "Command::new(\"tasklist\")"];
    assert!(!nearest_cfg_excludes_windows(&windows, 1));
    let any_windows = ["#[cfg(any(windows, unix))]", "Command::new(\"x\")"];
    assert!(!nearest_cfg_excludes_windows(&any_windows, 1));
    let ungated = ["fn probe() {", "Command::new(\"tasklist\")"];
    assert!(!nearest_cfg_excludes_windows(&ungated, 1));
}
