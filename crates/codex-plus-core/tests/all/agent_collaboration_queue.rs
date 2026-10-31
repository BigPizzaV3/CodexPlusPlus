//! 同一 Agent 的任务排队（#2772）：繁忙时新任务排队、按提交顺序启动、有上限、
//! 可取消排队中的任务，前一个失败后下一个仍会启动；执行进程失联时排队任务不会卡在 queued。
//! 同一 Agent 并发执行（#2800）：运行中的任务数小于并发上限时直接启动，达到上限后排队；
//! 多个进程同时轮询也不超发。
//! 这些用例只用各自的临时状态文件，不改进程级状态，无需全局锁。

use codex_plus_core::agent_collaboration::{
    CollaborationConfig, DEFAULT_MAX_CONCURRENT_TASKS, MAX_CONCURRENT_TASKS_LIMIT,
    MAX_QUEUED_PER_AGENT, QueueTurn, Store, TaskRequest,
};
use serde_json::{Value, json};

fn store(temp: &tempfile::TempDir) -> Store {
    Store::new(temp.path().join("collaboration.sqlite3"))
}

fn enable(store: &Store, ids: &[&str]) {
    let mut config = CollaborationConfig::default();
    config.enabled = true;
    for agent in &mut config.agents {
        agent.enabled = ids.contains(&agent.id.as_str());
        // 仅作为真实存在、可执行的文件通过路径探测；Store 层用例不会启动它。
        agent.executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
    }
    store.save_config(config).unwrap();
}

fn request(temp: &tempfile::TempDir, agent: &str, prompt: &str) -> TaskRequest {
    TaskRequest {
        agent_id: agent.to_string(),
        prompt: prompt.to_string(),
        cwd: temp.path().to_string_lossy().into_owned(),
        continue_task_id: None,
    }
}

#[test]
fn busy_agent_queues_new_tasks_and_starts_them_in_submission_order_even_after_failure() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex", "claude"]);
    let first = store.submit(request(&temp, "codex", "第一个")).unwrap();
    let second = store.submit(request(&temp, "codex", "第二个")).unwrap();
    let third = store.submit(request(&temp, "codex", "第三个")).unwrap();
    assert_eq!(first.progress, "等待启动 CLI");
    assert_eq!(second.status, "queued");
    assert!(
        second.progress.contains("前面还有 1 个"),
        "{}",
        second.progress
    );
    assert!(
        third.progress.contains("前面还有 2 个"),
        "{}",
        third.progress
    );

    // 不同 Agent 之间仍然并行。
    let other = store.submit(request(&temp, "claude", "并行")).unwrap();
    assert!(store.claim(&other.id).unwrap());

    // 后提交的任务不能插队。
    assert!(!store.claim(&third.id).unwrap());
    assert!(!store.claim(&second.id).unwrap());
    assert!(store.claim(&first.id).unwrap());
    assert_eq!(
        store.poll_queue(&third.id).unwrap(),
        QueueTurn::Wait { ahead: 2 }
    );
    assert_eq!(store.task(&third.id).unwrap().status, "queued");

    // 前一个失败后，下一个照常按顺序启动。
    store
        .finish(&first.id, "failed", "", "原生 CLI 登录失败", "")
        .unwrap();
    assert!(!store.claim(&third.id).unwrap());
    assert_eq!(
        store.poll_queue(&third.id).unwrap(),
        QueueTurn::Wait { ahead: 1 }
    );
    assert!(
        store
            .task(&third.id)
            .unwrap()
            .progress
            .contains("前面还有 1 个")
    );
    assert!(store.claim(&second.id).unwrap());
    assert_eq!(store.task(&second.id).unwrap().status, "running");

    store
        .finish(&second.id, "completed", "done", "", "native-2")
        .unwrap();
    assert_eq!(store.poll_queue(&third.id).unwrap(), QueueTurn::Start);
    assert_eq!(store.task(&third.id).unwrap().status, "running");
    // 已认领的任务不会被重复启动。
    assert_eq!(store.poll_queue(&third.id).unwrap(), QueueTurn::Stop);
}

