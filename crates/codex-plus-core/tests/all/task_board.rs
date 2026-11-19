use codex_plus_core::task_board::{
    apply_auto_status, auto_status, bridge_route, create_task, delete_task, dispatch, is_scratch_dir, linked_runs, list_tasks,
    new_thread_link, parse_rollout_timestamp, parse_task, read_task, recent_projects, render_task, rollout_run_state,
    set_codex_home_for_tests, set_picked_folder_for_tests, set_tasks_dir_for_tests,
    take_opened_links_for_tests, task_marker_id, task_prompt, update_task, NewTask, RunState, Task, TaskPatch, TaskRun, TaskStatus,
};
use std::path::Path;
use serde_json::json;

fn new_task(title: &str) -> NewTask {
    NewTask { title: title.to_string(), ..Default::default() }
}

#[test]
fn missing_dir_lists_nothing() {
    let temp = tempfile::tempdir().unwrap();
    assert!(list_tasks(&temp.path().join("tasks")).is_empty());
}

#[test]
fn create_numbers_tasks_and_writes_markdown_files() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("tasks");
    let first = create_task(&dir, new_task("修登录页的报错")).unwrap();
    let second = create_task(
        &dir,
        NewTask { title: "  补测试\n第二行不要  ".into(), body: "细节\n".into(), status: Some(TaskStatus::Todo), project: Some(" D:/repo ".into()) },
    )
    .unwrap();
    assert_eq!((first.id, second.id), (1, 2));
    assert_eq!(first.status, TaskStatus::Backlog, "默认进收集");
    assert_eq!(second.title, "补测试", "标题只取第一行并去空白");
    assert_eq!(second.project.as_deref(), Some("D:/repo"));

    let text = std::fs::read_to_string(dir.join("2.md")).unwrap();
    assert!(text.starts_with("---\ntitle: \"补测试\"\nstatus: \"todo\"\nproject: \"D:/repo\"\n"), "{text}");
    assert!(text.ends_with("---\n\n细节\n"), "{text}");
    assert_eq!(std::fs::read_to_string(dir.join(".tasks.json")).unwrap(), r#"{"next":3}"#);

    let ids: Vec<_> = list_tasks(&dir).into_iter().map(|task| task.id).collect();
    assert_eq!(ids, [2, 1], "新的在前");
}

#[test]
fn create_skips_ids_taken_by_hand_written_files() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::write(dir.join("1.md"), "手动放进来的\n").unwrap();
    let task = create_task(dir, new_task("新的")).unwrap();
    assert_eq!(task.id, 2);
    assert_eq!(read_task(dir, 1).unwrap().title, "手动放进来的", "原文件不被覆盖");
}

#[test]
fn create_requires_title_or_body() {
    let temp = tempfile::tempdir().unwrap();
    assert!(create_task(temp.path(), new_task("   ")).is_err());
    let from_body = create_task(temp.path(), NewTask { body: "# 从正文取标题\n更多".into(), ..Default::default() }).unwrap();
    assert_eq!(from_body.title, "从正文取标题");
}

#[test]
fn update_changes_only_given_fields_and_tracks_status_time() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let task = create_task(dir, NewTask { title: "原标题".into(), body: "原正文".into(), project: Some("D:/a".into()), ..Default::default() }).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(5));
    let moved = update_task(dir, task.id, TaskPatch { status: Some(TaskStatus::InReview), ..Default::default() }).unwrap();
    assert_eq!(moved.status, TaskStatus::InReview);
    assert_eq!((moved.title.as_str(), moved.body.as_str()), ("原标题", "原正文"));
    assert!(moved.status_at > task.status_at && moved.updated_at > task.updated_at);
    assert_eq!(moved.created_at, task.created_at);

    let renamed = update_task(dir, task.id, TaskPatch { title: Some("新标题".into()), project: Some("".into()), ..Default::default() }).unwrap();
    assert_eq!(renamed.status_at, moved.status_at, "没改状态不动状态时间");
    assert_eq!(renamed.project, None, "空字符串清除项目");
    assert_eq!(read_task(dir, task.id).unwrap(), renamed, "落盘后读回一致");

    assert!(update_task(dir, task.id, TaskPatch { title: Some(" ".into()), ..Default::default() }).is_err());
    assert!(update_task(dir, 99, TaskPatch::default()).is_err());
}

