use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use codex_plus_core::gateway::activity::{ActivitySnapshot, AgentActivity, RouteRecord, RouteStep};
use codex_plus_core::gateway::agents::{self, AgentEnv};
use codex_plus_core::gateway::apply::{
    ApplyContext, CODEX_GATEWAY_RELAY_ID, ProviderInput, codex_gateway_profile, create_provider,
    enable_binding,
};
use codex_plus_core::gateway::diagnostics::{AgentDiagnostics, inspect, safe_endpoint};
use codex_plus_core::gateway::store::{AgentBinding, GatewaySettings, GatewayStore};
use codex_plus_core::settings::{BackendSettings, RelayProtocol, SettingsStore};
use codex_plus_core::tools::ToolId;
use serde_json::{Value, json};

struct Fixture {
    _temp: tempfile::TempDir,
    ctx: ApplyContext,
    home: PathBuf,
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
        settings: SettingsStore::new(state.join("settings.json")),
        backup_root: state.join("backups"),
        port: 57321,
        agent_env: AgentEnv::at(&home),
    };
    create_provider(
        &ctx.gateway,
        &ProviderInput {
            name: "Relay".into(),
            base_url: "https://relay.test/v1".into(),
            protocol: RelayProtocol::Responses,
            keys: vec!["sk-upstream-secret".into()],
            model: "test-model".into(),
            model_list: "test-model".into(),
            price: None,
            model_prices: None,
            proxy: None,
            strip_image_generation_tool: None,
            strip_responses_image_detail: None,
            disable_tool_search: None,
        },
        "",
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
        enabled: true,
        model: "test-model".into(),
        token: "cpx-gw-local-secret".into(),
        ..AgentBinding::default()
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn set_age(path: &Path, age_ms: u64) {
    // 改修改时间要写权限：Windows 上用只读打开的句柄 set_modified 会报「拒绝访问」。
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_millis(age_ms))
        .unwrap();
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn activity(agent: &ToolId, time: u64, success: bool) -> ActivitySnapshot {
    ActivitySnapshot {
        agents: vec![AgentActivity {
            agent: agent.as_str().to_string(),
            requests: 3,
            errors: u64::from(!success),
            last: Some(RouteRecord {
                requested_model: "test-model".into(),
                model: "test-model".into(),
                group_id: None,
                planned: Vec::new(),
                steps: vec![RouteStep {
                    relay_id: "relay-private".into(),
                    relay_name: "relay-private".into(),
                    key_id: "sk-key-private".into(),
                    model: "test-model".into(),
                    outcome: if success { "answered" } else { "failed" }.into(),
                    failure: (!success).then(|| "auth_invalid".into()),
                    status: Some(if success { 200 } else { 401 }),
                    elapsed_ms: 1,
                }],
                result: if success { "answered" } else { "failed" }.into(),
                cooling_ms: None,
                finished_at_ms: time,
                elapsed_ms: 1,
            }),
            ..AgentActivity::default()
        }],
    }
}

fn claude_fixture(fixture: &Fixture) -> (ToolId, GatewaySettings, PathBuf) {
    let agent = ToolId::Claude;
    let mut gateway = fixture.ctx.gateway.load().unwrap();
    gateway.bindings.insert("claude".into(), binding());
    let path = fixture.ctx.claude.claude_home.join("settings.json");
    write(
        &path,
        r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:57321/gw/claude","ANTHROPIC_AUTH_TOKEN":"cpx-gw-local-secret"}}"#,
    );
    (agent, gateway, path)
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            stack.extend(
                std::fs::read_dir(path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        } else {
            files.push((path.clone(), std::fs::read(path).unwrap()));
        }
    }
    files.sort();
    files
}

fn has_code(result: &AgentDiagnostics, code: &str) -> bool {
    result.config_evidence.iter().any(|item| item.code == code)
}

#[test]
fn every_generic_agent_is_verified_read_only_after_connecting() {
    for spec in agents::all() {
        let fixture = fixture();
        let agent = ToolId::Unknown(spec.id.to_string());
        enable_binding(&fixture.ctx, &agent, binding()).unwrap();
        let gateway = fixture.ctx.gateway.load().unwrap();
        let before = snapshot(&fixture.home);
        let result = inspect(
            &fixture.ctx,
            &agent,
            &gateway,
            true,
            &ActivitySnapshot::default(),
        );
        assert_eq!(
            result.state, "awaitingTraffic",
            "{}: {:?}",
            spec.id, result.config_evidence
        );
        assert_eq!(snapshot(&fixture.home), before, "{} 改了配置", spec.id);
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains("cpx-gw-local-secret"));
        assert!(!serialized.contains("sk-upstream-secret"));
    }
}

