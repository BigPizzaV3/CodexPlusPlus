use super::*;
use crate::gateway::store::{
    AgentBinding, GatewayGroup, GatewayProvider, GroupMember, GroupMode, GroupSticky,
};
use crate::settings::{RelayMode, RelayProfile};
use serde_json::json;
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    sqlite: PathBuf,
    registry: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let sqlite = temp.path().join("sqlite");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&sqlite).unwrap();
        let registry = temp.path().join("registry.sqlite");
        Self {
            _temp: temp,
            home,
            sqlite,
            registry,
        }
    }

    fn apply(
        &self,
        gateway: &GatewaySettings,
        thread: &str,
        turn: &str,
        parent: Option<(&str, &str)>,
        model: &str,
    ) -> anyhow::Result<Value> {
        let metadata = json!({"thread_id":thread,"turn_id":turn,
            "parent_thread_id":parent.map(|parent|parent.0).unwrap_or(""),
            "parent_turn_id":parent.map(|parent|parent.1).unwrap_or("")});
        let mut headers = vec![("x-codex-turn-metadata".into(), metadata.to_string())];
        if let Some((parent, _)) = parent {
            headers.push(("x-codex-parent-thread-id".into(), parent.into()));
            headers.push(("x-openai-subagent".into(), "collab_spawn".into()));
        }
        let mut request = json!({"model":model,"input":[]});
        prepare_at(
            gateway,
            &ToolId::Codex,
            &mut request,
            ProxySessionHeaders {
                thread_id: Some(thread),
                client_identity: &headers,
                ..Default::default()
            },
            &self.registry,
            &self.home,
            &self.sqlite,
        )?;
        Ok(request)
    }
}

fn provider(id: &str) -> GatewayProvider {
    serde_json::from_value(json!({"enabled":true,"relay":RelayProfile {
        id:id.into(), name:id.into(), relay_mode:RelayMode::PureApi,
        base_url:format!("https://{id}.test/v1"), api_key:"fake-key".into(),
        model:"gpt-6.1-sol".into(), model_list:"gpt-6.1-sol\ngpt-6.1-terra".into(),
        ..Default::default()
    }}))
    .unwrap()
}