#[test]
fn queue_has_a_per_agent_limit_with_a_clear_error() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex", "claude"]);
    let running = store.submit(request(&temp, "codex", "运行中")).unwrap();
    assert!(store.claim(&running.id).unwrap());
    let mut queued = Vec::new();
    for index in 0..MAX_QUEUED_PER_AGENT {
        queued.push(
            store
                .submit(request(&temp, "codex", &format!("排队 {index}")))
                .unwrap(),
        );
    }
    let error = store
        .submit(request(&temp, "codex", "超出上限"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("上限"), "{error}");
    assert!(error.contains(&MAX_QUEUED_PER_AGENT.to_string()), "{error}");
    assert_eq!(
        store.snapshot().unwrap()["tasks"].as_array().unwrap().len(),
        MAX_QUEUED_PER_AGENT + 1,
        "超限时不写入任务"
    );
    assert_eq!(
        store.snapshot().unwrap()["capabilities"]["maxQueuedPerAgent"],
        MAX_QUEUED_PER_AGENT
    );

    // 上限按 Agent 计算，其他 Agent 不受影响。
    store
        .submit(request(&temp, "claude", "其他 Agent"))
        .unwrap();
    // 取消一个排队任务后腾出位置。
    store.cancel(&queued[0].id).unwrap();
    store.submit(request(&temp, "codex", "补位")).unwrap();
}

#[test]
fn cancelling_a_queued_task_removes_it_from_the_queue() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex"]);
    let first = store.submit(request(&temp, "codex", "第一个")).unwrap();
    let second = store.submit(request(&temp, "codex", "第二个")).unwrap();
    let third = store.submit(request(&temp, "codex", "第三个")).unwrap();
    assert!(store.claim(&first.id).unwrap());

    let cancelled = store.cancel(&second.id).unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert!(cancelled.progress.contains("尚未启动"));
    assert_eq!(store.poll_queue(&second.id).unwrap(), QueueTurn::Stop);
    assert!(!store.claim(&second.id).unwrap());
    assert_eq!(
        store.poll_queue(&third.id).unwrap(),
        QueueTurn::Wait { ahead: 1 }
    );

    store
        .finish(&first.id, "completed", "done", "", "native-1")
        .unwrap();
    assert!(store.claim(&third.id).unwrap());
    assert_eq!(store.task(&second.id).unwrap().status, "cancelled");
}

