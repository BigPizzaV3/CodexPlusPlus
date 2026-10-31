use std::path::PathBuf;

use codex_plus_core::agent_collaboration::{CollaborationConfig, Store, TaskRequest, mcp};
use serde_json::{Value, json};

fn store(temp: &tempfile::TempDir) -> Store {
    Store::new(temp.path().join("collaboration.sqlite3"))
}

fn enable(store: &Store, ids: &[&str]) {
    let mut config = CollaborationConfig::default();
    config.enabled = true;
    for agent in &mut config.agents {
        agent.enabled = ids.contains(&agent.id.as_str());
        // 不执行测试二进制；仅作为真实存在、具有执行权限的文件验证路径探测。
        agent.executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
    }
    store.save_config(config).unwrap();
}

fn request(temp: &tempfile::TempDir, agent: &str) -> TaskRequest {
    TaskRequest {
        agent_id: agent.to_string(),
        prompt: "请检查当前目录".to_string(),
        cwd: temp.path().to_string_lossy().into_owned(),
        continue_task_id: None,
    }
}

#[test]
fn default_is_opt_in_and_never_accepts_work_before_both_switches_are_enabled() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot["config"]["enabled"], false);
    assert!(
        snapshot["config"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|agent| agent["enabled"] == false)
    );
    assert_eq!(snapshot["capabilities"]["desktopSessionWake"], false);
    assert!(store.submit(request(&temp, "codex")).is_err());
    let mut config = store.config().unwrap();
    config.enabled = true;
    store.save_config(config).unwrap();
    assert!(store.submit(request(&temp, "codex")).is_err());
    assert!(
        store.snapshot().unwrap()["tasks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn legacy_configuration_gets_a_default_without_enabling_workers_or_changing_models() {
    let config: CollaborationConfig = serde_json::from_value(json!({"enabled":false,"agents":[
        {"id":"codex","name":"Codex","executable":"","enabled":false},
        {"id":"deepseek","name":"DeepSeek Harness","executable":"","enabled":false},
        {"id":"claude","name":"Claude Code","executable":"","enabled":false}
    ]}))
    .unwrap();
    assert_eq!(config.default_agent_id, "deepseek");
    assert!(
        config
            .agents
            .iter()
            .all(|agent| agent.model.is_empty() && !agent.enabled && !agent.reuse_saved_auth)
    );
    assert!(
        serde_json::to_value(&config).unwrap()["agents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|agent| agent.get("reuseSavedAuth").is_none())
    );
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    store.save_config(config.clone()).unwrap();
    let mut invalid = config.clone();
    invalid.default_agent_id = "unknown".to_string();
    assert!(store.save_config(invalid).is_err());
    let mut invalid = config.clone();
    invalid.agents[1].model = "missing-provider".to_string();
    assert!(store.save_config(invalid).is_err());
    assert_eq!(store.config().unwrap(), config);
}

#[test]
fn dsh_saved_auth_is_opt_in_and_round_trips_without_serializing_disabled_flags() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let mut config = CollaborationConfig::default();
    config.agents[1].reuse_saved_auth = true;
    store.save_config(config.clone()).unwrap();
    assert_eq!(store.config().unwrap(), config);
    let serialized = serde_json::to_value(&config).unwrap();
    assert_eq!(serialized["agents"][1]["reuseSavedAuth"], true);
    assert!(serialized["agents"][0].get("reuseSavedAuth").is_none());
    assert!(serialized["agents"][2].get("reuseSavedAuth").is_none());

    config.agents[1].reuse_saved_auth = false;
    store.save_config(config.clone()).unwrap();
    assert_eq!(store.config().unwrap(), config);
    assert!(
        store.snapshot().unwrap()["config"]["agents"][1]
            .get("reuseSavedAuth")
            .is_none()
    );
}

#[test]
fn saved_auth_for_other_agents_is_rejected_without_overwriting_saved_choice() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let mut config = CollaborationConfig::default();
    config.agents[1].reuse_saved_auth = true;
    store.save_config(config.clone()).unwrap();
    for index in [0, 2] {
        let mut invalid = config.clone();
        invalid.agents[index].reuse_saved_auth = true;
        assert!(store.save_config(invalid).is_err());
        assert_eq!(store.config().unwrap(), config);
    }
}

#[test]
fn dsh_model_routes_round_trip_without_changing_other_agents_or_native_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let mut config = CollaborationConfig::default();
    for route in [
        "deepseek-account/deepseek-flash",
        "deepseek-official/deepseek-v4-pro",
        "provider.custom_v2/nested/model",
        "",
    ] {
        config.agents[1].model = route.to_string();
        store.save_config(config.clone()).unwrap();
        assert_eq!(store.config().unwrap(), config);
        assert_eq!(
            store.snapshot().unwrap()["config"]["agents"][1]["model"],
            route
        );
    }
    config.agents[1].model = "  deepseek-account/deepseek-flash  ".to_string();
    store.save_config(config).unwrap();
    assert_eq!(
        store.config().unwrap().agents[1].model,
        "deepseek-account/deepseek-flash"
    );
}