fn gateway() -> GatewaySettings {
    let mut gateway = GatewaySettings {
        auto_models: false,
        providers: vec![provider("inherit-a"), provider("inherit-b")],
        groups: vec![GatewayGroup {
            id: "inherit-group".into(),
            name: "Inheritance group".into(),
            mode: GroupMode::Manual,
            sticky: GroupSticky::Off,
            members: vec![
                GroupMember {
                    model: "inherit-a/gpt-6.1-sol".into(),
                    weight: 1,
                },
                GroupMember {
                    model: "inherit-b/gpt-6.1-terra".into(),
                    weight: 1,
                },
            ],
            pick: "inherit-b/gpt-6.1-terra".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    gateway.bindings.insert(
        "codex".into(),
        AgentBinding {
            enabled: true,
            model: "group/inherit-group".into(),
            ..Default::default()
        },
    );
    gateway
}

fn target(gateway: &GatewaySettings, model: &str) -> (String, String) {
    let plan = route::resolve_plan(gateway, &ToolId::Codex, model, None).unwrap();
    let candidate = plan.candidates.first().unwrap();
    (candidate.relay.id.clone(), candidate.upstream_model.clone())
}

#[test]
fn child_inherits_parent_provider_and_model_instead_of_bare_override() {
    let fixture = Fixture::new();
    let gateway = gateway();
    fixture
        .apply(
            &gateway,
            "parent",
            "parent-turn",
            None,
            "inherit-b/gpt-6.1-terra",
        )
        .unwrap();
    let child = fixture
        .apply(
            &gateway,
            "child",
            "child-turn",
            Some(("parent", "parent-turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(child["model"], "inherit-b/gpt-6.1-terra");
    assert_eq!(
        target(&gateway, child["model"].as_str().unwrap()),
        ("inherit-b".into(), "gpt-6.1-terra".into())
    );
    assert!(child.get("tools").is_none());
}

#[test]
fn aggregate_identity_preserves_group_and_tracks_manual_selection() {
    let fixture = Fixture::new();
    let mut gateway = gateway();
    fixture
        .apply(&gateway, "parent", "parent-turn", None, "")
        .unwrap();
    let child = fixture
        .apply(
            &gateway,
            "child",
            "child-turn",
            Some(("parent", "parent-turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(child["model"], "group/inherit-group");
    assert_eq!(target(&gateway, "group/inherit-group").0, "inherit-b");
    gateway.groups[0].pick = "inherit-a/gpt-6.1-sol".into();
    let retried = fixture
        .apply(
            &gateway,
            "child",
            "child-turn",
            Some(("parent", "parent-turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(retried["model"], "group/inherit-group");
    assert_eq!(target(&gateway, "group/inherit-group").0, "inherit-a");
}

#[test]
fn recorded_parent_turn_survives_reopen_and_default_or_parent_model_changes() {
    let fixture = Fixture::new();
    let mut gateway = gateway();
    fixture
        .apply(
            &gateway,
            "parent",
            "old-turn",
            None,
            "inherit-b/gpt-6.1-terra",
        )
        .unwrap();
    gateway.bindings.get_mut("codex").unwrap().model = "inherit-a/gpt-6.1-sol".into();
    fixture
        .apply(
            &gateway,
            "parent",
            "new-turn",
            None,
            "inherit-a/gpt-6.1-sol",
        )
        .unwrap();
    let child = fixture
        .apply(
            &gateway,
            "child",
            "child-turn",
            Some(("parent", "old-turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(child["model"], "inherit-b/gpt-6.1-terra");
}

#[test]
fn different_parents_with_the_same_turn_id_do_not_share_identity() {
    let fixture = Fixture::new();
    let gateway = gateway();
    for (parent, model) in [
        ("parent-a", "inherit-a/gpt-6.1-sol"),
        ("parent-b", "inherit-b/gpt-6.1-terra"),
    ] {
        fixture
            .apply(&gateway, parent, "same-turn", None, model)
            .unwrap();
        let child = fixture
            .apply(
                &gateway,
                &format!("child-{parent}"),
                "child-turn",
                Some((parent, "same-turn")),
                "gpt-6.1-sol",
            )
            .unwrap();
        assert_eq!(child["model"], model);
    }
}

#[test]
fn nested_subagents_inherit_the_direct_parent_logical_group() {
    let fixture = Fixture::new();
    let gateway = gateway();
    fixture
        .apply(&gateway, "root", "root-turn", None, "group/inherit-group")
        .unwrap();
    fixture
        .apply(
            &gateway,
            "child",
            "child-turn",
            Some(("root", "root-turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    let nested = fixture
        .apply(
            &gateway,
            "nested",
            "nested-turn",
            Some(("child", "child-turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(nested["model"], "group/inherit-group");
}

#[test]
fn missing_parent_record_fails_but_missing_parent_turn_uses_legacy_routing() {
    let fixture = Fixture::new();
    let gateway = gateway();
    assert!(
        fixture
            .apply(
                &gateway,
                "child",
                "turn",
                Some(("missing", "parent-turn")),
                "gpt-6.1-sol"
            )
            .is_err()
    );
    let legacy = fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(legacy["model"], "gpt-6.1-sol");
}

#[test]
fn ordinary_turn_updates_its_record_without_rewriting_later_requests() {
    let fixture = Fixture::new();
    let gateway = gateway();
    let first = fixture
        .apply(&gateway, "parent", "turn", None, "inherit-a/gpt-6.1-sol")
        .unwrap();
    let second = fixture
        .apply(&gateway, "parent", "turn", None, "inherit-b/gpt-6.1-terra")
        .unwrap();
    assert_eq!(first["model"], "inherit-a/gpt-6.1-sol");
    assert_eq!(second["model"], "inherit-b/gpt-6.1-terra");
    let child = fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(child["model"], "inherit-b/gpt-6.1-terra");
}

#[test]
fn ordinary_turn_passes_through_when_identity_or_registry_is_unavailable() {
    let fixture = Fixture::new();
    let gateway = gateway();
    let malformed = vec![("x-codex-turn-metadata".into(), "invalid".into())];
    let original = json!({"model":"inherit-a/gpt-6.1-sol","input":[]});
    let mut request = original.clone();
    prepare_at(
        &gateway,
        &ToolId::Codex,
        &mut request,
        ProxySessionHeaders {
            client_identity: &malformed,
            ..Default::default()
        },
        &fixture.registry,
        &fixture.home,
        &fixture.sqlite,
    )
    .unwrap();
    assert_eq!(request, original);
    let metadata = vec![(
        "x-codex-turn-metadata".into(),
        json!({"thread_id":"parent","turn_id":"turn"}).to_string(),
    )];
    for (path, home, sqlite) in [
        (
            fixture.registry.as_path(),
            fixture.home.join("missing"),
            fixture.sqlite.clone(),
        ),
        (
            fixture.home.as_path(),
            fixture.home.clone(),
            fixture.sqlite.clone(),
        ),
    ] {
        let mut request = original.clone();
        prepare_at(
            &gateway,
            &ToolId::Codex,
            &mut request,
            ProxySessionHeaders {
                client_identity: &metadata,
                ..Default::default()
            },
            path,
            &home,
            &sqlite,
        )
        .unwrap();
        assert_eq!(request, original);
    }
}

#[test]
fn compatibility_fallback_reports_the_reason_without_using_global_log_paths() {
    let fixture = Fixture::new();
    let gateway = gateway();
    let reasons = std::cell::RefCell::new(Vec::new());
    let report = |reason: &str| reasons.borrow_mut().push(reason.to_string());
    let malformed = vec![("x-codex-turn-metadata".into(), "invalid".into())];
    decide_at(
        &gateway,
        &ToolId::Codex,
        "gpt-6.1-sol",
        ProxySessionHeaders {
            client_identity: &malformed,
            ..Default::default()
        },
        &fixture.registry,
        &fixture.home,
        &fixture.sqlite,
        &report,
    )
    .unwrap();
    let legacy = vec![(
        "x-codex-turn-metadata".into(),
        json!({"thread_id":"child","turn_id":"turn","parent_thread_id":"parent"}).to_string(),
    )];
    decide_at(
        &gateway,
        &ToolId::Codex,
        "gpt-6.1-sol",
        ProxySessionHeaders {
            client_identity: &legacy,
            ..Default::default()
        },
        &fixture.registry,
        &fixture.home,
        &fixture.sqlite,
        &report,
    )
    .unwrap();
    let ordinary = vec![(
        "x-codex-turn-metadata".into(),
        json!({"thread_id":"parent","turn_id":"turn"}).to_string(),
    )];
    decide_at(
        &gateway,
        &ToolId::Codex,
        "gpt-6.1-sol",
        ProxySessionHeaders {
            client_identity: &ordinary,
            ..Default::default()
        },
        &fixture.registry,
        &fixture.home.join("missing"),
        &fixture.sqlite,
        &report,
    )
    .unwrap();
    assert_eq!(
        reasons.into_inner(),
        [
            "invalid_metadata",
            "missing_child_identity",
            "registry_unavailable"
        ]
    );
}

#[test]
fn explicit_child_model_is_preserved_and_becomes_the_nested_parent_identity() {
    let fixture = Fixture::new();
    let gateway = gateway();
    fixture
        .apply(&gateway, "parent", "turn", None, "group/inherit-group")
        .unwrap();
    let child = fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "inherit-a/gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(child["model"], "inherit-a/gpt-6.1-sol");
    let nested = fixture
        .apply(
            &gateway,
            "nested",
            "turn",
            Some(("child", "turn")),
            "gpt-6.1-terra",
        )
        .unwrap();
    assert_eq!(nested["model"], "inherit-a/gpt-6.1-sol");
    let explicit_group = fixture
        .apply(
            &gateway,
            "other-child",
            "turn",
            Some(("parent", "turn")),
            "group/inherit-group",
        )
        .unwrap();
    assert_eq!(explicit_group["model"], "group/inherit-group");
}

#[test]
fn declared_model_with_slash_is_bare_not_an_explicit_provider() {
    let fixture = Fixture::new();
    let mut gateway = gateway();
    gateway.providers[0]
        .relay
        .model_list
        .push_str("\nopenai/gpt-5\ngroup/not-a-group");
    fixture
        .apply(&gateway, "parent", "turn", None, "inherit-b/gpt-6.1-terra")
        .unwrap();
    for (child, model) in [
        ("child-a", "openai/gpt-5"),
        ("child-b", "group/not-a-group"),
    ] {
        let request = fixture
            .apply(&gateway, child, "turn", Some(("parent", "turn")), model)
            .unwrap();
        assert_eq!(request["model"], "inherit-b/gpt-6.1-terra");
    }
}

#[test]
fn changed_slash_interpretation_and_empty_group_cannot_switch_suppliers() {
    {
        let fixture = Fixture::new();
        let mut gateway = gateway();
        fixture
            .apply(&gateway, "parent", "turn", None, "inherit-b/gpt-6.1-terra")
            .unwrap();
        gateway.providers[0]
            .relay
            .model_list
            .push_str("\ninherit-b/gpt-6.1-terra");
        assert!(
            fixture
                .apply(
                    &gateway,
                    "child",
                    "turn",
                    Some(("parent", "turn")),
                    "gpt-6.1-sol"
                )
                .is_err()
        );
    }

    let fixture = Fixture::new();
    let mut gateway = gateway();
    fixture
        .apply(&gateway, "parent", "turn", None, "group/inherit-group")
        .unwrap();
    gateway.groups[0].members.clear();
    gateway.bindings.get_mut("codex").unwrap().model = "inherit-a/gpt-6.1-sol".into();
    assert!(
        fixture
            .apply(
                &gateway,
                "child",
                "turn",
                Some(("parent", "turn")),
                "gpt-6.1-sol"
            )
            .is_err()
    );
}

#[test]
fn ordinary_api_and_other_agents_do_not_create_identity_records() {
    let fixture = Fixture::new();
    let gateway = gateway();
    let original = json!({"model":"gpt-6.1-sol","input":[]});
    let mut request = original.clone();
    prepare_at(
        &gateway,
        &ToolId::Codex,
        &mut request,
        ProxySessionHeaders::default(),
        &fixture.registry,
        &fixture.home,
        &fixture.sqlite,
    )
    .unwrap();
    assert_eq!(request, original);
    let malformed = vec![("x-codex-turn-metadata".into(), "invalid".into())];
    prepare_at(
        &gateway,
        &ToolId::Grok,
        &mut request,
        ProxySessionHeaders {
            client_identity: &malformed,
            ..Default::default()
        },
        &fixture.registry,
        &fixture.home,
        &fixture.sqlite,
    )
    .unwrap();
    assert_eq!(request, original);
    assert!(!fixture.registry.exists());
}

#[test]
fn codex_home_and_sqlite_home_are_both_part_of_identity_scope() {
    let first = Fixture::new();
    let second = Fixture::new();
    let gateway = gateway();
    first
        .apply(
            &gateway,
            "parent",
            "parent-turn",
            None,
            "inherit-b/gpt-6.1-terra",
        )
        .unwrap();
    let metadata = json!({"thread_id":"child","turn_id":"turn","parent_thread_id":"parent","parent_turn_id":"parent-turn"}).to_string();
    let headers = vec![("x-codex-turn-metadata".into(), metadata)];
    for (home, sqlite) in [(&second.home, &first.sqlite), (&first.home, &second.sqlite)] {
        let mut request = json!({"model":"gpt-6.1-sol"});
        assert!(
            prepare_at(
                &gateway,
                &ToolId::Codex,
                &mut request,
                ProxySessionHeaders {
                    thread_id: Some("child"),
                    client_identity: &headers,
                    ..Default::default()
                },
                &first.registry,
                home,
                sqlite
            )
            .is_err()
        );
    }
}

#[test]
fn parent_rollout_fallback_uses_the_requested_turn_not_the_latest_model() {
    let fixture = Fixture::new();
    let gateway = gateway();
    let sessions = fixture.home.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let rollout = sessions.join("parent.jsonl");
    let rows = [
        json!({"type":"turn_context","payload":{"turn_id":"wanted","model":"group/inherit-group"}}),
        json!({"type":"turn_context","payload":{"turn_id":"newer","model":"inherit-a/gpt-6.1-sol"}}),
    ];
    std::fs::write(
        &rollout,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let database = Connection::open(fixture.sqlite.join("state_5.sqlite")).unwrap();
    database
        .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT, source TEXT);")
        .unwrap();
    database
        .execute(
            "INSERT INTO threads VALUES (?1,?2,'cli')",
            params!["parent", rollout.to_str().unwrap()],
        )
        .unwrap();
    drop(database);
    let child = fixture
        .apply(
            &gateway,
            "child",
            "child-turn",
            Some(("parent", "wanted")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(child["model"], "group/inherit-group");
}

#[test]
fn parent_rollout_fallback_skips_a_database_without_threads() {
    let fixture = Fixture::new();
    let gateway = gateway();
    let sqlite_dir = fixture.sqlite.join("sqlite");
    std::fs::create_dir_all(&sqlite_dir).unwrap();
    let wrong = Connection::open(sqlite_dir.join("codex-dev.db")).unwrap();
    wrong
        .execute_batch("CREATE TABLE inbox_items(id TEXT);")
        .unwrap();
    drop(wrong);
    std::fs::write(sqlite_dir.join("damaged.db"), b"not sqlite").unwrap();
    let rollout = fixture.home.join("parent.jsonl");
    std::fs::write(&rollout, json!({"type":"turn_context","payload":{"turn_id":"wanted","model":"inherit-b/gpt-6.1-terra"}}).to_string()).unwrap();
    let database = Connection::open(sqlite_dir.join("valid.db")).unwrap();
    database
        .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT, source TEXT);")
        .unwrap();
    database
        .execute(
            "INSERT INTO threads VALUES (?1,?2,'cli')",
            params!["parent", rollout.to_str().unwrap()],
        )
        .unwrap();
    drop(database);
    assert!(!fixture.sqlite.join("state_5.sqlite").exists());
    let child = fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "wanted")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(child["model"], "inherit-b/gpt-6.1-terra");
}

#[test]
fn legacy_registry_migrates_and_cleanup_preserves_active_ancestors() {
    let fixture = Fixture::new();
    let gateway = gateway();
    let home = std::fs::canonicalize(&fixture.home).unwrap();
    let sqlite = std::fs::canonicalize(&fixture.sqlite).unwrap();
    let scope = serde_json::to_string(&(&home, &sqlite)).unwrap();
    let database = Connection::open(&fixture.registry).unwrap();
    database
        .execute_batch(
            "CREATE TABLE gateway_subagent_models (
        scope TEXT NOT NULL, thread TEXT NOT NULL, turn TEXT NOT NULL,
        parent TEXT NOT NULL, parent_turn TEXT NOT NULL, model TEXT NOT NULL,
        PRIMARY KEY(scope, thread, turn)) WITHOUT ROWID;",
        )
        .unwrap();
    database.execute("INSERT INTO gateway_subagent_models VALUES (?1,'parent','turn','','','inherit-b/gpt-6.1-terra')", [&scope]).unwrap();
    drop(database);
    let child = fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(child["model"], "inherit-b/gpt-6.1-terra");
    let database = Connection::open(&fixture.registry).unwrap();
    assert!(has_column(&database, "updated_at").unwrap());
    assert!(has_column(&database, "model_kind").unwrap());
    database
        .execute("UPDATE gateway_subagent_models SET updated_at=1", [])
        .unwrap();
    database.execute("INSERT INTO gateway_subagent_models(scope,thread,turn,parent,parent_turn,model,updated_at,model_kind) VALUES (?1,'obsolete','turn','','','unused',1,'bare')", [&scope]).unwrap();
    drop(database);
    fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    let database = Connection::open(&fixture.registry).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    cleanup_expired(&database, now).unwrap();
    assert!(
        recorded_model(&database, &scope, "parent", "turn")
            .unwrap()
            .is_some()
    );
    assert!(
        recorded_model(&database, &scope, "child", "turn")
            .unwrap()
            .is_some()
    );
    assert!(
        recorded_model(&database, &scope, "obsolete", "turn")
            .unwrap()
            .is_none()
    );
}

#[test]
fn locked_registry_does_not_block_ordinary_or_explicit_child_requests() {
    let fixture = Fixture::new();
    let gateway = gateway();
    fixture
        .apply(&gateway, "parent", "turn", None, "inherit-b/gpt-6.1-terra")
        .unwrap();
    let database = Connection::open(&fixture.registry).unwrap();
    database.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    let ordinary = fixture
        .apply(&gateway, "parent", "turn", None, "inherit-a/gpt-6.1-sol")
        .unwrap();
    assert_eq!(ordinary["model"], "inherit-a/gpt-6.1-sol");
    let explicit = fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "inherit-a/gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(explicit["model"], "inherit-a/gpt-6.1-sol");
    assert!(
        fixture
            .apply(
                &gateway,
                "child",
                "turn",
                Some(("parent", "turn")),
                "gpt-6.1-sol"
            )
            .is_err()
    );
    database.execute_batch("ROLLBACK;").unwrap();
    let parent_key = identity_key(
        &fixture.registry,
        &fixture.home,
        &fixture.sqlite,
        "parent",
        "turn",
    );
    UNRELIABLE
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(&parent_key);
    assert!(
        fixture
            .apply(
                &gateway,
                "child-after-unlock",
                "turn",
                Some(("parent", "turn")),
                "gpt-6.1-sol"
            )
            .is_err()
    );
    fixture
        .apply(&gateway, "parent", "turn", None, "inherit-a/gpt-6.1-sol")
        .unwrap();
    let recovered = fixture
        .apply(
            &gateway,
            "child-after-unlock",
            "turn",
            Some(("parent", "turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(recovered["model"], "inherit-a/gpt-6.1-sol");
}

#[test]
fn concurrent_recordings_stay_unreliable_until_all_complete() {
    let fixture = Fixture::new();
    let key = identity_key(
        &fixture.registry,
        &fixture.home,
        &fixture.sqlite,
        "parent",
        "turn",
    );
    let first = begin_record(&key);
    let second = begin_record(&key);
    finish_record(&key, second, true);
    assert!(record_is_unreliable(&key));
    finish_record(&key, first, true);
    assert!(!record_is_unreliable(&key));
    let first = begin_record(&key);
    let second = begin_record(&key);
    finish_record(&key, second, false);
    assert!(record_is_unreliable(&key));
    finish_record(&key, first, true);
    assert!(record_is_unreliable(&key));
    let retry = begin_record(&key);
    finish_record(&key, retry, true);
    assert!(!record_is_unreliable(&key));
}

#[test]
fn explicit_child_must_reconfirm_its_own_model_after_a_registry_fault() {
    let fixture = Fixture::new();
    let gateway = gateway();
    fixture
        .apply(&gateway, "parent", "turn", None, "inherit-a/gpt-6.1-sol")
        .unwrap();
    fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "inherit-b/gpt-6.1-terra",
        )
        .unwrap();
    mark_registry_unreliable(&fixture.registry).unwrap();
    fixture
        .apply(&gateway, "parent", "turn", None, "inherit-a/gpt-6.1-sol")
        .unwrap();
    assert!(
        fixture
            .apply(
                &gateway,
                "child",
                "turn",
                Some(("parent", "turn")),
                "gpt-6.1-sol"
            )
            .is_err()
    );
    fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "inherit-b/gpt-6.1-terra",
        )
        .unwrap();
    let resumed = fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(resumed["model"], "inherit-b/gpt-6.1-terra");
}

#[test]
fn existing_child_recovers_only_after_same_parent_identity_is_confirmed() {
    let fixture = Fixture::new();
    let gateway = gateway();
    fixture
        .apply(&gateway, "parent", "turn", None, "inherit-b/gpt-6.1-terra")
        .unwrap();
    fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    mark_registry_unreliable(&fixture.registry).unwrap();
    assert!(
        fixture
            .apply(
                &gateway,
                "child",
                "turn",
                Some(("parent", "turn")),
                "gpt-6.1-sol"
            )
            .is_err()
    );
    fixture
        .apply(&gateway, "parent", "turn", None, "inherit-b/gpt-6.1-terra")
        .unwrap();
    let recovered = fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(recovered["model"], "inherit-b/gpt-6.1-terra");
    mark_registry_unreliable(&fixture.registry).unwrap();
    fixture
        .apply(&gateway, "parent", "turn", None, "inherit-a/gpt-6.1-sol")
        .unwrap();
    assert!(
        fixture
            .apply(
                &gateway,
                "child",
                "turn",
                Some(("parent", "turn")),
                "gpt-6.1-sol"
            )
            .is_err()
    );
}

#[test]
fn identified_child_with_conflicting_metadata_does_not_fall_back() {
    let fixture = Fixture::new();
    let gateway = gateway();
    let headers = vec![
        ("x-openai-subagent".into(), "collab_spawn".into()),
        ("x-codex-parent-thread-id".into(), "parent-a".into()),
        ("x-codex-turn-metadata".into(), json!({"thread_id":"child","turn_id":"turn","parent_thread_id":"parent-b","parent_turn_id":"turn"}).to_string()),
    ];
    let mut request = json!({"model":"gpt-6.1-sol"});
    assert!(
        prepare_at(
            &gateway,
            &ToolId::Codex,
            &mut request,
            ProxySessionHeaders {
                client_identity: &headers,
                ..Default::default()
            },
            &fixture.registry,
            &fixture.home,
            &fixture.sqlite
        )
        .is_err()
    );
}

#[test]
fn a_recorded_child_turn_cannot_be_rebound_to_a_different_parent() {
    let fixture = Fixture::new();
    let gateway = gateway();
    fixture
        .apply(&gateway, "parent-a", "turn", None, "inherit-a/gpt-6.1-sol")
        .unwrap();
    fixture
        .apply(
            &gateway,
            "parent-b",
            "turn",
            None,
            "inherit-b/gpt-6.1-terra",
        )
        .unwrap();
    fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent-a", "turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert!(
        fixture
            .apply(
                &gateway,
                "child",
                "turn",
                Some(("parent-b", "turn")),
                "gpt-6.1-sol"
            )
            .is_err()
    );
}

#[test]
fn unavailable_parent_provider_does_not_fall_back_to_another_supplier() {
    let fixture = Fixture::new();
    let mut gateway = gateway();
    fixture
        .apply(&gateway, "parent", "turn", None, "inherit-b/gpt-6.1-terra")
        .unwrap();
    gateway.providers[1].enabled = false;
    assert!(
        fixture
            .apply(
                &gateway,
                "child",
                "turn",
                Some(("parent", "turn")),
                "gpt-6.1-sol"
            )
            .is_err()
    );
}

#[test]
fn spawn_tool_guidance_preserves_complete_identity_without_duplicate_policy() {
    let mut tools = json!([
        {"type":"namespace","name":"multi_agent_v1","tools":[
            {"type":"function","name":"spawn_agent","description":"Spawn a sub-agent."}]},
        {"type":"namespace","name":"unrelated","tools":[
            {"type":"function","name":"spawn_agent","description":"Unrelated tool."}]}
    ]);
    patch_spawn_tools(&mut tools, "group/inherit-group", false);
    patch_spawn_tools(&mut tools, "group/inherit-group", false);
    let description = tools[0]["tools"][0]["description"].as_str().unwrap();
    assert_eq!(description.matches(POLICY_MARKER).count(), 1);
    assert!(description.contains("group/inherit-group"));
    assert_eq!(tools[1]["tools"][0]["description"], "Unrelated tool.");
}

#[test]
fn metadata_thread_id_precedes_an_opaque_session_id() {
    let fixture = Fixture::new();
    let gateway = gateway();
    let metadata = json!({"thread_id":"parent","turn_id":"turn"}).to_string();
    let headers = vec![("x-codex-turn-metadata".into(), metadata)];
    let mut request = json!({"model":"inherit-b/gpt-6.1-terra"});
    prepare_at(
        &gateway,
        &ToolId::Codex,
        &mut request,
        ProxySessionHeaders {
            session_id: Some("opaque-cache-key"),
            client_identity: &headers,
            ..Default::default()
        },
        &fixture.registry,
        &fixture.home,
        &fixture.sqlite,
    )
    .unwrap();
    let child = fixture
        .apply(
            &gateway,
            "child",
            "turn",
            Some(("parent", "turn")),
            "gpt-6.1-sol",
        )
        .unwrap();
    assert_eq!(child["model"], "inherit-b/gpt-6.1-terra");
}