#[test]
fn missing_invalid_and_disabled_are_distinct() {
    let fixture = fixture();
    let agent = ToolId::Claude;
    let mut gateway = fixture.ctx.gateway.load().unwrap();
    gateway.bindings.insert("claude".into(), binding());
    let empty = ActivitySnapshot::default();
    assert_eq!(
        inspect(&fixture.ctx, &agent, &gateway, true, &empty).state,
        "configMissing"
    );
    write(
        &fixture.ctx.claude.claude_home.join("settings.json"),
        r#"{"token":"do-not-echo", invalid}"#,
    );
    let invalid = inspect(&fixture.ctx, &agent, &gateway, true, &empty);
    assert_eq!(invalid.state, "configUnreadable");
    assert!(
        !serde_json::to_string(&invalid)
            .unwrap()
            .contains("do-not-echo")
    );
    gateway.bindings.get_mut("claude").unwrap().enabled = false;
    assert_eq!(
        inspect(&fixture.ctx, &agent, &gateway, false, &empty).state,
        "disabled"
    );
}

#[test]
fn unreadable_and_unsupported_configs_are_not_reported_as_changed() {
    let fixture = fixture();
    let mut gateway = fixture.ctx.gateway.load().unwrap();
    gateway.bindings.insert("claude".into(), binding());
    std::fs::create_dir_all(fixture.ctx.claude.claude_home.join("settings.json")).unwrap();
    let empty = ActivitySnapshot::default();
    assert_eq!(
        inspect(&fixture.ctx, &ToolId::Claude, &gateway, true, &empty).state,
        "configUnreadable"
    );
    gateway.bindings.insert("unsupported".into(), binding());
    assert_eq!(
        inspect(
            &fixture.ctx,
            &ToolId::Unknown("unsupported".into()),
            &gateway,
            true,
            &empty
        )
        .state,
        "configUnverified"
    );
}

#[test]
fn no_traffic_is_unverified_not_bypassed_and_helper_stopped_is_explicit() {
    let fixture = fixture();
    let (agent, gateway, _) = claude_fixture(&fixture);
    let empty = ActivitySnapshot::default();
    let waiting = inspect(&fixture.ctx, &agent, &gateway, true, &empty);
    assert_eq!(waiting.state, "awaitingTraffic");
    assert_eq!(waiting.last_seen_ms, None);
    assert_eq!(
        inspect(&fixture.ctx, &agent, &gateway, false, &empty).state,
        "gatewayStopped"
    );
}

#[test]
fn recent_activity_cannot_validate_a_later_configuration_repair() {
    let fixture = fixture();
    let (agent, gateway, path) = claude_fixture(&fixture);
    let completed = now_ms().saturating_sub(10_000);
    let last = activity(&agent, completed, true);
    let result = inspect(&fixture.ctx, &agent, &gateway, true, &last);
    assert_eq!(result.state, "awaitingTraffic");
    assert_eq!(result.last_seen_ms, Some(completed));
    set_age(&path, 20_000);
    assert_eq!(
        inspect(&fixture.ctx, &agent, &gateway, true, &last).state,
        "active"
    );
    set_age(&path, 600_000);
    let old = activity(&agent, now_ms().saturating_sub(360_000), true);
    assert_eq!(
        inspect(&fixture.ctx, &agent, &gateway, true, &old).state,
        "awaitingTraffic"
    );
}

#[test]
fn in_flight_without_a_completed_request_is_active() {
    let fixture = fixture();
    let (agent, gateway, _) = claude_fixture(&fixture);
    let current = ActivitySnapshot {
        agents: vec![AgentActivity {
            agent: "claude".into(),
            in_flight: vec!["private-provider-id".into()],
            ..AgentActivity::default()
        }],
    };
    let result = inspect(&fixture.ctx, &agent, &gateway, true, &current);
    assert_eq!(result.state, "active");
    assert_eq!(result.in_flight, 1);
    assert_eq!(result.last_seen_ms, None);
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("private-provider-id")
    );
}

#[test]
fn upstream_error_uses_only_safe_status_and_fixed_classification() {
    let fixture = fixture();
    let (agent, gateway, path) = claude_fixture(&fixture);
    set_age(&path, 20_000);
    let last = activity(&agent, now_ms().saturating_sub(1_000), false);
    let result = inspect(&fixture.ctx, &agent, &gateway, true, &last);
    assert_eq!(result.state, "upstreamError");
    let error = result.recent_error.as_ref().unwrap();
    assert_eq!(error.kind, "auth_invalid");
    assert_eq!(error.status, Some(401));
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("sk-key-private"));
    assert!(!encoded.contains("relay-private"));
    let mut server_error = last.clone();
    let step = &mut server_error.agents[0].last.as_mut().unwrap().steps[0];
    step.failure = Some("private-upstream-body".into());
    step.status = Some(503);
    let result = inspect(&fixture.ctx, &agent, &gateway, true, &server_error);
    assert_eq!(result.recent_error.as_ref().unwrap().kind, "server_error");
    assert_eq!(result.recent_error.as_ref().unwrap().status, Some(503));
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("private-upstream-body")
    );
}

