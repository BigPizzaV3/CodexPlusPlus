use codex_plus_core::settings::{AgentPetSettings, BackendSettings, SettingsStore};
use serde_json::{Value, json};

fn store_with(contents: Value) -> (tempfile::TempDir, std::path::PathBuf, SettingsStore) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();
    let store = SettingsStore::new(path.clone());
    (temp, path, store)
}

fn saved(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn agent_pet_defaults_serialize_as_camel_case() {
    let settings = BackendSettings::default();
    assert_eq!(settings.agent_pet, AgentPetSettings::default());
    assert_eq!(
        serde_json::to_value(&settings).unwrap()["agentPet"],
        json!({
            "enabled": false,
            "agents": ["codex"],
            "image": "whale",
            "customImageUrl": "",
            "size": 112,
            "soundEnabled": true,
            "soundSet": "duck",
            "customSoundPress": "",
            "customSoundRelease": "",
            "volume": 0.6,
        })
    );
    let parsed: BackendSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(parsed.agent_pet, AgentPetSettings::default());
}

#[test]
fn agent_pet_round_trips_through_settings_store() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());
    let mut settings = store.load().unwrap();
    settings.agent_pet = AgentPetSettings {
        enabled: true,
        agents: vec!["codex".into()],
        image: "custom".into(),
        custom_image_url: "https://example.com/pet.webp".into(),
        size: 180,
        sound_enabled: false,
        sound_set: "custom".into(),
        custom_sound_press: "press-0123456789abcdef.mp3".into(),
        custom_sound_release: "release-fedcba9876543210.wav".into(),
        volume: 0.25,
    };
    store.save(&settings).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.agent_pet, settings.agent_pet);
    assert_eq!(saved(&path)["agentPet"]["customImageUrl"], "https://example.com/pet.webp");
    assert_eq!(saved(&path)["agentPet"]["soundSet"], "custom");
    assert_eq!(saved(&path)["agentPet"]["customSoundPress"], "press-0123456789abcdef.mp3");
}

#[test]
fn agent_pet_normalizes_out_of_range_and_unknown_values() {
    let parsed: BackendSettings = serde_json::from_value(json!({
        "agentPet": {
            "enabled": true,
            "agents": ["claude", "CODEX", "codex", "grok"],
            "image": "dragon",
            "customImageUrl": "  https://example.com/a.png  ",
            "size": 999,
            "soundSet": "bell",
            "volume": 3.5,
        }
    }))
    .unwrap();
    let pet = parsed.agent_pet;
    assert!(pet.enabled);
    assert_eq!(pet.agents, vec!["codex"]);
    assert_eq!(pet.image, "whale");
    assert_eq!(pet.custom_image_url, "https://example.com/a.png");
    assert_eq!(pet.size, 240);
    assert!(pet.sound_enabled);
    assert_eq!(pet.sound_set, "duck");
    assert_eq!(pet.volume, 1.0);

    let small: AgentPetSettings =
        serde_json::from_value(json!({"size": 10, "volume": -1, "image": "custom"})).unwrap();
    assert_eq!(small.size, 64);
    assert_eq!(small.volume, 0.0);
    assert_eq!(small.image, "custom");

    // 类型写错只回落该字段，不让整份设置解析失败。
    let broken: BackendSettings = serde_json::from_value(json!({
        "codexAppConversationView": true,
        "agentPet": {"enabled": "yes", "agents": "codex", "size": -5, "volume": "loud"}
    }))
    .unwrap();
    assert!(broken.codex_app_conversation_view);
    assert!(!broken.agent_pet.enabled);
    assert_eq!(broken.agent_pet.agents, vec!["codex"]);
    assert_eq!(broken.agent_pet.size, 64);
    assert_eq!(broken.agent_pet.volume, 0.6);
    let not_object: BackendSettings = serde_json::from_value(json!({"agentPet": 1})).unwrap();
    assert_eq!(not_object.agent_pet, AgentPetSettings::default());

    let mut nan = AgentPetSettings {
        volume: f64::NAN,
        ..AgentPetSettings::default()
    };
    nan.normalize();
    assert_eq!(nan.volume, 0.6);

    // 取消勾选全部 Agent 时保留空列表，桌宠在哪都不显示。
    let none: AgentPetSettings = serde_json::from_value(json!({"enabled": true, "agents": []})).unwrap();
    assert!(none.agents.is_empty());
    assert!(!none.enabled_for("codex"));
}