#[test]
fn parse_tolerates_hand_edited_files_and_keeps_unknown_keys() {
    let text = "---\r\ntitle: 裸字符串标题\r\nstatus: \"whatever\"\r\nowner: {\"agent\":\"codex\"}\r\n---\r\n\r\n正文\r\n";
    let task = parse_task(7, text, 123);
    assert_eq!(task.title, "裸字符串标题");
    assert_eq!(task.status, TaskStatus::Backlog, "不认识的状态归到收集");
    assert_eq!(task.body, "正文");
    assert_eq!((task.created_at, task.status_at), (123, 123), "缺时间字段时用文件时间");
    let rendered = render_task(&task);
    assert!(rendered.contains("owner: {\"agent\":\"codex\"}\n"), "不认识的键原样写回：{rendered}");
    assert_eq!(parse_task(7, &rendered, 0), task);

    let plain = parse_task(8, "没有 frontmatter 的一行\n第二行", 5);
    assert_eq!(plain.title, "没有 frontmatter 的一行");
    assert_eq!(plain.body, "没有 frontmatter 的一行\n第二行");

    let unclosed = parse_task(9, "---\ntitle: \"x\"\n正文", 5);
    assert_eq!(unclosed.title, "---", "frontmatter 没闭合时整段当正文");
}

#[test]
fn list_ignores_non_task_files() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    create_task(dir, new_task("留下")).unwrap();
    std::fs::write(dir.join("notes.md"), "x").unwrap();
    std::fs::write(dir.join("01.md"), "x").unwrap();
    std::fs::write(dir.join("3.md.tmp"), "x").unwrap();
    std::fs::create_dir_all(dir.join("5.md")).unwrap();
    let ids: Vec<_> = list_tasks(dir).into_iter().map(|task| task.id).collect();
    assert_eq!(ids, [1]);
}

#[test]
fn delete_removes_one_file_and_never_reuses_the_id() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let task = create_task(dir, new_task("删掉")).unwrap();
    delete_task(dir, task.id).unwrap();
    assert!(!dir.join("1.md").exists());
    assert!(delete_task(dir, task.id).is_err());
    assert_eq!(create_task(dir, new_task("下一个")).unwrap().id, 2, "编号不回收");
}

#[test]
fn dispatch_round_trips_through_the_frontend_actions() {
    let _lock = crate::common::global_state_lock();
    let temp = tempfile::tempdir().unwrap();
    set_tasks_dir_for_tests(Some(temp.path().join("tasks")));
    // 不读真机的 Codex 会话：真机里可能正好有带 [T-1] 的对话，会把状态推走。
    set_codex_home_for_tests(Some(temp.path().join("codex-home")));

    let created = dispatch("create", &json!({ "task": { "title": "从界面建的", "status": "todo" } })).unwrap();
    assert_eq!(created["created"], 1);
    assert_eq!(created["tasks"][0]["status"], "todo");
    assert_eq!(created["tasks"][0]["statusAt"], created["tasks"][0]["createdAt"]);

    let updated = dispatch("update", &json!({ "id": 1, "patch": { "status": "in_progress", "body": "正文" } })).unwrap();
    assert_eq!(updated["tasks"][0]["status"], "in_progress");
    assert_eq!(updated["tasks"][0]["body"], "正文");

    assert!(dispatch("update", &json!({ "id": 1, "patch": { "status": "nope" } })).is_err());
    assert!(dispatch("delete", &json!({})).is_err(), "缺编号");
    assert!(dispatch("explode", &json!({})).is_err());

    let listed = dispatch("list", &json!(null)).unwrap();
    assert_eq!(listed["tasks"].as_array().unwrap().len(), 1);
    let deleted = dispatch("delete", &json!({ "id": 1 })).unwrap();
    assert!(deleted["tasks"].as_array().unwrap().is_empty());

    // Codex 页面经 bridge 路由 `/task-board` 传 `{ action, ...args }`，成功时带 status: ok。
    let via_bridge = bridge_route(&json!({ "action": "create", "task": { "title": "从 Codex 建的" } })).unwrap();
    assert_eq!(via_bridge["status"], "ok");
    assert_eq!(via_bridge["tasks"][0]["title"], "从 Codex 建的");
    assert_eq!(bridge_route(&json!({})).unwrap()["tasks"].as_array().unwrap().len(), 1, "缺 action 时按 list");
    assert!(bridge_route(&json!({ "action": "update", "id": 99, "patch": {} })).is_err());

    set_tasks_dir_for_tests(None);
    set_codex_home_for_tests(None);
}