/// 直接改写持久化记录，模拟执行进程失联或电脑睡眠后租约过期。
fn rewrite_state(store: &Store, change: impl FnOnce(&mut Value)) {
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let payload: String = connection
        .query_row(
            "SELECT payload FROM collaboration_state WHERE id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut state: Value = serde_json::from_str(&payload).unwrap();
    change(&mut state);
    connection
        .execute(
            "UPDATE collaboration_state SET payload=?1 WHERE id=1",
            [state.to_string()],
        )
        .unwrap();
}

/// 改写持久化记录里某个任务的字段。
fn rewrite_task(store: &Store, index: usize, field: &str, value: Value) {
    rewrite_state(store, |state| state["tasks"][index][field] = value);
}

/// 电脑睡眠或进程卡顿超过租约：排队任务的持有进程（这里是测试进程自己）仍在，
/// 任务不能被判成失联；持有者下一次轮询照常续租。持有进程真的不在了才按失联处理。
#[test]
fn queued_task_survives_an_expired_lease_while_its_owner_process_is_alive() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex"]);
    let running = store.submit(request(&temp, "codex", "运行中")).unwrap();
    assert!(store.claim(&running.id).unwrap());
    let waiting = store.submit(request(&temp, "codex", "睡眠前排队")).unwrap();
    // 第一次轮询登记本进程为持有者。
    assert_eq!(
        store.poll_queue(&waiting.id).unwrap(),
        QueueTurn::Wait { ahead: 1 }
    );

    // 睡眠醒来：租约早已过期，但持有进程还在。
    rewrite_task(&store, 1, "updatedAtMs", json!(1));
    let task = store.task(&waiting.id).unwrap();
    assert_eq!(task.status, "queued", "{}", task.error);
    assert_eq!(
        store.poll_queue(&waiting.id).unwrap(),
        QueueTurn::Wait { ahead: 1 }
    );
    assert!(
        store.task(&waiting.id).unwrap().updated_at_ms > 1,
        "持有者轮询时续租"
    );
    store
        .finish(&running.id, "completed", "done", "", "native-1")
        .unwrap();
    assert_eq!(store.poll_queue(&waiting.id).unwrap(), QueueTurn::Start);

    // 持有者换成一个已经不在的进程（标记文件不存在）：租约过期后照常按失联处理。
    let orphan = store
        .submit(request(&temp, "codex", "持有进程已退出"))
        .unwrap();
    assert_eq!(
        store.poll_queue(&orphan.id).unwrap(),
        QueueTurn::Wait { ahead: 1 }
    );
    let orphan_id = orphan.id.clone();
    rewrite_state(&store, move |state| {
        state["queueOwners"][orphan_id.as_str()] = json!(uuid::Uuid::new_v4().to_string());
        state["tasks"][2]["updatedAtMs"] = json!(1);
    });
    let result = store.task(&orphan.id).unwrap();
    assert_eq!(result.status, "failed");
    assert!(result.error.contains("未启动"), "{}", result.error);
}

#[test]
fn queued_tasks_of_a_lost_process_fail_with_a_reason_instead_of_staying_queued() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable(&store, &["codex"]);
    let running = store.submit(request(&temp, "codex", "运行中")).unwrap();
    assert!(store.claim(&running.id).unwrap());
    let lost = store
        .submit(request(&temp, "codex", "失联进程的排队任务"))
        .unwrap();
    let next = store.submit(request(&temp, "codex", "之后提交")).unwrap();

    // 提交 lost 的进程崩溃，不再续租：租约过期后标记为未启动的失败。
    rewrite_task(&store, 1, "updatedAtMs", json!(1));
    let result = store.task(&lost.id).unwrap();
    assert_eq!(result.status, "failed");
    assert!(result.error.contains("未启动"), "{}", result.error);
    assert!(result.error.contains("重新提交"), "{}", result.error);
    assert_eq!(
        store.poll_queue(&next.id).unwrap(),
        QueueTurn::Wait { ahead: 1 },
        "失联的排队任务不再挡住后面的任务"
    );

    // 正常退出时由执行进程主动放弃仍在排队的任务；已启动的任务不受影响。
    assert!(
        store
            .abandon_queued(&next.id, "协作执行进程已退出，任务未启动")
            .unwrap()
    );
    let abandoned = store.task(&next.id).unwrap();
    assert_eq!(abandoned.status, "failed");
    assert_eq!(abandoned.error, "协作执行进程已退出，任务未启动");
    assert!(!store.abandon_queued(&running.id, "不应生效").unwrap());
    assert_eq!(store.task(&running.id).unwrap().status, "running");
}