#[test]
fn agent_pet_partial_update_merges_and_normalizes() {
    let (_temp, path, store) = store_with(json!({}));
    let updated = store
        .update(json!({"agentPet": {"enabled": true, "image": "fox", "size": 150.4}}))
        .unwrap();
    assert!(updated.agent_pet.enabled);
    assert_eq!(updated.agent_pet.image, "fox");
    assert_eq!(updated.agent_pet.size, 150);
    let updated = store
        .update(json!({"agentPet": {"volume": 2, "agents": ["claude"], "soundSet": "fx1"}}))
        .unwrap();
    assert!(updated.agent_pet.enabled, "未出现在补丁里的字段保持不变");
    assert_eq!(updated.agent_pet.image, "fox");
    assert_eq!(updated.agent_pet.volume, 1.0);
    assert!(updated.agent_pet.agents.is_empty());
    assert_eq!(updated.agent_pet.sound_set, "fx1");
    assert_eq!(store.load().unwrap().agent_pet, updated.agent_pet);
    let saved = saved(&path);
    assert_eq!(saved["agentPet"]["size"], 150);
    assert_eq!(saved["agentPet"]["agents"], json!([]));
}

#[test]
fn legacy_widget_switch_migrates_to_agent_pet() {
    let legacy = json!({
        "codexAppWhaleWidgetEnabled": true,
        "codexAppWhaleBalanceProtocol": "custom",
        "codexAppWhaleBalancePath": "/api/balance",
        "codexAppWhaleBalanceField": "data.balance",
        "codexAppWhaleBalanceCurrency": "CNY",
        "codexAppWhaleBalanceScale": 0.5,
    });

    // 读取：开着旧开关、没有 agentPet → 桌宠开启，其余字段为默认值。
    let (_temp, path, store) = store_with(legacy.clone());
    let loaded = store.load().unwrap();
    assert_eq!(
        loaded.agent_pet,
        AgentPetSettings {
            enabled: true,
            ..AgentPetSettings::default()
        }
    );
    // 整体保存：旧键全部消失，agentPet 写出。
    store.save(&loaded).unwrap();
    let after_save = saved(&path);
    for key in [
        "codexAppWhaleWidgetEnabled",
        "codexAppWhaleBalanceProtocol",
        "codexAppWhaleBalancePath",
        "codexAppWhaleBalanceField",
        "codexAppWhaleBalanceCurrency",
        "codexAppWhaleBalanceScale",
    ] {
        assert!(after_save.get(key).is_none(), "{key} should be dropped");
    }
    assert_eq!(after_save["agentPet"]["enabled"], true);

    // 局部更新路径同样迁移并清理旧键。
    let (_temp, path, store) = store_with(legacy);
    let updated = store.update(json!({"codexAppThreadIdBadge": true})).unwrap();
    assert!(updated.agent_pet.enabled);
    let after_update = saved(&path);
    assert!(after_update.get("codexAppWhaleWidgetEnabled").is_none());
    assert!(after_update.get("codexAppWhaleBalanceScale").is_none());
    assert_eq!(after_update["agentPet"]["enabled"], true);
    assert_eq!(after_update["agentPet"]["image"], "whale");
}

#[test]
fn legacy_widget_switch_does_not_override_existing_agent_pet() {
    let (_temp, _path, store) = store_with(json!({
        "codexAppWhaleWidgetEnabled": true,
        "agentPet": {"enabled": false, "image": "cat"},
    }));
    let loaded = store.load().unwrap();
    assert!(!loaded.agent_pet.enabled);
    assert_eq!(loaded.agent_pet.image, "cat");
    let updated = store.update(json!({})).unwrap();
    assert!(!updated.agent_pet.enabled);

    // 旧开关关着时不迁移。
    let (_temp, _path, store) = store_with(json!({"codexAppWhaleWidgetEnabled": false}));
    assert!(!store.load().unwrap().agent_pet.enabled);
}

#[test]
fn agent_pet_custom_sound_files_must_match_import_names() {
    let parsed: AgentPetSettings = serde_json::from_value(json!({
        "soundSet": " custom ",
        "customSoundPress": " press-0123456789abcdef.ogg ",
        "customSoundRelease": "../release-0123456789abcdef.mp3",
    }))
    .unwrap();
    assert_eq!(parsed.sound_set, "custom");
    assert_eq!(parsed.custom_sound_press, "press-0123456789abcdef.ogg");
    assert_eq!(parsed.custom_sound_release, "", "不符合导入命名的文件名一律清空");

    for bad in [
        "release-0123456789abcdef.mp3",
        "press-0123456789ABCDEF.mp3",
        "press-0123456789abcde.mp3",
        "press-0123456789abcdef.exe",
        "/etc/press-0123456789abcdef.mp3",
    ] {
        let parsed: AgentPetSettings =
            serde_json::from_value(json!({"customSoundPress": bad})).unwrap();
        assert_eq!(parsed.custom_sound_press, "", "{bad}");
    }
}
