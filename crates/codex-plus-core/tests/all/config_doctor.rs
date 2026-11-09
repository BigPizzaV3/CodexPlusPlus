//! 配置体检：本地规则的判定口径，以及送给外部模型的摘要必须脱敏。

use codex_plus_core::config_doctor::{
    DoctorSettings, Level, SOURCE_CURRENT, SOURCE_CUSTOM, SOURCE_LOCAL, ai_request_body,
    build_snapshot, current_source, mask_secret, model_entries, parse_ai_reply, presets,
    redact_secrets, run_ai, run_rules, snapshot_prompt_text,
};
use codex_plus_core::settings::{BackendSettings, RelayProfile};

fn base_settings(profile: RelayProfile) -> BackendSettings {
    let mut settings = BackendSettings::default();
    settings.relay_profiles_enabled = true;
    settings.relay_profiles = vec![profile];
    settings
}

fn good_profile() -> RelayProfile {
    let mut profile = RelayProfile::default();
    profile.id = "relay-good".into();
    profile.name = "测试中转".into();
    profile.base_url = "https://api.example.com/v1".into();
    profile.api_key = "sk-test-0000000000000000".into();
    profile.model_list = "deepseek-v4-pro[128K]\nqwen3-8b".into();
    profile.context_window = "128000".into();
    profile.auto_compact_limit = "110000".into();
    profile
}

#[test]
fn healthy_profile_has_no_errors() {
    let dir = tempfile::tempdir().unwrap();
    let findings = run_rules(&base_settings(good_profile()), dir.path());
    let errors: Vec<&str> = findings
        .iter()
        .filter(|item| item.level == Level::Error)
        .map(|item| item.id.as_str())
        .collect();
    assert!(errors.is_empty(), "健康配置不该报错：{errors:?}");
}

#[test]
fn flags_auto_compact_above_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = good_profile();
    profile.auto_compact_limit = "200000".into();
    let findings = run_rules(&base_settings(profile), dir.path());
    assert!(findings.iter().any(|item| item.id == "profile.auto_compact.too_late"));
    // 早于 30% 是另一个警告，不该混进来。
    assert!(!findings.iter().any(|item| item.id == "profile.auto_compact.too_early"));
}

#[test]
fn flags_missing_api_key_when_auth_required() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = good_profile();
    profile.api_key.clear();
    let findings = run_rules(&base_settings(profile), dir.path());
    let item = findings
        .iter()
        .find(|item| item.id == "profile.api_key.missing")
        .expect("缺 Key 要报出来");
    assert_eq!(item.level, Level::Error);
    assert!(item.fix.is_some(), "报错必须带修复建议");
}

#[test]
fn no_auth_profile_does_not_require_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = good_profile();
    profile.api_key.clear();
    profile.relay_mode = codex_plus_core::settings::RelayMode::PureApi;
    profile.no_auth = true;
    let findings = run_rules(&base_settings(profile), dir.path());
    assert!(!findings.iter().any(|item| item.id == "profile.api_key.missing"));
}

#[test]
fn flags_duplicate_ids_and_models() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = good_profile();
    first.id = "relay-dup".into();
    first.model_list = "qwen3-8b\nqwen3-8b".into();
    let mut second = good_profile();
    second.id = "relay-dup".into();
    second.name = "另一条".into();
    let mut settings = BackendSettings::default();
    settings.relay_profiles = vec![first, second];
    let findings = run_rules(&settings, dir.path());
    assert!(findings.iter().any(|item| item.id == "relay.duplicate_id"));
    assert!(findings.iter().any(|item| item.id == "profile.model_list.duplicate"));
}

#[test]
fn flags_base_url_written_down_to_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = good_profile();
    profile.base_url = "https://api.example.com/v1/chat/completions".into();
    let findings = run_rules(&base_settings(profile), dir.path());
    assert!(findings.iter().any(|item| item.id == "profile.base_url.over_path"));
}

#[test]
fn flags_plain_http_to_remote_host() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = good_profile();
    profile.base_url = "http://api.example.com/v1".into();
    let findings = run_rules(&base_settings(profile), dir.path());
    assert!(findings.iter().any(|item| item.id == "profile.base_url.insecure"));
    // 本机地址不算风险。
    let mut local = good_profile();
    local.base_url = "http://127.0.0.1:11434/v1".into();
    let findings = run_rules(&base_settings(local), dir.path());
    assert!(!findings.iter().any(|item| item.id == "profile.base_url.insecure"));
}

