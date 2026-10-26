#![cfg_attr(windows, windows_subsystem = "windows")]

/// Compatibility binary for the v1.7.x macOS updater.
///
/// The old updater requires a second manager bundle and opens it after replacing
/// both apps. The real Codex++ bundle is opened immediately afterward, so this
/// hidden shim only needs to be a valid small Mach-O process that exits cleanly.
///
/// 用户开了「显示隐藏文件」时会看到这个壳并把它拖进「应用程序」，双击后什么都
/// 不发生（#2493）。所以 macOS 上被启动时转交给主 Codex++：主程序已安装就直接
/// 拉起（旧更新器紧接着也会打开主程序，单实例下重复激活无害），找不到主程序才
/// 弹窗说明该拖哪个 app。
fn main() {
    #[cfg(target_os = "macos")]
    handoff(|program, args| {
        std::process::Command::new(program)
            .args(args)
            .status()
            .is_ok_and(|status| status.success())
    });
}

const MAIN_BUNDLE_ID: &str = "com.bigpizzav3.codexplusplus";
const MISPLACED_SHIM_DIALOG: &str = "display dialog \"这是旧版更新器使用的兼容组件，不是 Codex++ 主程序。请把安装包里的「Codex++.app」拖入「应用程序」后再打开。\" buttons {\"好\"} default button 1 with title \"Codex++\"";

/// 返回是否弹出了提示。`run` 执行命令并返回是否成功，便于测试注入。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn handoff(mut run: impl FnMut(&str, &[&str]) -> bool) -> bool {
    if run("/usr/bin/open", &["-b", MAIN_BUNDLE_ID]) {
        return false;
    }
    run("/usr/bin/osascript", &["-e", MISPLACED_SHIM_DIALOG]);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_main_app_without_dialog_when_installed() {
        let mut calls = Vec::new();
        let dialog = handoff(|program, args| {
            calls.push((program.to_string(), args.join(" ")));
            true
        });
        assert!(!dialog);
        assert_eq!(calls, vec![("/usr/bin/open".to_string(), format!("-b {MAIN_BUNDLE_ID}"))]);
    }

    #[test]
    fn explains_misplaced_shim_when_main_app_is_missing() {
        let mut programs = Vec::new();
        let dialog = handoff(|program, _| {
            programs.push(program.to_string());
            program != "/usr/bin/open"
        });
        assert!(dialog);
        assert_eq!(programs, vec!["/usr/bin/open", "/usr/bin/osascript"]);
    }
}