#[test]
fn public_endpoint_display_rejects_unsafe_schemes_and_removes_credentials() {
    assert_eq!(
        safe_endpoint("http://127.0.0.1:57321/gw/claude/v1/"),
        Some("http://127.0.0.1:57321/gw/claude/v1".into())
    );
    assert_eq!(safe_endpoint("not a URL with private-key"), None);
    assert_eq!(safe_endpoint("file:///private/secret"), None);
    assert_eq!(safe_endpoint("javascript:private-secret"), None);
    let display = safe_endpoint(
        "https://private-user:private-password@relay.test/private-key/v1?token=private-query#private-fragment",
    )
    .unwrap();
    assert!(display.starts_with("https://relay.test/"));
    for secret in [
        "private-user",
        "private-password",
        "private-key",
        "private-query",
        "private-fragment",
    ] {
        assert!(!display.contains(secret), "地址展示泄露了 {secret}");
    }
}

#[test]
fn current_port_host_and_agent_path_are_compared_exactly() {
    let fixture = fixture();
    let (agent, gateway, path) = claude_fixture(&fixture);
    for (endpoint, code) in [
        ("http://127.0.0.1:57322/gw/claude", "endpoint_port_mismatch"),
        (
            "https://external.test:57321/gw/claude",
            "endpoint_host_mismatch",
        ),
        (
            "http://127.0.0.1:57321/gw/claude-other",
            "endpoint_path_mismatch",
        ),
    ] {
        write(
            &path,
            &json!({"env": {
                "ANTHROPIC_BASE_URL": endpoint,
                "ANTHROPIC_AUTH_TOKEN": "cpx-gw-local-secret"
            }})
            .to_string(),
        );
        let result = inspect(
            &fixture.ctx,
            &agent,
            &gateway,
            true,
            &ActivitySnapshot::default(),
        );
        assert_eq!(result.state, "configChanged");
        assert!(has_code(&result, code), "{:?}", result.config_evidence);
    }
}

#[test]
fn jsonc_comments_do_not_verify_an_endpoint_and_urls_are_redacted() {
    let fixture = fixture();
    let agent = ToolId::Unknown("opencode".into());
    enable_binding(&fixture.ctx, &agent, binding()).unwrap();
    let spec = agents::spec("opencode").unwrap();
    let path = (spec.config_path)(&fixture.ctx.agent_env);
    let mut value: Value =
        agents::files::parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["provider"]["codexplus"]["options"]["baseURL"] = json!(
        "https://user-secret:password-secret@relay.test/private-token/v1?api_key=query-secret#fragment-secret"
    );
    write(
        &path,
        &format!(
            "// http://127.0.0.1:57321/gw/opencode/v1\n{}",
            serde_json::to_string_pretty(&value).unwrap()
        ),
    );
    let result = inspect(
        &fixture.ctx,
        &agent,
        &fixture.ctx.gateway.load().unwrap(),
        true,
        &ActivitySnapshot::default(),
    );
    assert_eq!(result.state, "configChanged");
    let encoded = serde_json::to_string(&result).unwrap();
    for secret in [
        "user-secret",
        "password-secret",
        "private-token",
        "query-secret",
        "fragment-secret",
        "cpx-gw-local-secret",
    ] {
        assert!(!encoded.contains(secret), "诊断泄露了 {secret}");
    }
}

#[test]
fn jsonc_with_only_a_gateway_comment_is_not_connected() {
    let fixture = fixture();
    let (agent, gateway, path) = claude_fixture(&fixture);
    write(
        &path,
        "// http://127.0.0.1:57321/gw/claude\n/* comment only */",
    );
    let result = inspect(
        &fixture.ctx,
        &agent,
        &gateway,
        true,
        &ActivitySnapshot::default(),
    );
    assert_eq!(result.state, "configChanged");
    assert!(has_code(&result, "endpoint_missing"));
}

#[test]
fn selected_provider_is_checked_even_when_old_gateway_entry_remains() {
    let fixture = fixture();
    let agent = ToolId::Unknown("opencode".into());
    enable_binding(&fixture.ctx, &agent, binding()).unwrap();
    let spec = agents::spec("opencode").unwrap();
    let path = (spec.config_path)(&fixture.ctx.agent_env);
    let mut value: Value =
        agents::files::parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for selected in [
        "another-provider/another-model",
        "codexplus-another-provider/another-model",
    ] {
        value["model"] = json!(selected);
        write(&path, &value.to_string());
        let result = inspect(
            &fixture.ctx,
            &agent,
            &fixture.ctx.gateway.load().unwrap(),
            true,
            &ActivitySnapshot::default(),
        );
        assert_eq!(result.state, "configChanged");
        assert!(has_code(&result, "selection_changed"));
    }
}

