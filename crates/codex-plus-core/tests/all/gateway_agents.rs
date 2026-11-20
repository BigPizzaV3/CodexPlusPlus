//! 通用 Agent（`gateway::agents` 规格表）的接入与还原：每条规格都过一遍「接入 → 配置指向网关 → 断开 → 一字不差还原」。

use codex_plus_core::gateway::agents::{self, AgentEnv};
use codex_plus_core::gateway::apply::{
    ApplyContext, ProviderInput, create_provider, disable_binding, enable_binding,
};
use codex_plus_core::gateway::store::{AgentBinding, GatewayStore};
use codex_plus_core::settings::{BackendSettings, RelayProtocol};
use codex_plus_core::tools::ToolId;

struct Fixture {
    _temp: tempfile::TempDir,
    ctx: ApplyContext,
    home: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let ctx = ApplyContext {
        gateway: GatewayStore::at(state.join("gateway.json")),
        claude: codex_plus_core::tools::claude::ClaudeContext {
            store_path: state.join("claude-providers.json"),
            claude_home: temp.path().join("claude"),
            backup_root: state.join("backups"),
        },
        grok_home: temp.path().join("grok"),
        codex_home: temp.path().join("codex"),
        settings: codex_plus_core::settings::SettingsStore::new(state.join("settings.json")),
        backup_root: state.join("backups"),
        port: 57321,
        agent_env: AgentEnv::at(&home),
    };
    create_provider(
        &ctx.gateway,
        &ProviderInput {
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.test/v1".into(),
            protocol: RelayProtocol::ChatCompletions,
            keys: vec!["sk-ds".into()],
            model: "deepseek-v4-pro".into(),
            model_list: "deepseek-v4-pro\ndeepseek-v4-flash".into(),
            price: None,
            model_prices: None,
            proxy: None,
            strip_image_generation_tool: None,
            subscription_policy: None,
            strip_responses_image_detail: None,
            disable_tool_search: None,
            custom_headers: None,
        },
        "preset:deepseek",
        None,
    )
    .unwrap();
    Fixture {
        _temp: temp,
        ctx,
        home,
    }
}

fn binding() -> AgentBinding {
    AgentBinding {
        model: "deepseek-v4-pro".into(),
        context_window: Some(128_000),
        ..AgentBinding::default()
    }
}