#[cfg(unix)]
#[tokio::test]
async fn mcp_delegations_to_a_busy_agent_queue_and_run_one_after_another() {
    use codex_plus_core::agent_collaboration::mcp;
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let log = temp.path().join("order.log");
    let executable = temp.path().join("controlled-codex");
    // 测试逐字定义的可控 CLI：记录开始/结束顺序，prompt 以 fail 开头时失败退出。
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\np=$(/bin/cat)\necho \"start $p\" >> '{log}'\n/bin/sleep 0.2\necho \"end $p\" >> '{log}'\ncase \"$p\" in fail*) exit 2;; esac\nprintf '%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"native-1\"}}' '{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"done\"}}}}' '{{\"type\":\"turn.completed\",\"usage\":{{}}}}'\n",
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = CollaborationConfig::default();
    config.enabled = true;
    let agent = config
        .agents
        .iter_mut()
        .find(|agent| agent.id == "codex")
        .unwrap();
    agent.enabled = true;
    agent.executable = executable.to_string_lossy().into_owned();
    store.save_config(config).unwrap();

    let mut ids = Vec::new();
    for (index, prompt) in ["fail-1", "ok-2", "ok-3"].into_iter().enumerate() {
        let response = mcp::handle_request(
            &store,
            json!({"jsonrpc":"2.0","id":index,"method":"tools/call","params":{"name":"delegate_to_codex",
                "arguments":{"prompt":prompt,"cwd":temp.path().to_str().unwrap()}}}),
        )
        .unwrap();
        assert_eq!(response["result"]["isError"], false, "{response}");
        let result: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(result["task"]["status"], "queued");
        if index > 0 {
            let progress = result["task"]["progress"].as_str().unwrap();
            assert!(
                progress.contains(&format!("前面还有 {index} 个")),
                "{progress}"
            );
        }
        ids.push(result["taskId"].as_str().unwrap().to_owned());
    }

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while !ids.iter().all(|id| {
            codex_plus_core::agent_collaboration::is_terminal(&store.task(id).unwrap().status)
        }) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("排队任务应在前一个结束后自动启动并完成");

    // 各自按 taskId 收取结果。
    let statuses: Vec<_> = ids
        .iter()
        .map(|id| {
            let response = mcp::handle_request(
                &store,
                json!({"jsonrpc":"2.0","id":"get","method":"tools/call","params":{"name":"get_task","arguments":{"taskId":id}}}),
            )
            .unwrap();
            let task: Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            (task["status"].as_str().unwrap().to_owned(), task["answer"].as_str().unwrap().to_owned())
        })
        .collect();
    assert_eq!(
        statuses,
        [
            ("failed".to_owned(), String::new()),
            ("completed".to_owned(), "done".to_owned()),
            ("completed".to_owned(), "done".to_owned()),
        ]
    );
    // 同一 Agent 严格串行：上一个结束后下一个才开始。
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "start fail-1",
            "end fail-1",
            "start ok-2",
            "end ok-2",
            "start ok-3",
            "end ok-3"
        ]
    );
}

// ── 同一 Agent 并发执行（#2800）──────────────────────────────────────────────

/// 启用 ids 并把 agent 的并发上限设为 limit。
fn enable_with_limit(store: &Store, ids: &[&str], agent: &str, limit: u32) {
    enable(store, ids);
    let mut config = store.config().unwrap();
    config
        .agents
        .iter_mut()
        .find(|item| item.id == agent)
        .unwrap()
        .max_concurrent_tasks = limit;
    store.save_config(config).unwrap();
}

