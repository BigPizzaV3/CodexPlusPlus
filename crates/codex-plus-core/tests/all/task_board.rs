use codex_plus_core::task_board::{
    bridge_route, create_task, delete_task, dispatch, list_tasks, parse_task, read_task, render_task, set_tasks_dir_for_tests, update_task,
    NewTask, TaskPatch, TaskStatus,
};
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
}
