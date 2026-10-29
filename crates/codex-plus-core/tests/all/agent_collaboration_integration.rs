use std::path::PathBuf;

use codex_plus_core::agent_collaboration::integration;

fn executable() -> PathBuf {
    std::env::current_exe().unwrap()
}

#[test]
fn explicit_install_preserves_other_configuration_and_is_idempotent_with_private_backup() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("shared state with spaces");
    let path = home.path().join("config.toml");
    let original = "# keep this comment\nmodel = \"chosen-model\" # user choice\n\n[mcp_servers.other]\ncommand = \"other-server\"\n\n[model_providers.user]\nname = \"My provider\"\nbase_url = \"https://example.invalid/v1\"\n";
    std::fs::write(&path, original).unwrap();
    assert_eq!(
        integration::status(home.path(), &executable(), &state)
            .unwrap()
            .status,
        "missing"
    );
    let first = integration::install(home.path(), &executable(), &state).unwrap();
    assert!(first.changed);
    assert!(first.status.registered);
    let backup = PathBuf::from(first.backup_path.as_ref().unwrap());
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("# keep this comment"));
    assert!(written.contains("# user choice"));
    let doc = written.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(doc["model"].as_str(), Some("chosen-model"));
    assert_eq!(
        doc["mcp_servers"]["other"]["command"].as_str(),
        Some("other-server")
    );
    assert_eq!(
        doc["model_providers"]["user"]["name"].as_str(),
        Some("My provider")
    );
    assert_eq!(
        doc["mcp_servers"][integration::SERVER_ID]["args"][2].as_str(),
        state.to_str()
    );
    let second = integration::install(home.path(), &executable(), &state).unwrap();
    assert!(!second.changed);
    assert!(second.backup_path.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
    assert_eq!(
        std::fs::read_dir(backup.parent().unwrap()).unwrap().count(),
        1
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(backup.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[test]
fn existing_owned_server_updates_only_its_launch_fields_and_keeps_user_options() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let source = "# context comment\n[mcp_servers.codex_plus_collaboration]\n# launch comment\ncommand = \"/old/manager\" # keep command note\nargs = [\"--codex-plus-agent-mcp\", \"--state-dir\", \"/old/state\"]\nenabled = false\napproval_mode = \"manual\"\n\n[mcp_servers.codex_plus_collaboration.env]\nCUSTOM_OPTION = \"keep-value\"\n\n[mcp_servers.other]\ncommand = \"keep\"\n";
    std::fs::write(home.path().join("config.toml"), source).unwrap();
    assert_eq!(
        integration::status(home.path(), &executable(), &state)
            .unwrap()
            .status,
        "needs_update"
    );
    let updated = integration::upsert_managed_context(source, &executable(), &state).unwrap();
    let doc = updated.parse::<toml_edit::DocumentMut>().unwrap();
    let entry = &doc["mcp_servers"][integration::SERVER_ID];
    assert_eq!(entry["command"].as_str(), executable().to_str());
    assert_eq!(entry["approval_mode"].as_str(), Some("manual"));
    assert_eq!(entry["env"]["CUSTOM_OPTION"].as_str(), Some("keep-value"));
    assert_eq!(entry["enabled"].as_bool(), Some(true));
    assert!(updated.contains("# launch comment"));
    assert!(updated.contains("# keep command note"));
    assert_eq!(
        doc["mcp_servers"]["other"]["command"].as_str(),
        Some("keep")
    );
    integration::install(home.path(), &executable(), &state).unwrap();
    assert!(
        integration::status(home.path(), &executable(), &state)
            .unwrap()
            .registered
    );
}

#[test]
fn foreign_same_name_and_invalid_toml_are_never_overwritten_or_echoed() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let path = home.path().join("config.toml");
    let foreign = "[mcp_servers.codex_plus_collaboration]\ncommand = \"foreign\"\nargs = [\"not-our-server\"]\n";
    std::fs::write(&path, foreign).unwrap();
    assert_eq!(
        integration::status(home.path(), &executable(), &state)
            .unwrap()
            .status,
        "conflict"
    );
    assert!(integration::install(home.path(), &executable(), &state).is_err());
    assert!(integration::upsert_managed_context(foreign, &executable(), &state).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), foreign);
    assert!(!home.path().join(".codex-plus-agent-mcp-backups").exists());
    let malformed = "api_key = \"FAKE_FIXTURE_DO_NOT_ECHO\" [broken\n";
    std::fs::write(&path, malformed).unwrap();
    let error = integration::install(home.path(), &executable(), &state).unwrap_err();
    assert!(!format!("{error:#}").contains("FAKE_FIXTURE_DO_NOT_ECHO"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);
}

#[test]
fn owned_marker_with_http_transport_is_a_conflict_and_never_claims_to_be_registered() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let mut source = integration::upsert_managed_context("", &executable(), &state).unwrap();
    source.push_str("url = \"https://example.invalid/mcp\"\n");
    let path = home.path().join("config.toml");
    std::fs::write(&path, &source).unwrap();
    let status = integration::status(home.path(), &executable(), &state).unwrap();
    assert_eq!(status.status, "conflict");
    assert!(!status.registered);
    assert!(integration::install(home.path(), &executable(), &state).is_err());
    assert!(integration::upsert_managed_context(&source, &executable(), &state).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn inline_tables_remain_valid_and_existing_matching_arguments_keep_comments() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let inline = "model = \"keep\"\nmcp_servers = { other = { command = \"other\" } }\n";
    let updated = integration::upsert_managed_context(inline, &executable(), &state).unwrap();
    let doc = updated.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(
        doc["mcp_servers"]["other"]["command"].as_str(),
        Some("other")
    );
    assert!(
        doc["mcp_servers"][integration::SERVER_ID]["args"]
            .as_array()
            .is_some()
    );
    let expected = integration::upsert_managed_context("", &executable(), &state).unwrap();
    let expected = expected.replacen("args = [", "args = [ # keep array note\n", 1);
    assert_eq!(
        integration::upsert_managed_context(&expected, &executable(), &state).unwrap(),
        expected
    );
}

#[test]
fn managed_registration_survives_the_existing_relay_apply_path() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let managed = integration::upsert_managed_context("", &executable(), &state).unwrap();
    // 接入保存进 relay_context_config_contents 后，既有 apply 流程负责重新合并它。
    codex_plus_core::relay_config::apply_relay_files_to_home_with_context(
        home.path(),
        "model = \"first\"\n",
        "{}",
        &managed,
        "",
        "",
    )
    .unwrap();
    codex_plus_core::relay_config::apply_relay_files_to_home_with_context(
        home.path(),
        "model = \"second\"\n",
        "{}",
        &managed,
        "",
        "",
    )
    .unwrap();
    let status = integration::status(home.path(), &executable(), &state).unwrap();
    assert!(status.registered);
    let doc = std::fs::read_to_string(home.path().join("config.toml"))
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    assert_eq!(doc["model"].as_str(), Some("second"));
}

#[test]
fn registration_rejects_relative_paths_and_does_not_replace_a_symlinked_config() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    assert!(integration::install(home.path(), std::path::Path::new("manager"), &state).is_err());
    assert!(
        integration::install(home.path(), &executable(), std::path::Path::new("relative")).is_err()
    );
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("external.toml");
        std::fs::write(&target, "model = \"unchanged\"\n").unwrap();
        std::os::unix::fs::symlink(&target, home.path().join("config.toml")).unwrap();
        assert!(integration::install(home.path(), &executable(), &state).is_err());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "model = \"unchanged\"\n"
        );
    }
}