// ---------- 交给 Codex ----------

fn sample_task(id: u64, status: TaskStatus, status_at: i64) -> Task {
    let mut task = parse_task(id, &format!("---\ntitle: \"任务 {id}\"\nstatus: \"{}\"\n---\n", status.as_str()), 0);
    task.status_at = status_at;
    task
}

fn run(task_id: u64, state: RunState, started_at: Option<i64>, finished_at: Option<i64>) -> TaskRun {
    TaskRun {
        task_id,
        thread_id: format!("thread-{task_id}"),
        title: String::new(),
        cwd: String::new(),
        state,
        updated_at: 0,
        started_at,
        finished_at,
        last_message: None,
    }
}

#[test]
fn prompt_and_link_carry_the_task_marker_and_project() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().display().to_string();
    let mut task = parse_task(12, "---\ntitle: \"修登录 & 报错\"\n---\n\n要求：\n- 不能 100% 失败\n", 0);
    assert_eq!(task_prompt(&task), "[T-12] 修登录 & 报错\n\n要求：\n- 不能 100% 失败");

    let link = new_thread_link(&task);
    assert!(link.starts_with("codex://threads/new?prompt=%5BT-12%5D%20%E4%BF%AE"), "{link}");
    assert!(link.contains("%20%26%20") && link.contains("100%25"), "& 和 % 要转义：{link}");
    assert!(!link.contains("&path="), "没有项目时不带 path");

    task.project = Some(project.clone());
    let link = new_thread_link(&task);
    let path = link.split("&path=").nth(1).expect("目录存在时带 path");
    assert!(!path.contains('/'), "路径里的斜杠要转义：{path}");
    assert!(path.contains("%2F") || path.contains("%5C"));

    task.project = Some(temp.path().join("不存在").display().to_string());
    assert!(!new_thread_link(&task).contains("&path="), "目录不存在时不带 path，免得 Codex 建出空项目");
}

#[test]
fn marker_is_the_first_well_formed_task_key() {
    assert_eq!(task_marker_id("[T-12] 修登录"), Some(12));
    assert_eq!(task_marker_id("# Files mentioned by the user:\n\n## a.txt\n\n[T-3] 接着做"), Some(3), "前面有附件说明也认");
    assert_eq!(task_marker_id("[T-] [T-01] [T-x] [T-7]"), Some(7), "跳过写坏的");
    assert_eq!(task_marker_id("T-12 没有方括号"), None);
    assert_eq!(task_marker_id(&format!("{}[T-5]", "字".repeat(5000))), None, "只看开头");
}

#[test]
fn rollout_timestamps_parse_as_utc_millis() {
    assert_eq!(parse_rollout_timestamp("2026-11-02T10:08:57.450Z"), Some(1_793_614_137_450));
    assert_eq!(parse_rollout_timestamp("2000-02-29T00:00:00Z"), Some(951_782_400_000));
    assert_eq!(parse_rollout_timestamp("2026-11-02T10:08:57.4Z"), Some(1_793_614_137_400));
    assert_eq!(parse_rollout_timestamp("2026-11-02 10:08:57"), None);
    assert_eq!(parse_rollout_timestamp("2026-13-02T10:08:57Z"), None);
}

fn event(timestamp: &str, payload: serde_json::Value) -> String {
    json!({ "timestamp": timestamp, "type": "event_msg", "payload": payload }).to_string()
}