#[test]
fn invalid_dsh_model_routes_preserve_saved_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let mut config = CollaborationConfig::default();
    config.agents[1].model = "deepseek-account/deepseek-flash".to_string();
    store.save_config(config.clone()).unwrap();
    for route in [
        "model-only",
        "/deepseek-flash",
        "deepseek-account/",
        "provider name/model",
        "供应商/model",
        "provider:custom/model",
        "provider/-model",
        "provider/model\nother",
    ] {
        let mut invalid = config.clone();
        invalid.agents[1].model = route.to_string();
        assert!(store.save_config(invalid).is_err(), "accepted {route:?}");
        assert_eq!(store.config().unwrap(), config);
    }
    let mut oversized = config.clone();
    oversized.agents[1].model = format!("provider/{}", "x".repeat(256));
    assert!(store.save_config(oversized).is_err());
    assert_eq!(store.config().unwrap(), config);
}

#[test]
fn config_rejects_shell_commands_and_unknown_workers_without_overwriting_saved_choice() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex"]);
    let saved = store.config().unwrap();
    let mut changed = saved.clone();
    changed.agents[0].executable = "sh -c 'echo injected'".to_string();
    assert!(store.save_config(changed).is_err());
    let mut changed = saved.clone();
    changed.agents[0].id = "shell".to_string();
    assert!(store.save_config(changed).is_err());
    assert_eq!(store.config().unwrap(), saved);
    let mut invalid = request(&temp, "codex");
    invalid.cwd = ".".to_string();
    assert!(store.submit(invalid).is_err());
}

#[test]
fn cancellation_wins_over_late_completion_and_disabling_stops_queued_and_running_work() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex", "claude"]);
    let running = store.submit(request(&temp, "codex")).unwrap();
    assert!(store.claim(&running.id).unwrap());
    assert!(store.cancel(&running.id).unwrap().cancel_requested);
    let finished = store
        .finish(
            &running.id,
            "completed",
            "late result",
            "",
            "native-session",
        )
        .unwrap();
    assert_eq!(finished.status, "cancelled");
    let frozen = store
        .finish(&running.id, "completed", "replacement", "", "other-session")
        .unwrap();
    assert_eq!(frozen, finished);

    let running = store.submit(request(&temp, "codex")).unwrap();
    let queued = store.submit(request(&temp, "claude")).unwrap();
    assert!(store.claim(&running.id).unwrap());
    let mut config = store.config().unwrap();
    config.enabled = false;
    store.save_config(config).unwrap();
    assert!(store.worker_should_stop(&running.id).unwrap());
    assert_eq!(store.task(&queued.id).unwrap().status, "cancelled");
    assert!(!store.claim(&queued.id).unwrap());
}

#[test]
fn continuation_is_scoped_to_completed_native_session_and_rejects_concurrent_work() {
    let temp = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex", "claude"]);
    let parent = store.submit(request(&temp, "codex")).unwrap();
    // 同一 worker 繁忙时新任务排队而不是被拒绝（#2772）；取消后不影响下面的续聊校验。
    let queued = store.submit(request(&temp, "codex")).unwrap();
    assert_eq!(queued.status, "queued");
    store.cancel(&queued.id).unwrap();
    store.claim(&parent.id).unwrap();
    store
        .finish(&parent.id, "completed", "done", "", "native-session")
        .unwrap();
    let mut next = request(&temp, "claude");
    next.continue_task_id = Some(parent.id.clone());
    assert!(store.submit(next).is_err());
    let mut next = request(&other, "codex");
    next.continue_task_id = Some(parent.id.clone());
    assert!(store.submit(next).is_err());
    let mut next = request(&temp, "codex");
    next.continue_task_id = Some(parent.id.clone());
    let continued = store.submit(next.clone()).unwrap();
    assert_eq!(continued.session_id, "native-session");
    assert_eq!(
        continued.parent_task_id.as_deref(),
        Some(parent.id.as_str())
    );
    assert!(store.submit(next).is_err(), "同一会话不能并发续聊");
}

#[test]
fn child_claim_helper() {
    let Some(path) = std::env::var_os("CODEX_PLUS_COLLAB_TEST_DB") else {
        return;
    };
    let id = std::env::var("CODEX_PLUS_COLLAB_TEST_TASK").unwrap();
    let store = Store::new(PathBuf::from(path));
    println!("COLLAB_CLAIM_RESULT={}", store.claim(&id).unwrap());
}

#[test]
fn two_processes_observe_the_same_store_and_only_one_can_claim_a_task() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex"]);
    let task = store.submit(request(&temp, "codex")).unwrap();
    let child = || {
        std::process::Command::new(std::env::current_exe().unwrap())
            // 合并测试程序后用例名带模块前缀。
            .args([
                "--exact",
                "agent_collaboration::child_claim_helper",
                "--nocapture",
            ])
            .env("CODEX_PLUS_COLLAB_TEST_DB", store.path())
            .env("CODEX_PLUS_COLLAB_TEST_TASK", &task.id)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let first = child();
    let second = child();
    let outputs = [
        first.wait_with_output().unwrap(),
        second.wait_with_output().unwrap(),
    ];
    assert!(outputs.iter().all(|output| output.status.success()));
    assert_eq!(
        outputs
            .iter()
            .filter(|output| String::from_utf8_lossy(&output.stdout)
                .contains("COLLAB_CLAIM_RESULT=true"))
            .count(),
        1
    );
    assert_eq!(
        Store::new(store.path().to_path_buf())
            .task(&task.id)
            .unwrap()
            .status,
        "running"
    );
}

