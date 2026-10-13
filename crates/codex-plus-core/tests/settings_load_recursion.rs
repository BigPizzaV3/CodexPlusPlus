//! 回归：聚合供应商的 `configContents` 非空时，`SettingsStore::load()` 曾经无限递归。
//!
//! 链路：`load → normalize_settings_config_sections → normalize_relay_profile_for_storage
//! → complete_relay_profile_config`（聚合分支会 `SettingsStore::default().load()` 拿
//! `session_provider`）→ 又回到 `load`。主线程栈溢出 → 应用启动即崩。
//! 触发场景真实存在：切换供应商时 `backfill_relay_profile_from_live` 会把 live 配置
//! 回填进聚合档案，此后每次启动都会崩。

use codex_plus_core::paths::set_settings_path_for_tests;
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, SettingsStore};

fn aggregate_profile_with_config_contents() -> RelayProfile {
    RelayProfile {
        id: "aggregate-test".to_string(),
        name: "聚合供应商".to_string(),
        relay_mode: RelayMode::Aggregate,
        // 触发条件：聚合档案带着回填进来的 config.toml 文本。
        config_contents: concat!(
            "model_provider = \"custom\"\n",
            "model = \"cpx-member/model\"\n",
            "model_catalog_json = \"model-catalogs/aggregate-test.json\"\n",
            "[model_providers.custom]\n",
            "name = \"custom\"\n",
            "wire_api = \"responses\"\n",
            "requires_openai_auth = false\n",
            "base_url = \"http://127.0.0.1:57321/v1\"\n",
        )
        .to_string(),
        ..RelayProfile::default()
    }
}

#[test]
fn aggregate_profile_with_backfilled_config_contents_loads_without_recursing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let previous = set_settings_path_for_tests(Some(path.clone()));

    let mut settings = BackendSettings::default();
    settings.relay_profiles = vec![
        aggregate_profile_with_config_contents(),
        RelayProfile {
            id: "member".to_string(),
            name: "成员".to_string(),
            model: "model".to_string(),
            base_url: "https://example.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            ..RelayProfile::default()
        },
    ];
    settings.active_relay_id = "aggregate-test".to_string();
    settings.active_aggregate_relay_id = "aggregate-test".to_string();
    std::fs::write(&path, serde_json::to_string(&settings).unwrap()).unwrap();

    let loaded = SettingsStore::new(path.clone())
        .load()
        .expect("聚合档案带 configContents 时 load 不应递归爆栈");
    assert_eq!(loaded.relay_profiles.len(), 2);
    assert!(loaded.relay_profiles.iter().any(|profile| profile.id == "aggregate-test"));

    // 归一化仍然生效：成员档案的 base_url/key 等字段可用（这里只要求不崩且档案完整）。
    let again = SettingsStore::new(path).load().expect("重复 load 同样安全");
    assert_eq!(again.relay_profiles.len(), 2);

    set_settings_path_for_tests(previous);
}