#[test]
fn rollout_state_comes_from_the_last_turn_event() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    let started = event("2026-11-02T10:00:00.000Z", json!({ "type": "task_started", "turn_id": "a" }));
    let complete = event("2026-11-02T10:05:00.000Z", json!({ "type": "task_complete", "turn_id": "a", "last_agent_message": "  改好了，已提交。 " }));
    let message = json!({ "timestamp": "2026-11-02T10:05:01.000Z", "type": "response_item", "payload": { "type": "message", "text": "task_started" } }).to_string();

    assert_eq!(rollout_run_state(&temp.path().join("missing.jsonl")).0, RunState::Idle);
    std::fs::write(&path, format!("{started}\n")).unwrap();
    let (state, started_at, finished_at, _) = rollout_run_state(&path);
    assert_eq!((state, started_at, finished_at), (RunState::Running, parse_rollout_timestamp("2026-11-02T10:00:00.000Z"), None));

    std::fs::write(&path, format!("{started}\n{complete}\n{message}\n")).unwrap();
    let (state, started_at, finished_at, last) = rollout_run_state(&path);
    assert_eq!(state, RunState::Finished, "不是 event_msg 的行里出现 task_started 也不算");
    assert_eq!(started_at, parse_rollout_timestamp("2026-11-02T10:00:00.000Z"));
    assert_eq!(finished_at, parse_rollout_timestamp("2026-11-02T10:05:00.000Z"));
    assert_eq!(last.as_deref(), Some("改好了，已提交。"));

    let aborted = event("2026-11-02T10:07:00.000Z", json!({ "type": "turn_aborted", "reason": "interrupted" }));
    let again = event("2026-11-02T10:06:00.000Z", json!({ "type": "task_started", "turn_id": "b" }));
    std::fs::write(&path, format!("{started}\n{complete}\n{again}\n{aborted}\n")).unwrap();
    assert_eq!(rollout_run_state(&path).0, RunState::Aborted);

    // 只读末尾：很长的 rollout 前面截断的半行不影响。
    let padding = format!("{{\"timestamp\":\"x\",\"type\":\"response_item\",\"payload\":{{\"text\":\"{}\"}}}}", "x".repeat(300 * 1024));
    std::fs::write(&path, format!("{padding}\n{started}\n")).unwrap();
    assert_eq!(rollout_run_state(&path).0, RunState::Running);
}

fn write_threads(home: &Path, rows: &[(&str, &str, i64, &Path, bool)]) {
    std::fs::create_dir_all(home).unwrap();
    let connection = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT, updated_at INTEGER, archived INTEGER, first_user_message TEXT);",
        )
        .unwrap();
    for (id, message, updated_at, rollout, archived) in rows {
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, '', '/repo', ?3, ?4, ?5)",
                rusqlite::params![id, rollout.display().to_string(), updated_at, *archived as i64, message],
            )
            .unwrap();
    }
}

#[test]
fn linked_runs_reads_marked_threads_from_the_codex_database() {
    let _lock = crate::common::global_state_lock();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    assert!(linked_runs(&home).is_empty(), "没有数据库时为空");

    let rollout = temp.path().join("a.jsonl");
    std::fs::write(&rollout, format!("{}\n", event("2026-11-02T10:00:00.000Z", json!({ "type": "task_started" })))).unwrap();
    let missing = temp.path().join("missing.jsonl");
    write_threads(
        &home,
        &[
            ("a", "[T-2] 修登录\n\n细节", 1_793_614_137, &rollout, false),
            ("b", "随便聊聊", 1_793_614_138, &missing, false),
            ("c", "[T-2] 旧的那次", 1_793_614_000, &missing, true),
            ("d", "[T-5] 另一个", 1_793_614_100, &missing, false),
        ],
    );
    let runs = linked_runs(&home);
    let ids: Vec<_> = runs.iter().map(|run| (run.thread_id.as_str(), run.task_id)).collect();
    assert_eq!(ids, [("a", 2), ("d", 5)], "不带编号的、已归档的不算；新的在前");
    assert_eq!(runs[0].state, RunState::Running);
    assert_eq!(runs[0].title, "[T-2] 修登录", "没有标题时用首行");
    assert_eq!(runs[0].updated_at, 1_793_614_137_000, "秒转毫秒");
    assert_eq!(runs[1].state, RunState::Idle, "rollout 读不到时当作还没开始");
}