#[test]
fn oauth_mode_does_not_validate_the_inactive_api_configuration() {
    for id in ["gemini-cli", "qwen-code"] {
        let fixture = fixture();
        let agent = ToolId::Unknown(id.into());
        enable_binding(&fixture.ctx, &agent, binding()).unwrap();
        let spec = agents::spec(id).unwrap();
        let path = (spec.config_path)(&fixture.ctx.agent_env);
        let mut value: Value =
            agents::files::parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
        value["security"]["auth"]["selectedType"] = json!("oauth-personal");
        write(&path, &value.to_string());
        let result = inspect(
            &fixture.ctx,
            &agent,
            &fixture.ctx.gateway.load().unwrap(),
            true,
            &activity(&agent, now_ms().saturating_add(1), true),
        );
        assert_eq!(result.state, "configChanged", "{id}");
        assert!(has_code(&result, "selection_changed"), "{id}");
    }
}

#[test]
fn cline_selected_builtin_provider_must_still_be_the_gateway_slot() {
    let fixture = fixture();
    let agent = ToolId::Unknown("cline".into());
    enable_binding(&fixture.ctx, &agent, binding()).unwrap();
    let path = (agents::spec("cline").unwrap().config_path)(&fixture.ctx.agent_env);
    let mut value: Value =
        agents::files::parse_jsonc(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["lastUsedProvider"] = json!("anthropic");
    write(&path, &value.to_string());
    let result = inspect(
        &fixture.ctx,
        &agent,
        &fixture.ctx.gateway.load().unwrap(),
        true,
        &ActivitySnapshot::default(),
    );
    assert_eq!(result.state, "configChanged");
    assert!(has_code(&result, "selection_changed"));
}

#[test]
fn grok_inspects_only_the_selected_model_and_never_echoes_its_alias() {
    let fixture = fixture();
    let agent = ToolId::Grok;
    let mut gateway = fixture.ctx.gateway.load().unwrap();
    gateway.bindings.insert("grok".into(), binding());
    write(
        &fixture.ctx.grok_home.join("config.toml"),
        r#"[models]
default = "private-model-alias"
[model.old]
base_url = "http://127.0.0.1:57321/gw/grok/v1"
api_key = "cpx-gw-local-secret"
[model.private-model-alias]
base_url = "https://external.test/v1"
api_key = "private-key"
"#,
    );
    let result = inspect(
        &fixture.ctx,
        &agent,
        &gateway,
        true,
        &ActivitySnapshot::default(),
    );
    assert_eq!(result.state, "configChanged");
    assert!(has_code(&result, "endpoint_host_mismatch"));
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("private-model-alias"));
    assert!(!encoded.contains("private-key"));
}

#[test]
fn codex_official_identity_requires_the_gateway_as_its_next_hop() {
    let _isolated = super::common::IsolatedTestState::new();
    let fixture = fixture();
    let agent = ToolId::Codex;
    let mut gateway = fixture.ctx.gateway.load().unwrap();
    gateway.bindings.insert("codex".into(), binding());
    let path = fixture.ctx.codex_home.join("config.toml");
    write(
        &path,
        "model_provider = \"openai\"\nopenai_base_url = \"http://127.0.0.1:57321/v1\"\n",
    );
    let profile = codex_gateway_profile(&gateway, &binding(), fixture.ctx.port, true).unwrap();
    fixture
        .ctx
        .settings
        .save(&BackendSettings {
            active_relay_id: CODEX_GATEWAY_RELAY_ID.into(),
            relay_profiles: vec![profile],
            ..BackendSettings::default()
        })
        .unwrap();
    let empty = ActivitySnapshot::default();
    let connected = inspect(&fixture.ctx, &agent, &gateway, true, &empty);
    assert_eq!(
        connected.state, "awaitingTraffic",
        "{:?}",
        connected.config_evidence
    );
    let mut settings = fixture.ctx.settings.load().unwrap();
    settings.active_relay_id = "not-the-gateway".into();
    fixture.ctx.settings.save(&settings).unwrap();
    let changed = inspect(&fixture.ctx, &agent, &gateway, true, &empty);
    assert_eq!(changed.state, "configChanged");
    assert!(has_code(&changed, "selection_changed"));
    write(
        &fixture._temp.path().join("state/settings.json"),
        "{\"api_key\":\"private-config-key\", not-json}",
    );
    let unreadable_hop = inspect(&fixture.ctx, &agent, &gateway, true, &empty);
    assert_eq!(unreadable_hop.state, "configUnverified");
    assert!(
        !serde_json::to_string(&unreadable_hop)
            .unwrap()
            .contains("private-config-key")
    );
}
