use std::path::PathBuf;

use codex_plus_core::cli_wrapper::{
    build_wrapper_source, parse_wrapper_source_settings, resolve_real_codex_from_candidates,
    should_refresh_cli_wrapper, wrapper_dir_from_roaming, wrapper_settings_for_refresh,
};
use codex_plus_core::settings::BackendSettings;

#[test]
fn wrapper_source_embeds_absolute_real_codex_path() {
    let settings = BackendSettings {
        cli_wrapper_enabled: true,
        cli_wrapper_base_url: "https://proxy.example/v1".to_string(),
        cli_wrapper_api_key: "sk-test".to_string(),
        cli_wrapper_api_key_env: "CUSTOM_KEY".to_string(),
        ..BackendSettings::default()
    };
    let source = build_wrapper_source(
        &PathBuf::from(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__abc\app\resources\codex.exe",
        ),
        &PathBuf::from(r"C:\Users\me\.codex-plus-plus-cli"),
        &settings,
    );

    assert!(source.contains(r#"string realCodex = @"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__abc\app\resources\codex.exe";"#));
    assert!(!source.contains(r#"string realCodex = @"codex";"#));
    assert!(source.contains(r#"string apiKeyEnv = @"CUSTOM_KEY";"#));
    assert!(source.contains(
        r#"startInfo.EnvironmentVariables["OPENAI_BASE_URL"] = @"https://proxy.example/v1";"#
    ));
    assert!(source.contains(r#"startInfo.EnvironmentVariables[apiKeyEnv] = @"sk-test";"#));
}

#[test]
fn resolves_user_runtime_before_packaged_resources_codex() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp
        .path()
        .join("OpenAI.Codex_1.0.0.0_x64__abc")
        .join("app");
    let packaged = app_dir.join("resources").join("codex.exe");
    let user_runtime = temp
        .path()
        .join("OpenAI")
        .join("Codex")
        .join("bin")
        .join("codex.exe");
    std::fs::create_dir_all(packaged.parent().unwrap()).unwrap();
    std::fs::create_dir_all(user_runtime.parent().unwrap()).unwrap();
    std::fs::write(&packaged, "").unwrap();
    std::fs::write(&user_runtime, "").unwrap();

    let resolved = resolve_real_codex_from_candidates(Some(&app_dir), &[user_runtime.clone()])
        .expect("user runtime codex should be preferred");

    assert_eq!(resolved, user_runtime);
}

#[test]
fn resolves_packaged_resources_when_user_runtime_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp
        .path()
        .join("OpenAI.Codex_1.0.0.0_x64__abc")
        .join("app");
    let packaged = app_dir.join("resources").join("codex.exe");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::create_dir_all(packaged.parent().unwrap()).unwrap();
    std::fs::write(&packaged, "").unwrap();

    let resolved = resolve_real_codex_from_candidates(Some(&app_dir), &[])
        .expect("packaged codex should be used as fallback");

    assert_eq!(resolved, packaged);
}

#[test]
fn wrapper_dir_uses_roaming_codex_plus_plus() {
    assert_eq!(
        wrapper_dir_from_roaming(&PathBuf::from(r"C:\Users\me\AppData\Roaming")),
        PathBuf::from(r"C:\Users\me\AppData\Roaming\Codex++")
    );
}

#[test]
fn repair_refreshes_when_wrapper_already_exists_even_if_setting_is_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let wrapper_dir = temp.path().join("Codex++");
    std::fs::create_dir_all(&wrapper_dir).unwrap();
    std::fs::write(wrapper_dir.join("codex-wrapper.exe"), "").unwrap();

    assert!(should_refresh_cli_wrapper(
        &BackendSettings::default(),
        &wrapper_dir
    ));
}

#[test]
fn repair_skips_when_wrapper_is_disabled_and_absent() {
    let temp = tempfile::tempdir().unwrap();

    assert!(!should_refresh_cli_wrapper(
        &BackendSettings::default(),
        temp.path()
    ));
}

#[test]
fn repair_preserves_existing_wrapper_api_settings_when_global_setting_is_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let wrapper_dir = temp.path().join("Codex++");
    std::fs::create_dir_all(&wrapper_dir).unwrap();
    std::fs::write(
        wrapper_dir.join("codex-wrapper.cs"),
        r#"class CodexWrapper
{
    static int Main(string[] args)
    {
        string apiKeyEnv = @"CUSTOM_KEY";
        string apiKey = @"sk-old";
        startInfo.EnvironmentVariables["OPENAI_BASE_URL"] = @"https://old.example/v1";
        startInfo.EnvironmentVariables[apiKeyEnv] = apiKey;
    }
}"#,
    )
    .unwrap();

    let settings = wrapper_settings_for_refresh(&BackendSettings::default(), &wrapper_dir);

    assert_eq!(settings.cli_wrapper_api_key_env, "CUSTOM_KEY");
    assert_eq!(settings.cli_wrapper_api_key, "sk-old");
    assert_eq!(
        settings.cli_wrapper_base_url,
        "https://old.example/v1".to_string()
    );
}

#[test]
fn parses_new_wrapper_source_api_settings() {
    let parsed = parse_wrapper_source_settings(
        r#"string apiKeyEnv = @"CUSTOM_KEY";
startInfo.EnvironmentVariables["OPENAI_BASE_URL"] = @"https://new.example/v1";
startInfo.EnvironmentVariables[apiKeyEnv] = @"sk-new";"#,
    )
    .unwrap();

    assert_eq!(parsed.cli_wrapper_api_key_env, "CUSTOM_KEY");
    assert_eq!(parsed.cli_wrapper_api_key, "sk-new");
    assert_eq!(parsed.cli_wrapper_base_url, "https://new.example/v1");
}