#[test]
fn scratch_dirs_are_not_offered_as_projects() {
    for path in [
        "/tmp",
        "/tmp/jj-exp",
        "/private/var/folders/x/T/abc",
        "/Users/me/Documents/Codex/2026-11-16/new-chat",
        "C:\\Users\\me\\Documents\\Codex\\2026-11-16\\1",
        "/Users/me/.codex/worktrees/ab12/repo",
    ] {
        assert!(is_scratch_dir(path), "{path}");
    }
    for path in ["/Users/me/code/CodexPlusPlus", "/Users/me/Codex/notes", "/Users/me/tmpfiles", "D:\\work\\repo"] {
        assert!(!is_scratch_dir(path), "{path}");
    }
}

#[test]
fn recent_projects_come_from_thread_working_dirs() {
    let _lock = crate::common::global_state_lock();
    // 不放系统临时目录：那里的目录会被当成临时目录过滤掉。
    let temp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let home = temp.path().join("codex-home");
    assert!(recent_projects(&home).is_empty(), "没有数据库时为空");

    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    let archived_only = temp.path().join("archived-only");
    for dir in [&repo_a, &repo_b, &archived_only] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::create_dir_all(&home).unwrap();
    let connection = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    connection
        .execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT, updated_at INTEGER, archived INTEGER);")
        .unwrap();
    let gone = temp.path().join("deleted-repo");
    let rows: [(&str, String, i64, i64); 7] = [
        ("1", format!("{}/", repo_b.display()), 50, 0),
        ("2", repo_a.display().to_string(), 40, 0),
        ("3", repo_b.display().to_string(), 30, 0),
        ("4", gone.display().to_string(), 60, 0),
        ("5", "/tmp".into(), 70, 0),
        ("6", archived_only.display().to_string(), 80, 1),
        ("7", String::new(), 90, 0),
    ];
    for (id, cwd, updated_at, archived) in rows {
        connection.execute("INSERT INTO threads VALUES (?1, '', ?2, ?3, ?4)", rusqlite::params![id, cwd, updated_at, archived]).unwrap();
    }
    drop(connection);
    assert_eq!(
        recent_projects(&home),
        [repo_b.display().to_string(), repo_a.display().to_string()],
        "最近的在前、去重（末尾斜杠不算不同）；不存在的、临时目录、已归档、空的都不算"
    );
}

#[test]
fn pick_project_writes_the_chosen_folder_and_cancel_changes_nothing() {
    let _lock = crate::common::global_state_lock();
    let temp = tempfile::tempdir().unwrap();
    set_tasks_dir_for_tests(Some(temp.path().join("tasks")));
    set_codex_home_for_tests(Some(temp.path().join("codex-home")));
    dispatch("create", &json!({ "task": { "title": "选项目" } })).unwrap();

    set_picked_folder_for_tests(Some(None));
    let cancelled = dispatch("pickProject", &json!({ "id": 1 })).unwrap();
    assert!(cancelled["picked"].is_null());
    assert!(cancelled["tasks"][0].get("project").is_none_or(|value| value.is_null()), "取消时不改");

    set_picked_folder_for_tests(Some(Some("/Users/me/code/repo".into())));
    let picked = dispatch("pickProject", &json!({ "id": 1 })).unwrap();
    assert_eq!(picked["picked"], "/Users/me/code/repo");
    assert_eq!(picked["tasks"][0]["project"], "/Users/me/code/repo");
    assert!(picked["projects"].is_array(), "面板数据里带最近项目");
    assert!(dispatch("pickProject", &json!({ "id": 9 })).is_err(), "任务不存在");

    set_picked_folder_for_tests(None);
    set_tasks_dir_for_tests(None);
    set_codex_home_for_tests(None);
}

