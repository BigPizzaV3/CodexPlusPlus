use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;
use std::process::Command;

#[test]
fn session_share_keeps_legacy_default_and_round_trips_partial_updates() {
    let old: BackendSettings =
        serde_json::from_value(json!({"enhancementsEnabled": true})).unwrap();
    assert_eq!(
        serde_json::to_value(old).unwrap()["codexAppSessionShare"],
        true
    );
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let profiles = serde_json::to_value(store.load().unwrap()).unwrap()["relayProfiles"].clone();
    store
        .update(json!({"codexAppSessionShare": false}))
        .unwrap();
    for invalid in [json!(null), json!("true"), json!(1), json!([]), json!({})] {
        let updated = store
            .update(json!({"codexAppSessionShare": invalid}))
            .unwrap();
        let value = serde_json::to_value(updated).unwrap();
        assert_eq!(value["codexAppSessionShare"], false);
        assert_eq!(value["relayProfiles"], profiles);
    }
    store.update(json!({"enhancementsEnabled": false})).unwrap();
    let updated = store.update(json!({"codexAppSessionShare": true})).unwrap();
    let value = serde_json::to_value(updated).unwrap();
    assert_eq!(value["codexAppSessionShare"], true);
    assert_eq!(value["enhancementsEnabled"], false);
    assert_eq!(serde_json::to_value(store.load().unwrap()).unwrap(), value);
}

#[test]
fn renderer_share_and_manager_fold_behaviors() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("node")
        .arg("--test")
        .arg("src/*.test.ts")
        .current_dir(root.join("apps/codex-plus-manager"))
        .output()
        .expect("node is required for renderer and manager UI tests");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn session_delete_renderer_behaviors() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("node")
        .args(["--test", "src/session-delete-runtime.test.ts"])
        .current_dir(root.join("apps/codex-plus-manager"))
        .output()
        .expect("node is required for session deletion UI tests");
    assert!(output.status.success(), "{}\n{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn manager_ui_typecheck() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manager = root.join("apps/codex-plus-manager");
    let output = Command::new("node")
        .arg(manager.join("node_modules/typescript/bin/tsc"))
        .args(["--noEmit", "-p", "tsconfig.json"])
        .current_dir(&manager)
        .output()
        .expect("node is required for manager UI checks");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn claude_localization_console_children_stay_hidden() {
    // 管理器是无控制台窗口的 GUI 进程：从它派生 PowerShell 时若不带
    // CREATE_NO_WINDOW，准备副本和启动中文 Claude 都会闪出 conhost 黑窗口；
    // 注入脚本拉起 Claude 时不隐藏窗口，用户误关那个窗口会直接结束 Claude。
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source =
        std::fs::read_to_string(root.join("crates/codex-plus-core/src/claude_localization.rs"))
            .expect("必须能读取 Claude 汉化模块源码");
    let inject = std::fs::read_to_string(root.join("assets/claude-localization/inject-cn.ps1"))
        .expect("必须能读取中文注入脚本");

    // 实测（MSIX 版 Claude 2.37337.1.0）加上 -WindowStyle Hidden 后 Claude 能启动，
    // 但 CDP 注入不再生效，脚本会抛 'Claude hook 注入失败'，所以必须保持不加。
    assert_eq!(
        inject
            .lines()
            .filter(|line| !line.trim_start().starts_with('#') && line.contains("-WindowStyle Hidden"))
            .count(),
        0,
        "inject-cn.ps1 不能给 Start-Process 加 -WindowStyle Hidden，会让 CDP 注入失效"
    );
    assert!(
        source.contains("fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> Command"),
        "汉化脚本进程必须经由 hidden_command 派生"
    );
    assert!(source.contains("command.creation_flags(crate::windows_create_no_window());"));
    assert!(source.contains("let mut command = hidden_command(program);"));
    // 裸 Command::new(program) 只允许出现在 hidden_command 内部这一处；
    // run_script 退回裸建 Command 就会重新弹出 conhost。
    assert_eq!(
        source
            .lines()
            .filter(|line| line.contains("let mut command = Command::new(program);"))
            .count(),
        1,
        "run_script 不能绕过 hidden_command 直接派生脚本进程"
    );
}

#[test]
fn claude_localization_copy_check_avoids_get_filehash() {
    // Get-FileHash 在被应用调起的 PowerShell 环境里可能不可用，副本校验必须走
    // .NET 实现；脚本同时要保持 UTF-8 BOM，否则 Windows 行尾转换会破坏解析。
    let build_copy = std::str::from_utf8(include_bytes!(
        "../../../assets/claude-localization/build-copy.ps1"
    ))
    .expect("build-copy.ps1 必须保持 UTF-8 编码");
    let inject_cn = std::str::from_utf8(include_bytes!(
        "../../../assets/claude-localization/inject-cn.ps1"
    ))
    .expect("inject-cn.ps1 必须保持 UTF-8 编码");

    assert_eq!(
        build_copy
            .lines()
            .filter(|line| !line.trim_start().starts_with('#') && line.contains("Get-FileHash"))
            .count(),
        0,
        "副本校验不能依赖 Get-FileHash"
    );
    assert!(build_copy.contains("Get-Sha256Hex"));
    assert!(build_copy.contains("副本校验失败："));
    assert!(inject_cn.contains("CODEX_PLUS_CLAUDE_COPY"));
    for script in [build_copy, inject_cn] {
        let bytes = script.as_bytes();
        assert!(
            bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
            "汉化脚本必须保留 UTF-8 BOM"
        );
        assert!(
            !bytes[3..].starts_with(&[0xEF, 0xBB, 0xBF]),
            "汉化脚本只能包含一个 UTF-8 BOM"
        );
    }
}

#[cfg(windows)]
#[test]
fn embedded_powershell_assets_parse_with_windows_powershell_5_1() {
    // ParseFile 会读取实际落盘的 UTF-8 资源；重复 BOM 不一定是语法错误，
    // 但会让 param 块退化成普通命令，因此同时检查 ParamBlock 是否存在。
    let assets = [
        (
            "build-copy.ps1",
            include_bytes!("../../../assets/claude-localization/build-copy.ps1").as_slice(),
        ),
        (
            "inject-cn.ps1",
            include_bytes!("../../../assets/claude-localization/inject-cn.ps1").as_slice(),
        ),
    ];
    let temp = tempfile::tempdir().unwrap();
    let validator = temp.path().join("validate-asset.ps1");
    std::fs::write(
        &validator,
        r#"param([string]$Path)
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errors)
if ($errors.Count -gt 0) {
  $errors | ForEach-Object { [Console]::Error.WriteLine($_.Message) }
  exit 1
}
if ($null -eq $ast.ParamBlock) {
  [Console]::Error.WriteLine('param block missing')
  exit 2
}
"#,
    )
    .unwrap();

    for (name, bytes) in assets {
        let path = temp.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&validator)
            .arg(&path)
            .output()
            .expect("Windows PowerShell 5.1 必须可用");
        assert!(
            output.status.success(),
            "{name} 未能通过 Windows PowerShell 5.1 Parser.ParseFile:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