#[test]
fn flags_broken_model_windows_json() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = good_profile();
    profile.model_windows = "{\"qwen3-8b\": 32768,".into();
    let findings = run_rules(&base_settings(profile), dir.path());
    assert!(findings.iter().any(|item| item.id.starts_with("profile.modelWindows")));
}

#[test]
fn flags_missing_catalog_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "model_catalog_json = \"codex-plus-models.json\"\n",
    )
    .unwrap();
    let findings = run_rules(&base_settings(good_profile()), dir.path());
    assert!(findings.iter().any(|item| item.id == "catalog.missing"));
}

#[test]
fn broken_config_toml_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "model = \"gpt-5\"\n[broken\n").unwrap();
    let findings = run_rules(&base_settings(good_profile()), dir.path());
    assert!(findings.iter().any(|item| item.id == "config.invalid" && item.level == Level::Error));
}

#[test]
fn snapshot_never_carries_the_api_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = good_profile();
    profile.api_key = "sk-live-abcdefghijklmnopqrstuvwxyz0123456789".into();
    let settings = base_settings(profile);
    let findings = run_rules(&settings, dir.path());
    let snapshot = build_snapshot(&settings, dir.path(), &findings);
    let text = snapshot_prompt_text(&snapshot);
    assert!(!text.contains("sk-live-abcdefghij"), "摘要里出现了 Key：{text}");
    assert!(text.contains("\"hasApiKey\": true"), "至少要告诉模型有没有 Key：{text}");
}

#[test]
fn redact_keeps_normal_text() {
    let text = "base_url=https://api.example.com/v1 窗口 128000";
    assert_eq!(redact_secrets(text), text);
    assert!(redact_secrets("Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123").contains("已脱敏"));
}

#[test]
fn model_entries_skip_blank_and_comment_lines() {
    let entries = model_entries("\n # 注释\ngpt-5\n\nqwen3-8b[128K], deepseek-v4-pro\n");
    assert_eq!(entries, vec!["gpt-5", "qwen3-8b[128K]", "deepseek-v4-pro"]);
}

#[test]
fn ai_body_is_chat_completions_shaped() {
    let settings = codex_plus_core::config_doctor::DoctorSettings {
        enabled: true,
        base_url: "https://api.example.com/v1".into(),
        model: "qwen3-8b".into(),
        api_key: "sk-x".into(),
        preset_id: "custom".into(),
        source: "custom".into(),
    };
    let body = ai_request_body(&settings, &serde_json::json!({"profiles": []}));
    assert_eq!(body["model"], "qwen3-8b");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["stream"], false);
}

#[test]
fn presets_cover_local_and_cloud() {
    let presets = presets();
    assert!(presets.iter().any(|preset| !preset.needs_key), "要有免 Key 的本地选项");
    assert!(presets.iter().any(|preset| preset.id == "ollama"));
    assert_eq!(mask_secret("sk-1234567890abcd"), "sk-1***cd");
}

#[test]
fn parse_reply_drops_entries_without_title() {
    let (summary, items) = parse_ai_reply(r#"{"summary":"还行","items":[{"level":"error"},{"level":"warn","title":"阈值偏低"}]}"#);
    assert_eq!(summary, "还行");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "阈值偏低");
}

#[test]
fn string_window_values_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = good_profile();
    profile.model_list = "gpt-5\nqwen3-8b".into();
    // 真实 settings.json 里窗口是「数字字符串」，自动化压缩是百分比字符串。
    profile.model_windows = r#"{"gpt-5":"400000","qwen3-8b":131072}"#.into();
    profile.model_auto_compact = r#"{"gpt-5":"90%"}"#.into();
    let findings = run_rules(&base_settings(profile), dir.path());
    let noise: Vec<&str> = findings
        .iter()
        .filter(|item| item.id.starts_with("profile.modelWindows") || item.id.starts_with("profile.modelAutoCompact"))
        .map(|item| item.id.as_str())
        .collect();
    assert!(noise.is_empty(), "字符串窗口值不该被当成错误：{noise:?}");
}

