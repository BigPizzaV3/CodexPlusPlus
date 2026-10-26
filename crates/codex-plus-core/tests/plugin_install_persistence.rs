use codex_plus_core::relay_config::{
    apply_relay_config_file_to_home, cleanup_legacy_official_marketplace_overrides_in_home,
};

#[test]
fn legacy_official_market_registration_is_removed_without_changing_plugins() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let root = home.join(".tmp").join("plugins");
    let config = format!(
        "# User settings\nmodel = 'test'\n\
         [marketplaces.openai-curated]\nsource_type = 'local'\nsource = '{}'\n\
         [marketplaces.openai-api-curated]\nsource_type = 'local'\nsource = '\\\\?\\{}'\n\
         [marketplaces.custom]\nsource_type = 'local'\nsource = '{}'\n\
         [plugins.'github@openai-api-curated']\nenabled = true\n\
         [plugins.'zotero@openai-api-curated']\nenabled = false\n",
        root.display(),
        root.display(),
        root.display()
    );
    std::fs::write(home.join("config.toml"), &config).unwrap();
    assert!(cleanup_legacy_official_marketplace_overrides_in_home(home).unwrap());
    let repaired = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let doc = repaired.parse::<toml_edit::DocumentMut>().unwrap();
    assert!(doc["marketplaces"].get("openai-curated").is_none());
    assert!(doc["marketplaces"].get("openai-api-curated").is_none());
    assert!(doc["marketplaces"].get("custom").is_some());
    assert_eq!(
        doc["plugins"]["github@openai-api-curated"]["enabled"].as_bool(),
        Some(true)
    );
    assert_eq!(
        doc["plugins"]["zotero@openai-api-curated"]["enabled"].as_bool(),
        Some(false)
    );
    assert!(repaired.contains("# User settings"));
    assert!(!cleanup_legacy_official_marketplace_overrides_in_home(home).unwrap());
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        repaired
    );
}

#[test]
fn user_official_named_market_with_different_source_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let original =
        "[marketplaces.openai-api-curated]\nsource_type = 'local'\nsource = '/custom/plugins'\n";
    std::fs::write(temp.path().join("config.toml"), original).unwrap();
    assert!(!cleanup_legacy_official_marketplace_overrides_in_home(temp.path()).unwrap());
    assert_eq!(
        std::fs::read_to_string(temp.path().join("config.toml")).unwrap(),
        original
    );
}

#[test]
fn applying_stale_template_does_not_restore_legacy_official_market_registration() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join(".tmp").join("plugins");
    let stale = format!(
        "model = 'test'\n[marketplaces.openai-api-curated]\nsource_type = 'local'\nsource = '{}'\n\
         [plugins.'github@openai-api-curated']\nenabled = true\n",
        root.display()
    );
    std::fs::write(temp.path().join("config.toml"), &stale).unwrap();
    apply_relay_config_file_to_home(temp.path(), &stale).unwrap();
    let written = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    assert!(!written.contains("[marketplaces.openai-api-curated]"));
    assert!(written.contains("[plugins."));
    assert!(written.contains("github@openai-api-curated"));
}

#[test]
fn invalid_config_is_not_overwritten_or_returned_in_error() {
    let temp = tempfile::tempdir().unwrap();
    let original = "private_test_value = 'never-echo-this";
    std::fs::write(temp.path().join("config.toml"), original).unwrap();
    let error = cleanup_legacy_official_marketplace_overrides_in_home(temp.path()).unwrap_err();
    assert!(!error.to_string().contains("never-echo-this"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("config.toml")).unwrap(),
        original
    );
}

#[test]
fn stale_template_is_sanitized_without_an_existing_live_config() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join(".tmp").join("plugins");
    let stale = format!(
        "model = 'test'\n[marketplaces.openai-api-curated]\nsource_type = 'local'\nsource = '{}'\n",
        root.display()
    );
    apply_relay_config_file_to_home(temp.path(), &stale).unwrap();
    let written = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    assert!(!written.contains("[marketplaces.openai-api-curated]"));
}

#[test]
fn corrupt_live_config_does_not_bypass_stale_template_cleanup() {
    for plugin in ["github@openai-api-curated", "github@openai-curated"] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("config.toml"), "[invalid\n").unwrap();
        let stale = format!(
            "model = 'test'\n[marketplaces.openai-api-curated]\nsource_type = 'local'\nsource = '{}'\n\
             [plugins.'{plugin}']\nenabled = true\n",
            temp.path().join(".tmp/plugins").display()
        );
        apply_relay_config_file_to_home(temp.path(), &stale).unwrap();
        let written = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
        let doc = written.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(doc.get("marketplaces").and_then(|item| item.get("openai-api-curated")).is_none());
        assert_eq!(doc["plugins"][plugin]["enabled"].as_bool(), Some(true));
    }
}