#[test]
fn concurrency_limit_defaults_to_one_round_trips_and_rejects_out_of_range_values() {
    let legacy: CollaborationConfig = serde_json::from_value(json!({"enabled":false,"agents":[
        {"id":"codex","name":"Codex","executable":"","enabled":false},
        {"id":"deepseek","name":"DeepSeek Harness","executable":"","enabled":false},
        {"id":"claude","name":"Claude Code","executable":"","enabled":false}
    ]}))
    .unwrap();
    assert!(
        legacy
            .agents
            .iter()
            .all(|agent| agent.max_concurrent_tasks == DEFAULT_MAX_CONCURRENT_TASKS)
    );
    assert!(
        serde_json::to_value(&legacy).unwrap()["agents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|agent| agent.get("maxConcurrentTasks").is_none()),
        "默认值不写出，回退到 deny_unknown_fields 的旧版本仍能读取"
    );

    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable_with_limit(&store, &["codex"], "codex", 3);
    let saved = store.config().unwrap();
    assert_eq!(saved.agents[0].max_concurrent_tasks, 3);
    assert_eq!(
        store.snapshot().unwrap()["config"]["agents"][0]["maxConcurrentTasks"],
        3
    );
    assert_eq!(
        store.snapshot().unwrap()["capabilities"]["maxConcurrentTasksLimit"],
        MAX_CONCURRENT_TASKS_LIMIT
    );
    for invalid in [0, MAX_CONCURRENT_TASKS_LIMIT + 1] {
        let mut bad = saved.clone();
        bad.agents[0].max_concurrent_tasks = invalid;
        let error = store.save_config(bad).unwrap_err().to_string();
        assert!(error.contains("同时执行"), "{error}");
    }
    assert_eq!(store.config().unwrap(), saved, "拒绝时不改已保存的值");
}

#[test]
fn concurrency_limit_two_runs_two_tasks_queues_the_third_and_starts_it_when_either_ends() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable_with_limit(&store, &["codex", "claude"], "codex", 2);
    let first = store.submit(request(&temp, "codex", "第一个")).unwrap();
    let second = store.submit(request(&temp, "codex", "第二个")).unwrap();
    let third = store.submit(request(&temp, "codex", "第三个")).unwrap();
    let fourth = store.submit(request(&temp, "codex", "第四个")).unwrap();
    assert_eq!(first.progress, "等待启动 CLI");
    // 前面只有一个排队任务、没有运行中的任务时也按提交顺序等待。
    assert!(
        second.progress.contains("前面还有 1 个"),
        "{}",
        second.progress
    );
    assert!(
        third.progress.contains("前面还有 2 个"),
        "{}",
        third.progress
    );

    // 后提交的任务不能插队：第二个要等第一个先启动。
    assert_eq!(
        store.poll_queue(&second.id).unwrap(),
        QueueTurn::Wait { ahead: 1 }
    );
    assert!(store.claim(&first.id).unwrap());
    // 运行中 1 个 < 上限 2：第二个直接启动，两个同时 running。
    assert!(store.claim(&second.id).unwrap());
    assert_eq!(store.task(&first.id).unwrap().status, "running");
    assert_eq!(store.task(&second.id).unwrap().status, "running");

    // 达到上限：第三个排队，第四个排在它后面。
    assert_eq!(
        store.poll_queue(&third.id).unwrap(),
        QueueTurn::Wait { ahead: 1 }
    );
    assert_eq!(
        store.poll_queue(&fourth.id).unwrap(),
        QueueTurn::Wait { ahead: 2 }
    );
    assert!(
        store
            .task(&third.id)
            .unwrap()
            .progress
            .contains("前面还有 1 个")
    );
    // 其他 Agent 不受影响。
    let other = store.submit(request(&temp, "claude", "并行")).unwrap();
    assert!(store.claim(&other.id).unwrap());

    // 后启动的那个先结束，同样腾出位置：第三个按顺序启动，第四个仍要等。
    store
        .finish(&second.id, "completed", "done", "", "native-2")
        .unwrap();
    assert!(!store.claim(&fourth.id).unwrap());
    assert_eq!(store.poll_queue(&third.id).unwrap(), QueueTurn::Start);
    assert_eq!(
        store.poll_queue(&fourth.id).unwrap(),
        QueueTurn::Wait { ahead: 1 }
    );

    // 运行中的任务失败后，排队任务照常启动。
    store
        .finish(&first.id, "failed", "", "原生 CLI 登录失败", "")
        .unwrap();
    assert_eq!(store.poll_queue(&fourth.id).unwrap(), QueueTurn::Start);
    let statuses: Vec<_> = [&first, &second, &third, &fourth]
        .iter()
        .map(|task| store.task(&task.id).unwrap().status)
        .collect();
    assert_eq!(statuses, ["failed", "completed", "running", "running"]);
}