#[test]
fn bad_windows_are_aggregated_into_one_finding() {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = good_profile();
    profile.model_list = "alpha\nbeta\ngamma\ndelta".into();
    profile.model_windows = r#"{"alpha":"5","beta":"6","gamma":"7","delta":"1048576"}"#.into();
    let findings = run_rules(&base_settings(profile), dir.path());
    let hits: Vec<&codex_plus_core::config_doctor::Finding> =
        findings.iter().filter(|item| item.id == "profile.modelWindows.range").collect();
    assert_eq!(hits.len(), 1, "同一个供应商的坏窗口值要合成一条");
    assert!(hits[0].detail.contains("alpha"), "前三项要列出来：{}", hits[0].detail);
    assert!(hits[0].detail.contains("delta") == false, "值正常的条目不该出现在结论里：{}", hits[0].detail);
}

#[test]
fn inactive_profile_gaps_are_not_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut active = good_profile();
    active.id = "relay-active".into();
    let mut spare = RelayProfile::default();
    spare.id = "relay-spare".into();
    spare.name = "备用".into();
    let mut settings = BackendSettings::default();
    settings.relay_profiles_enabled = true;
    settings.relay_profiles = vec![active, spare];
    let findings = run_rules(&settings, dir.path());
    let empty_url = findings
        .iter()
        .find(|item| item.id == "profile.base_url.empty" && item.scope.as_deref() == Some("备用"))
        .expect("备用供应商的空白地址还是要提一句");
    assert_eq!(empty_url.level, Level::Info, "没在用的供应商不该报致命错误");
}

#[test]
fn reuse_current_provider_needs_no_configuration() {
    // 默认来源就是「复用当前供应商」：打开开关即可跑，不用填地址或 Key。
    let settings = DoctorSettings { enabled: true, ..Default::default() };
    assert_eq!(settings.source(), SOURCE_CURRENT);
    assert!(settings.ready(), "复用当前供应商不该再要求填任何字段");
    assert_eq!(settings.view().source, SOURCE_CURRENT);
}

#[test]
fn custom_source_still_requires_url_and_model() {
    let empty = DoctorSettings { enabled: true, source: SOURCE_CUSTOM.into(), ..Default::default() };
    assert!(!empty.ready());
    let filled = DoctorSettings {
        enabled: true,
        source: SOURCE_CUSTOM.into(),
        base_url: "https://api.example.com/v1".into(),
        model: "free-model".into(),
        ..Default::default()
    };
    assert!(filled.ready());
    // 关掉开关就不允许发出去，哪怕填得再全。
    assert!(!DoctorSettings { enabled: false, ..filled }.ready());
}

#[test]
fn local_source_does_not_require_key() {
    let settings = DoctorSettings { enabled: true, source: SOURCE_LOCAL.into(), ..Default::default() };
    assert!(settings.ready());
}

#[test]
fn current_source_reads_model_from_generated_config() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "model = \"gpt-5.3-codex\"\n").unwrap();
    let source = current_source(&base_settings(good_profile()), dir.path());
    assert!(source.available);
    assert_eq!(source.model, "gpt-5.3-codex");
    assert!(source.label.contains("测试中转"), "标签要写清是哪家供应商：{}", source.label);

    let empty = tempfile::tempdir().unwrap();
    let missing = current_source(&base_settings(good_profile()), empty.path());
    assert!(!missing.available, "没读到默认模型时不能假装可用");
}

#[tokio::test]
async fn ai_run_skips_without_network_when_disabled() {
    let settings = DoctorSettings::default();
    let report = run_ai(&settings, &serde_json::json!({"profiles": []})).await;
    assert_eq!(report.status, "skipped");
    assert!(report.elapsed_ms == 0);
}

#[tokio::test]
async fn ai_run_reports_unreachable_endpoint_instead_of_panicking() {
    // 本机没人监听的端口：必须是一条 failed 结论，而不是 panic 或挂住。
    let settings = DoctorSettings {
        enabled: true,
        source: SOURCE_CUSTOM.into(),
        base_url: "http://127.0.0.1:9/v1".into(),
        model: "whatever".into(),
        ..Default::default()
    };
    let report = run_ai(&settings, &serde_json::json!({"profiles": []})).await;
    assert_eq!(report.status, "failed");
    assert!(!report.message.is_empty(), "失败要有原因");
}
