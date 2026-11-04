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
            strip_responses_image_detail: None,
            disable_tool_search: None,
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