#[test]
fn concurrent_tasks_in_the_same_cwd_get_a_hint_but_are_not_blocked() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable_with_limit(&store, &["codex"], "codex", 2);
    let first = store.submit(request(&temp, "codex", "第一个")).unwrap();
    assert_eq!(store.overlapping_cwd_tasks(&first.id).unwrap(), 0);
    assert!(store.claim(&first.id).unwrap());
    assert_eq!(store.task(&first.id).unwrap().progress, "正在启动 CLI");
    let second = store.submit(request(&temp, "codex", "第二个")).unwrap();
    assert_eq!(store.overlapping_cwd_tasks(&second.id).unwrap(), 1);
    assert!(store.claim(&second.id).unwrap());
    let progress = store.task(&second.id).unwrap().progress;
    assert!(progress.contains("同一工作目录"), "{progress}");

    // 并发上限为 1 时同一 Agent 的任务不会同时运行，不提示。
    let serial = tempfile::tempdir().unwrap();
    let serial_store = Store::new(serial.path().join("collaboration.sqlite3"));
    enable(&serial_store, &["codex"]);
    let running = serial_store
        .submit(request(&serial, "codex", "运行中"))
        .unwrap();
    assert!(serial_store.claim(&running.id).unwrap());
    let queued = serial_store
        .submit(request(&serial, "codex", "排队"))
        .unwrap();
    assert_eq!(serial_store.overlapping_cwd_tasks(&queued.id).unwrap(), 0);
}

#[test]
fn concurrent_cancel_and_lost_process_handling_still_apply_with_a_higher_limit() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable_with_limit(&store, &["codex"], "codex", 2);
    let first = store.submit(request(&temp, "codex", "第一个")).unwrap();
    let second = store.submit(request(&temp, "codex", "第二个")).unwrap();
    assert!(store.claim(&first.id).unwrap());
    assert!(store.claim(&second.id).unwrap());
    let lost = store.submit(request(&temp, "codex", "失联")).unwrap();
    let cancelled = store.submit(request(&temp, "codex", "取消")).unwrap();
    let last = store.submit(request(&temp, "codex", "最后")).unwrap();

    // 取消排队任务即移出队列。
    assert_eq!(store.cancel(&cancelled.id).unwrap().status, "cancelled");
    assert_eq!(store.poll_queue(&cancelled.id).unwrap(), QueueTurn::Stop);
    // 提交 lost 的进程崩溃：租约过期后标记为未启动的失败，不再挡住后面的任务。
    rewrite_task(&store, 2, "updatedAtMs", json!(1));
    let result = store.task(&lost.id).unwrap();
    assert_eq!(result.status, "failed");
    assert!(result.error.contains("未启动"), "{}", result.error);
    assert_eq!(
        store.poll_queue(&last.id).unwrap(),
        QueueTurn::Wait { ahead: 1 }
    );

    // 取消运行中的任务：在 worker 结束前仍占着位置，结束后下一个启动。
    store.cancel(&first.id).unwrap();
    assert!(!store.claim(&last.id).unwrap());
    store
        .finish(&first.id, "completed", "late", "", "")
        .unwrap();
    assert_eq!(store.task(&first.id).unwrap().status, "cancelled");
    assert_eq!(store.poll_queue(&last.id).unwrap(), QueueTurn::Start);
}

