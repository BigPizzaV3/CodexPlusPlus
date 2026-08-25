use codex_plus_core::settings::{
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};

fn profile(id: &str, model: &str, base: &str) -> RelayProfile {
    RelayProfile {
        id: id.into(),
        name: id.into(),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::Responses,
        model: model.into(),
        upstream_base_url: base.into(),
        config_contents: format!(
            "model = \"{model}\"\nmodel_provider = \"custom\"\n\n[model_providers.custom]\nname = \"{id}\"\nbase_url = \"{base}\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n"
        ),
        auth_contents: format!("{{\"OPENAI_API_KEY\":\"sk-{id}\"}}"),
        ..RelayProfile::default()
    }
}

/// 切换供应商时，上一个供应商的配置必须原样保留。
///
/// #1970 报告：添加第二个供应商并切过去之后，回到管理器发现**第一个**供应商的
/// 模型、Base URL、config.toml 里混进了第二个供应商的信息。这条按那个复现步骤
/// 走一遍后端切换链路，钉住「切换不会串配置」。
#[test]
fn switching_providers_does_not_contaminate_the_previous_profile() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let store = SettingsStore::new(temp.path().join("settings.json"));

    let longcat = profile("longcat", "LongCat-Flash", "https://longcat.example/v1");
    let deepseek = profile("deepseek", "deepseek-v4", "https://deepseek.example/v1");

    let mut settings = BackendSettings {
        relay_profiles_enabled: true,
        relay_profiles: vec![longcat.clone(), deepseek.clone()],
        active_relay_id: "longcat".into(),
        ..BackendSettings::default()
    };

    // 1) 先切到 longcat（模拟用户第一步）
    let r = codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &store,
        home,
        settings.clone(),
        "",
    );
    settings = r.expect("切到 longcat").settings;
    let live1 = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(live1.contains("longcat.example"), "live 应该是 longcat 的");

    // 2) 切到 deepseek（模拟用户第二步）
    settings.active_relay_id = "deepseek".into();
    let r = codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &store, home, settings, "longcat",
    );
    let settings = r.expect("切到 deepseek").settings;
    let live2 = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(
        live2.contains("deepseek.example"),
        "live 应该是 deepseek 的"
    );

    // 3) 检查 longcat 有没有被 deepseek 污染
    let lc = settings
        .relay_profiles
        .iter()
        .find(|p| p.id == "longcat")
        .unwrap();
    assert!(
        !lc.config_contents.contains("deepseek"),
        "longcat 的配置被 deepseek 污染了"
    );
    assert!(
        lc.config_contents.contains("longcat.example"),
        "longcat 丢失了自己的 base_url"
    );
    assert_eq!(lc.model, "LongCat-Flash");
    assert_eq!(lc.upstream_base_url, "https://longcat.example/v1");

    // 反向也要成立：deepseek 不该被 longcat 污染
    let ds = settings
        .relay_profiles
        .iter()
        .find(|p| p.id == "deepseek")
        .unwrap();
    assert!(!ds.config_contents.contains("longcat"));
}