#[test]
fn auto_status_only_follows_events_after_the_last_manual_move() {
    let todo = sample_task(1, TaskStatus::Todo, 1_000);
    assert_eq!(auto_status(&todo, &[]), None);
    assert_eq!(auto_status(&todo, &[run(1, RunState::Running, Some(2_000), None)]), Some(TaskStatus::InProgress));
    assert_eq!(auto_status(&todo, &[run(1, RunState::Running, Some(500), None)]), None, "进入待办之前就在跑的不算");
    assert_eq!(auto_status(&todo, &[run(2, RunState::Running, Some(2_000), None)]), None, "别的任务的对话");
    assert_eq!(auto_status(&todo, &[run(1, RunState::Finished, Some(1_500), Some(2_000))]), Some(TaskStatus::InReview), "面板没开时已经跑完");

    let doing = sample_task(1, TaskStatus::InProgress, 1_000);
    assert_eq!(auto_status(&doing, &[run(1, RunState::Finished, Some(1_500), Some(2_000))]), Some(TaskStatus::InReview));
    assert_eq!(
        auto_status(&doing, &[run(1, RunState::Finished, Some(1_500), Some(2_000)), run(1, RunState::Running, Some(2_500), None)]),
        None,
        "还有对话在跑就不进待验收"
    );
    assert_eq!(auto_status(&doing, &[run(1, RunState::Finished, Some(100), Some(500))]), None, "手动挪回进行中之后不被旧结果拽走");
    assert_eq!(auto_status(&doing, &[run(1, RunState::Aborted, Some(1_500), Some(2_000))]), None, "中断不算做完");

    let review = sample_task(1, TaskStatus::InReview, 1_000);
    assert_eq!(auto_status(&review, &[run(1, RunState::Running, Some(2_000), None)]), Some(TaskStatus::InProgress), "验收时又接着让它改");
    for status in [TaskStatus::Blocked, TaskStatus::Done, TaskStatus::Cancelled] {
        let task = sample_task(1, status, 1_000);
        assert_eq!(auto_status(&task, &[run(1, RunState::Running, Some(2_000), None)]), None, "{status:?} 不自动动");
        assert_eq!(auto_status(&task, &[run(1, RunState::Finished, Some(1_500), Some(2_000))]), None, "{status:?} 不自动动");
    }
}

#[test]
fn apply_auto_status_writes_the_new_status_once() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let task = create_task(dir, NewTask { title: "交给 Codex".into(), status: Some(TaskStatus::Todo), ..Default::default() }).unwrap();
    let runs = [run(task.id, RunState::Running, Some(task.status_at + 1), None)];
    assert_eq!(apply_auto_status(dir, &list_tasks(dir), &runs), 1);
    let moved = read_task(dir, task.id).unwrap();
    assert_eq!(moved.status, TaskStatus::InProgress);
    assert_eq!(apply_auto_status(dir, &list_tasks(dir), &runs), 0, "状态时间更新了，同一个事件不会再推一次");
}

#[test]
fn start_opens_a_new_thread_link_and_the_board_reports_linked_runs() {
    let _lock = crate::common::global_state_lock();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    set_tasks_dir_for_tests(Some(temp.path().join("tasks")));
    set_codex_home_for_tests(Some(home.clone()));

    dispatch("create", &json!({ "task": { "title": "交给 Codex", "status": "todo" } })).unwrap();
    let started = dispatch("start", &json!({ "id": 1 })).unwrap();
    assert_eq!(started["tasks"][0]["status"], "todo", "只是打开新对话，用户发出去之前不改状态");
    let links = take_opened_links_for_tests();
    assert_eq!(links.len(), 1);
    assert!(links[0].starts_with("codex://threads/new?prompt=%5BT-1%5D%20"), "{}", links[0]);
    assert!(dispatch("start", &json!({ "id": 9 })).is_err());
    assert!(take_opened_links_for_tests().is_empty(), "任务不存在时不打开");

    // 用户在 Codex 里发出去了：对话开始跑，下次刷新任务进「进行中」，并带回这条对话。
    let rollout = temp.path().join("a.jsonl");
    std::fs::write(&rollout, format!("{}\n", event("2099-01-01T00:00:00.000Z", json!({ "type": "task_started" })))).unwrap();
    let missing = temp.path().join("missing.jsonl");
    write_threads(&home, &[("thread-a", "[T-1] 交给 Codex", 1_793_614_137, &rollout, false), ("thread-x", "[T-42] 别的", 1, &missing, false)]);
    let board = dispatch("list", &json!({})).unwrap();
    assert_eq!(board["tasks"][0]["status"], "in_progress");
    let runs = board["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "认不出任务的对话不回传");
    assert_eq!(runs[0]["threadId"], "thread-a");
    assert_eq!(runs[0]["taskId"], 1);
    assert_eq!(runs[0]["state"], "running");

    set_tasks_dir_for_tests(None);
    set_codex_home_for_tests(None);
}