/// 子进程用例：反复轮询指定任务直到轮到或超时，输出是否认领成功。
/// 只在父用例设置环境变量时生效，单独运行时直接返回。
#[test]
fn child_concurrent_poll_helper() {
    let Some(path) = std::env::var_os("CODEX_PLUS_COLLAB_2800_DB") else {
        return;
    };
    let id = std::env::var("CODEX_PLUS_COLLAB_2800_TASK").unwrap();
    let store = Store::new(std::path::PathBuf::from(path));
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(3000);
    let mut started = false;
    while std::time::Instant::now() < deadline {
        match store.poll_queue(&id).unwrap() {
            QueueTurn::Start => {
                started = true;
                break;
            }
            QueueTurn::Stop => break,
            QueueTurn::Wait { .. } => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    println!("COLLAB_2800_STARTED={started}");
}

/// 多个进程同时轮询同一 Agent 的排队任务：判断和认领在同一个 IMMEDIATE 事务里，
/// 不会超过并发上限；运行中的任务都不结束时，恰好上限个任务启动，其余继续排队。
#[test]
fn processes_polling_at_once_never_start_more_tasks_than_the_concurrency_limit() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    enable_with_limit(&store, &["codex"], "codex", 2);
    let tasks: Vec<_> = (0..5)
        .map(|index| {
            store
                .submit(request(&temp, "codex", &format!("并发 {index}")))
                .unwrap()
        })
        .collect();
    let children: Vec<_> = tasks
        .iter()
        .map(|task| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "agent_collaboration_queue::child_concurrent_poll_helper",
                    "--nocapture",
                ])
                .env("CODEX_PLUS_COLLAB_2800_DB", store.path())
                .env("CODEX_PLUS_COLLAB_2800_TASK", &task.id)
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let started: Vec<bool> = children
        .into_iter()
        .map(|child| {
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success());
            String::from_utf8_lossy(&output.stdout).contains("COLLAB_2800_STARTED=true")
        })
        .collect();
    // 按提交顺序：前两个启动，后三个排队。
    assert_eq!(started, [true, true, false, false, false]);
    let statuses: Vec<_> = tasks
        .iter()
        .map(|task| store.task(&task.id).unwrap().status)
        .collect();
    assert_eq!(
        statuses,
        ["running", "running", "queued", "queued", "queued"]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn mcp_delegations_run_concurrently_up_to_the_limit_and_queue_the_rest() {
    use codex_plus_core::agent_collaboration::mcp;
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    let log = temp.path().join("order.log");
    let executable = temp.path().join("controlled-codex");
    // 可控 CLI：记录开始/结束；slow 开头的任务运行更久，用来确认第三个在任一任务结束后就启动。
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\np=$(/bin/cat)\necho \"start $p\" >> '{log}'\ncase \"$p\" in slow*) /bin/sleep 2;; *) /bin/sleep 0.3;; esac\necho \"end $p\" >> '{log}'\nprintf '%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"native-1\"}}' '{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"done\"}}}}' '{{\"type\":\"turn.completed\",\"usage\":{{}}}}'\n",
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = CollaborationConfig::default();
    config.enabled = true;
    let agent = config
        .agents
        .iter_mut()
        .find(|agent| agent.id == "codex")
        .unwrap();
    agent.enabled = true;
    agent.executable = executable.to_string_lossy().into_owned();
    agent.max_concurrent_tasks = 2;
    store.save_config(config).unwrap();

    let mut ids = Vec::new();
    let mut warnings = Vec::new();
    for (index, prompt) in ["slow-1", "ok-2", "ok-3"].into_iter().enumerate() {
        let response = mcp::handle_request(
            &store,
            json!({"jsonrpc":"2.0","id":index,"method":"tools/call","params":{"name":"delegate_to_codex",
                "arguments":{"prompt":prompt,"cwd":temp.path().to_str().unwrap()}}}),
        )
        .unwrap();
        assert_eq!(response["result"]["isError"], false, "{response}");
        let result: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        warnings.push(result.get("cwdWarning").is_some());
        ids.push(result["taskId"].as_str().unwrap().to_owned());
    }
    // 同一 cwd 可能并发时给提示，但不拒绝。
    assert_eq!(warnings, [false, true, true]);

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while !ids.iter().all(|id| {
            codex_plus_core::agent_collaboration::is_terminal(&store.task(id).unwrap().status)
        }) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("并发任务和排队任务都应完成");
    for id in &ids {
        assert_eq!(store.task(id).unwrap().status, "completed");
    }
    let lines: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let at = |line: &str| lines.iter().position(|item| item == line).unwrap();
    // 前两个同时运行：第二个在第一个结束前就已开始。
    assert!(at("start ok-2") < at("end slow-1"), "{lines:?}");
    // 第三个排队：任一任务结束后才开始，不必等两个都结束。
    assert!(at("start ok-3") > at("end ok-2"), "{lines:?}");
    assert!(at("start ok-3") < at("end slow-1"), "{lines:?}");
}