/// 目录下全部文件的 (相对路径, 内容)。
fn snapshot(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root).unwrap().to_string_lossy().to_string();
                out.push((relative, std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn editor_preserves_user_fields_through_gateway_refresh_and_disconnect() {
    use agents::editor::config;
    for (id, key, value, forbidden) in [
        ("pi", "theme", serde_json::json!("light"), "defaultProvider"),
        (
            "opencode",
            "autoupdate",
            serde_json::json!(false),
            "provider",
        ),
    ] {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        let agent = ToolId::Unknown(id.to_owned());
        let path = (agents::spec(id).unwrap().config_path)(&ctx.agent_env);
        let original_gateway = ctx.gateway.load().unwrap();
        let initial = config(ctx, id, None).unwrap();
        assert!(!path.exists());
        let draft = serde_json::json!({(key): value});
        let saved = config(ctx, id, Some((&initial.snapshot, &draft.to_string()))).unwrap();
        assert_eq!(
            ctx.gateway.load().unwrap().bindings,
            original_gateway.bindings
        );
        enable_binding(ctx, &agent, binding()).unwrap();
        let before = std::fs::read(&path).unwrap();
        for invalid in [
            "[".to_owned(),
            serde_json::json!({(forbidden): "override"}).to_string(),
        ] {
            assert!(config(ctx, id, Some((&saved.snapshot, &invalid))).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
        let changed_value = if id == "pi" {
            serde_json::json!("dark")
        } else {
            serde_json::json!(true)
        };
        let changed = serde_json::json!({(key): changed_value});
        let bindings = ctx.gateway.load().unwrap().bindings;
        config(ctx, id, Some((&saved.snapshot, &changed.to_string()))).unwrap();
        assert!(config(ctx, id, Some((&saved.snapshot, "{}"))).is_err());
        assert_eq!(ctx.gateway.load().unwrap().bindings, bindings);
        codex_plus_core::gateway::apply::refresh_enabled(ctx).unwrap();
        assert!(agents::connect::points_at_gateway(
            ctx,
            agents::spec(id).unwrap()
        ));
        let read = || agents::files::parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read()[key], changed_value);
        disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
        assert_eq!(read(), changed);
    }
}

#[test]
fn editor_rejects_switched_target_with_identical_preferences() {
    use agents::editor::config;
    for (id, initial, draft) in [
        ("hermes", "display:\n  skin: dark\n", "skin: light"),
        ("omp", "theme: dark\n", "theme: light"),
    ] {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        let resolved = (agents::spec(id).unwrap().config_path)(&ctx.agent_env);
        let dir = resolved.parent().unwrap();
        let (first, second) = if id == "hermes" {
            (resolved.clone(), dir.join("profiles/work/config.yaml"))
        } else {
            (dir.join("config.yaml"), dir.join("config.yml"))
        };
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::write(&first, initial).unwrap();
        let before = config(ctx, id, None).unwrap();
        std::fs::create_dir_all(second.parent().unwrap()).unwrap();
        std::fs::write(&second, initial).unwrap();
        if id == "hermes" {
            std::fs::write(dir.join("active_profile"), "work\n").unwrap();
        }
        let after = config(ctx, id, None).unwrap();
        assert_ne!(before.snapshot.source, after.snapshot.source);
        assert_eq!(before.snapshot.contents, after.snapshot.contents);
        let stale = config(ctx, id, Some((&before.snapshot, draft)));
        assert_eq!(
            std::fs::read_to_string(&second).unwrap(),
            initial,
            "{id}: stale save wrote the new target"
        );
        assert!(stale.is_err(), "{id}");
        assert_eq!(std::fs::read_to_string(&first).unwrap(), initial);
        config(ctx, id, Some((&after.snapshot, draft))).unwrap();
        assert!(std::fs::read_to_string(&second).unwrap().contains("light"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), initial);
    }
}

#[test]
fn editor_vscode_writes_only_default_user_settings() {
    use agents::editor::config;
    let fixture = fixture();
    let ctx = &fixture.ctx;
    for id in ["vscode", "vscode-insiders", "vscodium"] {
        let models = (agents::spec(id).unwrap().config_path)(&ctx.agent_env);
        let user_dir = models.parent().unwrap();
        let default_settings = user_dir.join("settings.json");
        let profile_settings = user_dir.join("profiles/work/settings.json");
        let initial = r#"{"editor.fontSize":12}"#;
        std::fs::create_dir_all(profile_settings.parent().unwrap()).unwrap();
        std::fs::write(&profile_settings, initial).unwrap();
        std::fs::write(&default_settings, initial).unwrap();
        let before = config(ctx, id, None).unwrap();
        assert!(before.snapshot.source.contains("默认 User/settings.json"));
        assert!(
            before
                .snapshot
                .source
                .contains(default_settings.to_str().unwrap())
        );
        config(
            ctx,
            id,
            Some((&before.snapshot, r#"{"editor.fontSize":16}"#)),
        )
        .unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(default_settings).unwrap()).unwrap();
        assert_eq!(saved["editor.fontSize"], 16);
        assert_eq!(std::fs::read_to_string(profile_settings).unwrap(), initial);
        assert!(!models.exists());
    }
}

#[test]
fn editor_opens_every_adapter_without_connecting_or_creating_files() {
    let fixture = fixture();
    let before = snapshot(&fixture.home);
    let ctx = &fixture.ctx;
    let bindings = ctx.gateway.load().unwrap().bindings;
    for id in agents::all().iter().map(|spec| spec.id).chain([
        "codex",
        "claude",
        "grok",
        "claude-desktop-local",
    ]) {
        let view =
            agents::editor::config(ctx, id, None).unwrap_or_else(|error| panic!("{id}: {error:#}"));
        assert!(!view.snapshot.source.is_empty(), "{id}");
        if !view.editable {
            assert!(agents::editor::config(ctx, id, Some((&view.snapshot, "{}"))).is_err());
        }
    }
    assert_eq!(bindings, ctx.gateway.load().unwrap().bindings);
    assert_eq!(before, snapshot(&fixture.home));
}

#[test]
fn editor_keeps_unrelated_toml_yaml_and_nested_json_fields() {
    let fixture = fixture();
    let ctx = &fixture.ctx;
    for (id, input, draft, excluded, expected) in [
        (
            "stepcode",
            "# setup\ntheme = 'dark'\n[providers.mine]\napi_key = 'fixture'\n",
            "theme = 'light'",
            "api_key",
            "light",
        ),
        (
            "omp",
            "# setup\ntheme: dark\nmodelRoles:\n  default: fixture\n",
            r#"{"theme":"light"}"#,
            "modelRoles",
            "light",
        ),
        (
            "gemini-cli",
            "{\n// setup\n\"security\":{\"fixture\":true},\"ui\":{\"hideBanner\":false}}",
            r#"{"hideBanner":true}"#,
            "security",
            "true",
        ),
        (
            "openchamber",
            r#"{"version":1,"fields":{"theme":{"value":"dark","updatedAt":1},"defaultModel":{"value":"fixture"}}}"#,
            r#"{"theme":"light"}"#,
            "defaultModel",
            "light",
        ),
    ] {
        let path = (agents::spec(id).unwrap().config_path)(&ctx.agent_env);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, input).unwrap();
        let view = agents::editor::config(ctx, id, None).unwrap();
        assert!(!view.snapshot.contents.contains(excluded));
        assert!(
            agents::editor::config(ctx, id, Some((&view.snapshot, r#"{"model":"x"}"#))).is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), input);
        agents::editor::config(ctx, id, Some((&view.snapshot, draft))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(excluded) && text.contains("fixture"));
        if input.contains("setup") {
            assert!(text.contains("setup"));
        }
        assert!(
            agents::editor::config(ctx, id, None)
                .unwrap()
                .snapshot
                .contents
                .contains(expected)
        );
    }
}

#[test]
fn editor_preserves_siblings_and_rejects_ambiguous_drafts() {
    use agents::editor::config;
    let fixture = fixture();
    let ctx = &fixture.ctx;
    for (id, original, updated, draft, invalid) in [
        (
            "jcode",
            "[ui]\ntheme  =  'dark' # mine\n[provider]\nname = 'mine'\n",
            "[ui]\ntheme  =  \"light\" # mine\n[provider]\nname = 'mine'\n",
            "theme = 'light'",
            "theme = 'a'\ntheme = 'b'",
        ),
        (
            "gemini-cli",
            "{\r\n  // mine\r\n  \"ui\": {\"theme\": \"dark\", \"other\": [1, 2,],},\r\n}\r\n",
            "{\r\n  // mine\r\n  \"ui\": {\"theme\": \"light\", \"other\": [1, 2,],},\r\n}\r\n",
            "{/* draft */ \"theme\": \"light\",}",
            "{\"theme\":\"a\",\"theme\":\"b\"}",
        ),
    ] {
        let path = (agents::spec(id).unwrap().config_path)(&ctx.agent_env);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, original).unwrap();
        let view = config(ctx, id, None).unwrap();
        assert!(
            !view.snapshot.contents.contains("mine") && view.snapshot.contents.contains("dark")
        );
        assert!(config(ctx, id, Some((&view.snapshot, invalid))).is_err());
        if id == "gemini-cli" {
            assert!(config(ctx, id, Some((&view.snapshot, "// no object"))).is_err());
            assert!(config(ctx, id, Some((&view.snapshot, "{} /* unclosed"))).is_err());
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let saved = config(ctx, id, Some((&view.snapshot, draft))).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        assert_eq!(
            config(ctx, id, None).unwrap().snapshot.contents,
            saved.snapshot.contents
        );
    }
}

#[test]
fn editor_round_trips_quoted_yaml_strings_without_coercion() {
    use agents::editor::config;
    use serde_json::{Value, json};

    let fixture = fixture();
    let ctx = &fixture.ctx;
    let path = (agents::spec("omp").unwrap().config_path)(&ctx.agent_env);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let model_roles = "modelRoles:\n  default: fixture\n";
    for (literal, expected) in [
        (r#""123""#, "123"),
        ("'true' # theme name", "true"),
        ("'it''s #1'", "it's #1"),
        (r#""[dark] \"blue\"" # comment"#, "[dark] \"blue\""),
    ] {
        std::fs::write(&path, format!("theme: {literal}\n{model_roles}")).unwrap();
        let view = config(ctx, "omp", None).unwrap();
        assert_eq!(
            serde_saphyr::from_str::<Value>(&view.snapshot.contents).unwrap(),
            json!({"theme": expected})
        );
        let draft = json!({"theme": "false"});
        config(ctx, "omp", Some((&view.snapshot, &draft.to_string()))).unwrap();
        assert_eq!(
            serde_saphyr::from_str::<Value>(&config(ctx, "omp", None).unwrap().snapshot.contents)
                .unwrap(),
            draft
        );
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            serde_saphyr::from_str::<Value>(&saved).unwrap()["theme"],
            "false"
        );
        assert!(saved.contains(model_roles));
    }
    let previous = config(ctx, "omp", None).unwrap();
    for invalid in ["123", "true", "'unterminated", "\"valid\" trailing"] {
        let original = format!("theme: {invalid}\n{model_roles}");
        std::fs::write(&path, &original).unwrap();
        assert!(config(ctx, "omp", None).is_err());
        assert!(config(ctx, "omp", Some((&previous.snapshot, "{}"))).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}

#[test]
fn native_preferences_survive_gateway_refresh_and_disconnect() {
    use agents::editor::config;
    for (id, input, first, second, expected, untouched) in [
        (
            "hermes",
            "# personal\ndisplay:\n  skin: night # my setting\n  language: en\nnotes: &notes [one, two]\ncopy: *notes\n",
            "skin: day\nlanguage: zh",
            "skin: dusk\nlanguage: zh",
            "dusk",
            "notes: &notes [one, two]\ncopy: *notes",
        ),
        (
            "openclaw",
            "{\n  // personal\n  ui: {prefs: {locale: 'en',},},\n  untouched: [0xff, 'mine'],\n}\n",
            "{locale:'zh',}",
            "{locale:'fr',}",
            "fr",
            "untouched: [0xff, 'mine']",
        ),
    ] {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        let agent = ToolId::Unknown(id.into());
        let path = (agents::spec(id).unwrap().config_path)(&ctx.agent_env);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, input).unwrap();
        let view = config(ctx, id, None).unwrap();
        let saved = config(ctx, id, Some((&view.snapshot, first))).unwrap();
        enable_binding(ctx, &agent, binding()).unwrap();
        config(ctx, id, Some((&saved.snapshot, second))).unwrap();
        codex_plus_core::gateway::apply::refresh_enabled(ctx).unwrap();
        assert!(agents::connect::points_at_gateway(
            ctx,
            agents::spec(id).unwrap()
        ));
        disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
        let source = std::fs::read_to_string(&path).unwrap();
        assert!(
            source.contains(untouched) && source.contains("personal"),
            "{source}"
        );
        assert!(!source.contains("127.0.0.1"));
        assert!(
            config(ctx, id, None)
                .unwrap()
                .snapshot
                .contents
                .contains(expected)
        );
    }
}

#[test]
fn native_preferences_can_create_a_missing_source_and_reject_bad_drafts() {
    use agents::editor::config;
    for (id, valid, invalid) in [
        (
            "openclaw",
            "{locale:'zh',}",
            ["{locale:'en',locale:'fr'}", "{model:'override'}"],
        ),
        ("hermes", "skin: night", ["skin: [", "model: override"]),
        (
            "jcode",
            "theme = 'light'",
            ["theme = [", "model = 'override'"],
        ),
    ] {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        let agent = ToolId::Unknown(id.into());
        let path = (agents::spec(id).unwrap().config_path)(&ctx.agent_env);
        let view = config(ctx, id, None).unwrap();
        let saved = config(ctx, id, Some((&view.snapshot, valid))).unwrap();
        let source = std::fs::read(&path).unwrap();
        for bad in invalid {
            assert!(config(ctx, id, Some((&saved.snapshot, bad))).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), source);
        }
        enable_binding(ctx, &agent, binding()).unwrap();
        disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), source);
    }
}

#[test]
fn yaml_edits_preserve_block_scalars_and_refuse_anchor_side_effects() {
    use agents::editor::config;
    let fixture = fixture();
    let ctx = &fixture.ctx;
    let path = (agents::spec("omp").unwrap().config_path)(&ctx.agent_env);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = "theme: &shared dark\nother: *shared\n";
    std::fs::write(&path, original).unwrap();
    let view = config(ctx, "omp", None).unwrap();
    assert!(config(ctx, "omp", Some((&view.snapshot, "theme: light"))).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    for broken in [
        "theme: [",
        "theme: dark\ntheme: light",
        "theme: one\n---\ntheme: two",
    ] {
        std::fs::write(&path, broken).unwrap();
        assert!(config(ctx, "omp", None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }
    std::fs::write(
        &path,
        "theme: |-\n  before\nmodelRoles: {default: untouched}\n",
    )
    .unwrap();
    let view = config(ctx, "omp", None).unwrap();
    config(
        ctx,
        "omp",
        Some((&view.snapshot, "theme: |-\n  first\n  second\n")),
    )
    .unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(source.contains("modelRoles: {default: untouched}"));
    assert_eq!(
        serde_saphyr::from_str::<serde_json::Value>(&source).unwrap()["theme"],
        "first\nsecond"
    );
}

#[test]
fn every_generic_agent_connects_and_restores_exactly() {
    for spec in agents::all() {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        let agent = ToolId::Unknown(spec.id.to_string());
        let before = snapshot(&fixture.home);

        enable_binding(ctx, &agent, binding())
            .unwrap_or_else(|error| panic!("{} 接入失败：{error:#}", spec.id));
        assert!(
            agents::connect::points_at_gateway(ctx, spec),
            "{} 接入后配置里找不到 /gw/{}",
            spec.id,
            spec.id
        );
        assert!(
            codex_plus_core::gateway::drift::check(ctx, &agent).is_none(),
            "{} 刚接入就报配置被改",
            spec.id
        );
        // 再写一次（模型列表变化时会重写）也不能出错。
        enable_binding(ctx, &agent, binding())
            .unwrap_or_else(|error| panic!("{} 重写失败：{error:#}", spec.id));

        disable_binding(ctx, &BackendSettings::default(), &agent)
            .unwrap_or_else(|error| panic!("{} 断开失败：{error:#}", spec.id));
        assert_eq!(
            snapshot(&fixture.home),
            before,
            "{} 断开后没有还原成接入前的样子",
            spec.id
        );
        assert!(!agents::connect::recorded(ctx, spec));
    }
}

#[test]
fn opencode_keeps_user_edits_made_while_connected() {
    let fixture = fixture();
    let ctx = &fixture.ctx;
    let spec = agents::spec("opencode").unwrap();
    let path = (spec.config_path)(&ctx.agent_env);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = "{\n  // 我的配置\n  \"theme\": \"dark\",\n  \"model\": \"anthropic/x\"\n}\n";
    std::fs::write(&path, original).unwrap();
    let agent = ToolId::Unknown("opencode".into());

    enable_binding(ctx, &agent, binding()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"codexplus/deepseek-v4-pro\""));
    assert!(text.contains("http://127.0.0.1:57321/gw/opencode/v1"));
    assert!(text.contains("\"theme\": \"dark\""));
    assert!(text.contains("// 我的配置"));

    // 接入期间用户自己加了一项。
    let mut value = agents::files::parse_jsonc(&text).unwrap();
    value["autoupdate"] = serde_json::json!(false);
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
    let restored =
        agents::files::parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(restored["model"], "anthropic/x");
    assert_eq!(restored["autoupdate"], false);
    assert!(restored.get("provider").is_none());
    assert!(restored.get("small_model").is_none());
}

#[test]
fn drift_reported_when_config_rewritten_elsewhere() {
    let fixture = fixture();
    let ctx = &fixture.ctx;
    let spec = agents::spec("opencode").unwrap();
    let agent = ToolId::Unknown("opencode".into());
    enable_binding(ctx, &agent, binding()).unwrap();
    std::fs::write((spec.config_path)(&ctx.agent_env), "{\"model\":\"other/x\"}").unwrap();
    assert!(codex_plus_core::gateway::drift::check(ctx, &agent).is_some());
    // 被接管时断开：不写 Agent 的文件，只丢掉存档。
    disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
    assert_eq!(
        std::fs::read_to_string((spec.config_path)(&ctx.agent_env)).unwrap(),
        "{\"model\":\"other/x\"}"
    );
    assert!(!agents::connect::recorded(ctx, spec));
}

#[test]
fn vscode_disconnect_keeps_user_model_groups_changed_while_connected() {
    use codex_plus_core::gateway::agents::files::{AgentStash, FileFormat, parse_jsonc};
    use serde_json::json;

    for id in ["vscode", "vscode-insiders", "vscodium"] {
        for existed in [true, false] {
            let fixture = fixture();
            let ctx = &fixture.ctx;
            let spec = agents::spec(id).unwrap();
            let path = (spec.config_path)(&ctx.agent_env);
            let agent = ToolId::Unknown(id.into());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let previous_ours =
                json!({"name": "codexplus", "vendor": "customoai", "models": ["old"]});
            if existed {
                std::fs::write(
                    &path,
                    json!([
                        {"name": "edit-me", "vendor": "customendpoint", "models": ["old"]},
                        previous_ours,
                        {"name": "delete-me", "vendor": "customendpoint", "models": []}
                    ])
                    .to_string(),
                )
                .unwrap();
            }
            enable_binding(ctx, &agent, binding()).unwrap();
            let mut groups = parse_jsonc(&std::fs::read_to_string(&path).unwrap())
                .unwrap()
                .as_array()
                .unwrap()
                .clone();
            groups.retain(|group| group["name"] != "delete-me");
            for group in &mut groups {
                if group["name"] == "edit-me" {
                    group["models"] = json!(["new"]);
                }
            }
            let added = json!({"name": "new-group", "vendor": "customendpoint", "models": ["new"]});
            let same_name = json!({"name": "codexplus", "vendor": "another-vendor", "models": []});
            groups.push(added.clone());
            groups.push(same_name.clone());
            std::fs::write(&path, serde_json::to_string(&groups).unwrap()).unwrap();

            // 重写模型列表不能把用户新增内容变成接入前快照，旧版 Raw 存档也要能安全断开。
            enable_binding(ctx, &agent, binding()).unwrap();
            let saved: AgentStash = codex_plus_core::gateway::restore::load_stash(
                &agents::connect::stash_path(ctx, id),
            )
            .unwrap();
            assert_eq!(saved.files[&path].format, FileFormat::Raw);
            disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
            let restored = parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let expected = if existed {
                json!([
                    {"name": "edit-me", "vendor": "customendpoint", "models": ["new"]},
                    previous_ours,
                    added,
                    same_name
                ])
            } else {
                json!([added, same_name])
            };
            assert_eq!(restored, expected, "{id}, existed={existed}");
        }
    }
}

#[test]
fn vscode_disconnect_restores_unchanged_models_file_verbatim() {
    let fixture = fixture();
    let ctx = &fixture.ctx;
    let spec = agents::spec("vscode").unwrap();
    let path = (spec.config_path)(&ctx.agent_env);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = "[\n  // 用户的模型\n  {\"name\":\"mine\",\"vendor\":\"customendpoint\",\"models\":[]},\n  {\"name\":\"codexplus\",\"vendor\":\"customoai\",\"models\":[\"old\"]},\n]\n";
    std::fs::write(&path, original).unwrap();
    let agent = ToolId::Unknown("vscode".into());
    enable_binding(ctx, &agent, binding()).unwrap();
    enable_binding(ctx, &agent, binding()).unwrap();
    disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn buddy_disconnect_keeps_user_models_changed_while_connected() {
    use codex_plus_core::gateway::agents::files::{AgentStash, FileFormat, parse_jsonc};
    use serde_json::json;

    for id in ["workbuddy", "workbuddy-ai", "codebuddy"] {
        for bare_array in [true, false] {
            let fixture = fixture();
            let ctx = &fixture.ctx;
            let spec = agents::spec(id).unwrap();
            let path = (spec.config_path)(&ctx.agent_env);
            let agent = ToolId::Unknown(id.into());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let previous_ours = json!({"id": "old-gateway", "vendor": "codexplus"});
            let original_models = json!([
                {"id": "mine", "vendor": "other", "name": "old"},
                previous_ours,
                {"id": "delete-me", "vendor": "other"}
            ]);
            let original = if bare_array {
                original_models
            } else {
                json!({
                    "models": original_models,
                    "availableModels": ["mine", "delete-me"],
                    "theme": "old"
                })
            };
            std::fs::write(&path, original.to_string()).unwrap();
            enable_binding(ctx, &agent, binding()).unwrap();
            let mut value = parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let models = if bare_array { &mut value } else { &mut value["models"] }
                .as_array_mut()
                .unwrap();
            models.retain(|model| model["id"] != "delete-me");
            models.iter_mut().find(|model| model["id"] == "mine").unwrap()["name"] =
                json!("edited");
            let added = json!({"id": "new-model", "vendor": "other"});
            // 其它供应商同名的模型与其白名单都不归网关所有。
            let same_id = json!({"id": "deepseek-v4-flash", "vendor": "other"});
            models.push(added.clone());
            models.push(same_id.clone());
            if !bare_array {
                value["availableModels"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|id| id != "delete-me");
                value["availableModels"].as_array_mut().unwrap().push(json!("new-model"));
                value["theme"] = json!("edited");
            }
            std::fs::write(&path, value.to_string()).unwrap();

            enable_binding(ctx, &agent, binding()).unwrap();
            let saved: AgentStash = codex_plus_core::gateway::restore::load_stash(
                &agents::connect::stash_path(ctx, id),
            )
            .unwrap();
            assert_eq!(
                saved.files[&path].format,
                if bare_array { FileFormat::Raw } else { FileFormat::Json }
            );
            disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
            let restored = parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let expected_models = json!([
                {"id": "mine", "vendor": "other", "name": "edited"},
                previous_ours,
                added,
                same_id
            ]);
            let expected = if bare_array {
                expected_models
            } else {
                json!({
                    "models": expected_models,
                    "availableModels": ["mine", "deepseek-v4-flash", "new-model"],
                    "theme": "edited"
                })
            };
            assert_eq!(restored, expected, "{id}, bare_array={bare_array}");
        }
    }
}

#[test]
fn buddy_disconnect_keeps_user_settings_in_new_models_file() {
    use codex_plus_core::gateway::agents::files::parse_jsonc;
    use serde_json::json;

    let fixture = fixture();
    let ctx = &fixture.ctx;
    let spec = agents::spec("workbuddy").unwrap();
    let path = (spec.config_path)(&ctx.agent_env);
    let agent = ToolId::Unknown("workbuddy".into());
    enable_binding(ctx, &agent, binding()).unwrap();
    let mut value = parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let added = json!({"id": "mine", "vendor": "other"});
    value["models"].as_array_mut().unwrap().push(added.clone());
    value["selectedModel"] = json!("mine");
    // 接入时没有改过的白名单，断开不能猜测其中哪些是用户写入的。
    value["availableModels"] = json!(["mine", "deepseek-v4-flash"]);
    std::fs::write(&path, value.to_string()).unwrap();
    disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
    let restored = parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        restored,
        json!({
            "models": [added],
            "selectedModel": "mine",
            "availableModels": ["mine", "deepseek-v4-flash"]
        })
    );
}

#[test]
fn buddy_disconnect_restores_unchanged_models_file_verbatim() {
    for original in [
        "[\n  // 用户的模型\n  {\"id\":\"mine\",\"vendor\":\"other\"},\n  {\"id\":\"old-gateway\",\"vendor\":\"codexplus\"},\n]",
        "{\n  // 用户的模型\n  \"models\":[{\"id\":\"mine\",\"vendor\":\"other\"}],\n  \"availableModels\":[\"mine\"],\n}",
        "{}",
    ] {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        let spec = agents::spec("workbuddy").unwrap();
        let path = (spec.config_path)(&ctx.agent_env);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, original).unwrap();
        let agent = ToolId::Unknown("workbuddy".into());
        enable_binding(ctx, &agent, binding()).unwrap();
        enable_binding(ctx, &agent, binding()).unwrap();
        disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}

#[test]
fn buddy_disconnect_keeps_all_other_models_deleted() {
    use codex_plus_core::gateway::agents::files::parse_jsonc;
    use serde_json::json;

    let fixture = fixture();
    let ctx = &fixture.ctx;
    let spec = agents::spec("workbuddy").unwrap();
    let path = (spec.config_path)(&ctx.agent_env);
    let agent = ToolId::Unknown("workbuddy".into());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"models":[{"id":"mine","vendor":"other"}]}"#).unwrap();
    enable_binding(ctx, &agent, binding()).unwrap();
    let mut value = parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["models"]
        .as_array_mut()
        .unwrap()
        .retain(|model| model["vendor"] == "codexplus");
    std::fs::write(&path, value.to_string()).unwrap();
    disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
    let restored = parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(restored, json!({"models": []}));
}

#[test]
fn dsh_disconnect_keeps_user_patch_changes() {
    let fixture = fixture();
    let ctx = &fixture.ctx;
    let spec = agents::spec("dsh").unwrap();
    let path = (spec.config_path)(&ctx.agent_env);
    let agent = ToolId::Unknown("dsh".into());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "- id: mine\n  config:\n    value: old\n").unwrap();
    enable_binding(ctx, &agent, binding()).unwrap();
    let current = std::fs::read_to_string(&path).unwrap().replace("value: old", "value: edited");
    let added = "- id: new-patch\n  config:\n    enabled: true\n";
    std::fs::write(&path, format!("{current}{added}")).unwrap();
    enable_binding(ctx, &agent, binding()).unwrap();
    disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("- id: mine\n  config:\n    value: edited\n{added}")
    );
}

#[test]
fn hermes_preserves_yaml_siblings_through_refresh_and_disconnect() {
    use serde_json::Value;
    for original in [
        "model: {default: mine, temperature: 0.3}\nproviders: {mine: {name: Mine}}\n",
        "model:\n  default: mine\n  temperature: 0.3\nproviders:\n  mine: {name: Mine}\n",
    ] {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        let spec = agents::spec("hermes").unwrap();
        let path = (spec.config_path)(&ctx.agent_env);
        let agent = ToolId::Unknown("hermes".into());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, original).unwrap();
        let read = || std::fs::read_to_string(&path).unwrap();
        enable_binding(ctx, &agent, binding()).unwrap();
        let connected: Value = serde_saphyr::from_str(&read()).unwrap();
        assert_eq!(connected["providers"]["mine"]["name"], "Mine");
        assert_eq!(connected["model"]["temperature"], 0.3);
        assert_eq!(connected["model"]["provider"], "codexplus");
        assert!(agents::connect::points_at_gateway(ctx, spec));
        disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
        assert_eq!(read(), original);

        enable_binding(ctx, &agent, binding()).unwrap();
        std::fs::write(
            &path,
            read()
                .replace("0.3", "0.8")
                .replace("Mine", "Edited")
                .replace("deepseek-v4-pro", "external-model"),
        )
        .unwrap();
        codex_plus_core::gateway::apply::refresh_enabled(ctx).unwrap();
        let refreshed: Value = serde_saphyr::from_str(&read()).unwrap();
        assert_eq!(refreshed["model"]["default"], "deepseek-v4-pro");
        disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
        let restored: Value = serde_saphyr::from_str(&read()).unwrap();
        assert_eq!(
            restored,
            serde_json::json!({"model":{"default":"mine","temperature":0.8},
            "providers":{"mine":{"name":"Edited"}}})
        );
    }
}

#[test]
fn hermes_refuses_unsafe_yaml_before_writing() {
    for original in [
        "model: scalar\n",
        "providers: []\n",
        "model: {default: a, default: b}\n",
        "model: &shared {default: mine}\ncopy: *shared\n",
        "model: {\n",
    ] {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        let path = (agents::spec("hermes").unwrap().config_path)(&ctx.agent_env);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, original).unwrap();
        assert!(enable_binding(ctx, &ToolId::Unknown("hermes".into()), binding()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
        assert!(!agents::connect::stash_path(ctx, "hermes").exists());
    }
}

#[test]
fn hermes_restores_legacy_top_level_yaml_stash_after_refresh() {
    let fixture = fixture();
    let ctx = &fixture.ctx;
    let spec = agents::spec("hermes").unwrap();
    let path = (spec.config_path)(&ctx.agent_env);
    let agent = ToolId::Unknown("hermes".into());
    let original = "model: {default: mine, temperature: 0.3}\nproviders: {mine: {name: Mine}}\n";
    enable_binding(ctx, &agent, binding()).unwrap();
    let stash = serde_json::json!({"files":{(path.to_str().unwrap()):{
        "format":"yaml", "before":original, "changed":[{"path":["model"]},{"path":["providers"]}]
    }}});
    std::fs::write(
        agents::connect::stash_path(ctx, "hermes"),
        stash.to_string(),
    )
    .unwrap();
    let connected = std::fs::read_to_string(&path)
        .unwrap()
        .replace("deepseek-v4-pro", "old_gateway");
    std::fs::write(&path, format!("{connected}\nnotes: keep\n")).unwrap();
    assert_eq!(
        codex_plus_core::gateway::apply::refresh_enabled(ctx).unwrap(),
        1
    );
    disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
    let restored: serde_json::Value =
        serde_saphyr::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        restored,
        serde_json::json!({"model":{"default":"mine","temperature":0.3},
        "providers":{"mine":{"name":"Mine"}},"notes":"keep"})
    );
}

#[test]
fn other_yaml_adapters_do_not_require_json_compatible_config() {
    let fixture = fixture();
    let ctx = &fixture.ctx;
    let path = (agents::spec("mister-morph").unwrap().config_path)(&ctx.agent_env);
    let agent = ToolId::Unknown("mister-morph".into());
    let original = "lookup:\n  ? [one, two]\n  : example\n";
    assert!(serde_saphyr::from_str::<serde_json::Value>(original).is_err());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, original).unwrap();
    enable_binding(ctx, &agent, binding()).unwrap();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with(original)
    );
    disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[test]
fn hermes_writes_flow_root_to_disk_and_restores_original_bytes() {
    for original in [
        "{model: {default: mine}, providers: {}}",
        "{\n  model: {default: mine},\n  providers: {},\n  user: [keep]\n}\n",
    ] {
        let fixture = fixture();
        let ctx = &fixture.ctx;
        let spec = agents::spec("hermes").unwrap();
        let path = (spec.config_path)(&ctx.agent_env);
        let agent = ToolId::Unknown("hermes".into());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, original).unwrap();
        enable_binding(ctx, &agent, binding()).unwrap();
        let saved: serde_json::Value =
            serde_saphyr::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let before: serde_json::Value = serde_saphyr::from_str(original).unwrap();
        assert_eq!(saved["user"], before["user"]);
        assert_eq!(saved["model"]["default"], "deepseek-v4-pro");
        assert_eq!(saved["model"]["provider"], "codexplus");
        assert!(
            saved["providers"]["codexplus"]["base_url"]
                .as_str()
                .unwrap()
                .ends_with("/gw/hermes/v1")
        );
        assert!(agents::connect::stash_path(ctx, "hermes").is_file());
        disable_binding(ctx, &BackendSettings::default(), &agent).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    }
}