#[test]
fn stale_worker_is_interrupted_and_cannot_publish_a_late_success() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex"]);
    let task = store.submit(request(&temp, "codex")).unwrap();
    store.claim(&task.id).unwrap();
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let payload: String = connection
        .query_row(
            "SELECT payload FROM collaboration_state WHERE id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut state: Value = serde_json::from_str(&payload).unwrap();
    state["tasks"][0]["updatedAtMs"] = json!(1);
    connection
        .execute(
            "UPDATE collaboration_state SET payload=?1 WHERE id=1",
            [state.to_string()],
        )
        .unwrap();
    let result = store.task(&task.id).unwrap();
    assert_eq!(result.status, "interrupted");
    assert!(result.error.contains("不确定"));
    assert_eq!(
        store
            .finish(&task.id, "completed", "late", "", "native-session")
            .unwrap()
            .status,
        "interrupted"
    );
}

#[test]
fn oversized_answer_is_a_failure_instead_of_silent_successful_truncation() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex"]);
    let task = store.submit(request(&temp, "codex")).unwrap();
    store.claim(&task.id).unwrap();
    let answer = "x".repeat(codex_plus_core::agent_collaboration::MAX_OUTPUT_BYTES + 1);
    let result = store
        .finish(&task.id, "completed", &answer, "", "session")
        .unwrap();
    assert_eq!(result.status, "failed");
    assert!(result.answer.is_empty());
    assert!(result.error.contains("上限"));
}

#[test]
fn recent_history_does_not_hide_an_older_active_task() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex", "claude"]);
    let active = store.submit(request(&temp, "codex")).unwrap();
    store.claim(&active.id).unwrap();
    for _ in 0..55 {
        let task = store.submit(request(&temp, "claude")).unwrap();
        store.claim(&task.id).unwrap();
        store.finish(&task.id, "completed", "done", "", "").unwrap();
        store.touch(&active.id).unwrap();
    }
    let snapshot = store.snapshot().unwrap();
    let tasks = snapshot["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 50);
    assert!(
        tasks
            .iter()
            .any(|task| task["id"] == active.id && task["status"] == "running")
    );
}

#[test]
fn mcp_protocol_keeps_errors_per_request_and_never_exposes_configuration_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    assert_eq!(
        mcp::process_line(&store, b"{").unwrap()["error"]["code"],
        -32700
    );
    assert!(
        mcp::handle_request(
            &store,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none()
    );
    let initialized = mcp::handle_request(&store, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    let tools = mcp::handle_request(
        &store,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .unwrap();
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_agents",
            "delegate_to_deepseek",
            "delegate_to_claude_code",
            "delegate_to_codex",
            "delegate_task",
            "send_task",
            "wait_task",
            "get_task",
            "list_tasks",
            "cancel_task"
        ]
    );
    let rejected = mcp::handle_request(&store, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"send_task","arguments":request(&temp,"codex")}})).unwrap();
    assert_eq!(rejected["result"]["isError"], true);
    let ping =
        mcp::handle_request(&store, json!({"jsonrpc":"2.0","id":"next","method":"ping"})).unwrap();
    assert_eq!(ping["id"], "next");
    assert_eq!(ping["result"], json!({}));
    assert!(
        store.snapshot().unwrap()["tasks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn sub_agent_runtime_limit_defaults_to_ten_minutes_and_is_configurable() {
    // #2618：运行上限过去写死 10 分钟，现在可在协作设置中调整。
    let legacy: CollaborationConfig = serde_json::from_value(json!({
        "enabled": true,
        "defaultAgentId": "deepseek",
        "agents": serde_json::to_value(CollaborationConfig::default().agents).unwrap()
    }))
    .unwrap();
    assert_eq!(legacy.max_runtime_minutes, 10, "旧配置缺省字段时保持原来的 10 分钟");
    assert!(
        serde_json::to_value(&legacy).unwrap().get("maxRuntimeMinutes").is_none(),
        "默认值不写出，回退到 deny_unknown_fields 的旧版本仍能读取"
    );

    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let mut config = CollaborationConfig::default();
    config.max_runtime_minutes = 45;
    store.save_config(config.clone()).unwrap();
    assert_eq!(store.config().unwrap().max_runtime_minutes, 45);
    assert_eq!(
        serde_json::to_value(store.config().unwrap()).unwrap()["maxRuntimeMinutes"],
        45
    );

    for invalid in [0, 241] {
        let mut bad = config.clone();
        bad.max_runtime_minutes = invalid;
        assert!(store.save_config(bad).is_err(), "{invalid} 分钟应被拒绝");
    }
    assert_eq!(store.config().unwrap().max_runtime_minutes, 45, "拒绝时不改已保存的值");
}
