//! 会话目录重定位（#1085）：目录重映射和批量移动要把会话库、侧边栏目录库、rollout、全局状态一起改到新目录。

use codex_plus_data::{SessionRelocationRequest, apply_session_relocation, plan_session_relocation};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tempfile::{TempDir, tempdir};

struct Fixture {
    temp: TempDir,
}

impl Fixture {
    /// 一个最小的 Codex 目录：会话库（含项目根表）、侧边栏目录库。
    fn new() -> Self {
        let temp = tempdir().unwrap();
        let fixture = Self { temp };
        fs::create_dir_all(fixture.home().join("sqlite")).unwrap();
        let state = Connection::open(fixture.state_db()).unwrap();
        state
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, title TEXT NOT NULL, cwd TEXT NOT NULL, archived INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE project_roots (project_id TEXT NOT NULL, position INTEGER NOT NULL, path TEXT NOT NULL, PRIMARY KEY (project_id, position));",
            )
            .unwrap();
        let catalog = Connection::open(fixture.catalog_db()).unwrap();
        catalog
            .execute_batch(
                "CREATE TABLE local_thread_catalog (host_id TEXT NOT NULL, thread_id TEXT NOT NULL, cwd TEXT, PRIMARY KEY (host_id, thread_id));
                 CREATE TABLE local_thread_catalog_metadata (id INTEGER PRIMARY KEY CHECK (id = 1), catalog_revision INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO local_thread_catalog_metadata (id, catalog_revision) VALUES (1, 7);",
            )
            .unwrap();
        fixture
    }

    fn home(&self) -> &Path {
        self.temp.path()
    }

    fn state_db(&self) -> PathBuf {
        self.home().join("state_5.sqlite")
    }

    fn catalog_db(&self) -> PathBuf {
        self.home().join("sqlite").join("codex.db")
    }

    fn backups(&self) -> PathBuf {
        self.home().join("backups")
    }

    fn rollout(&self, id: &str) -> PathBuf {
        self.home().join("sessions").join(format!("rollout-{id}.jsonl"))
    }

    /// 登记一个会话：会话库一行、目录库一行，`rollout` 为 Some 时写 rollout 文件。
    fn thread(&self, id: &str, cwd: &str, archived: bool, rollout: Option<&[Value]>) {
        let path = self.rollout(id);
        if let Some(records) = rollout {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let text: String = records.iter().map(|record| format!("{record}\n")).collect();
            fs::write(&path, text).unwrap();
        }
        Connection::open(self.state_db())
            .unwrap()
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd, archived) VALUES (?1, ?2, ?3, ?4, ?5)",
                (id, path.to_string_lossy().as_ref(), format!("Thread {id}"), cwd, archived as i64),
            )
            .unwrap();
        Connection::open(self.catalog_db())
            .unwrap()
            .execute(
                "INSERT INTO local_thread_catalog (host_id, thread_id, cwd) VALUES ('local', ?1, ?2)",
                (id, cwd),
            )
            .unwrap();
    }

    fn cwd(&self, id: &str) -> String {
        Connection::open(self.state_db())
            .unwrap()
            .query_row("SELECT cwd FROM threads WHERE id = ?1", [id], |row| row.get(0))
            .unwrap()
    }

    fn catalog_cwd(&self, id: &str) -> String {
        Connection::open(self.catalog_db())
            .unwrap()
            .query_row("SELECT cwd FROM local_thread_catalog WHERE thread_id = ?1", [id], |row| row.get(0))
            .unwrap()
    }

    fn catalog_revision(&self) -> i64 {
        Connection::open(self.catalog_db())
            .unwrap()
            .query_row("SELECT catalog_revision FROM local_thread_catalog_metadata", [], |row| row.get(0))
            .unwrap()
    }

    fn write_state(&self, state: Value) {
        fs::write(self.home().join(".codex-global-state.json"), serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    }

    fn state(&self) -> Value {
        serde_json::from_slice(&fs::read(self.home().join(".codex-global-state.json")).unwrap()).unwrap()
    }

    fn rollout_records(&self, id: &str) -> Vec<Value> {
        fs::read_to_string(self.rollout(id))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// 一份 rollout：会话元数据、一轮的上下文（带沙箱可写目录）、一条提到旧目录的对话正文。
fn rollout_records(cwd: &str, turn_cwd: &str) -> Vec<Value> {
    vec![
        json!({ "timestamp": "2026-01-01T00:00:00Z", "type": "session_meta", "payload": {
            "id": "t", "cwd": cwd, "runtime_workspace_roots": [cwd], "model_provider": "openai",
            "base_instructions": { "text": format!("{cwd} is mentioned inside a sentence") },
        } }),
        json!({ "timestamp": "2026-01-01T00:00:01Z", "type": "turn_context", "payload": {
            "cwd": turn_cwd, "model": "gpt",
            "file_system_sandbox_policy": { "entries": [
                { "path": { "type": "path", "path": cwd }, "access": "write" },
                { "path": { "type": "path", "path": "/unrelated/cache" }, "access": "write" },
            ] },
        } }),
        json!({ "timestamp": "2026-01-01T00:00:02Z", "type": "response_item", "payload": {
            "type": "message", "content": format!("ran ls in {cwd}"), "note": "\"session_meta\" \"turn_context\"", "cwd": cwd,
        } }),
    ]
}

fn remap(from: &str, to: &str) -> SessionRelocationRequest {
    SessionRelocationRequest::Remap { from: from.into(), to: to.into() }
}

fn move_to(ids: &[&str], to: &str) -> SessionRelocationRequest {
    SessionRelocationRequest::Move { thread_ids: ids.iter().map(|id| id.to_string()).collect(), to: to.into() }
}

#[test]
fn remap_updates_index_catalog_rollout_and_project_roots_together() {
    let fixture = Fixture::new();
    fixture.thread("t1", "/old/proj", false, Some(&rollout_records("/old/proj", "/old/proj/pkg")));
    fixture.thread("t2", "/old/proj/sub", true, Some(&rollout_records("/old/proj/sub", "/old/proj/sub")));
    fixture.thread("t3", "/old/project-other", false, Some(&rollout_records("/old/project-other", "/old/project-other")));
    fixture.thread("t4", "/elsewhere", false, None);
    Connection::open(fixture.state_db())
        .unwrap()
        .execute_batch("INSERT INTO project_roots VALUES ('p1', 0, '/old/proj'), ('p2', 0, '/elsewhere');")
        .unwrap();
    fixture.write_state(json!({
        "local-projects": {
            "p1": { "id": "p1", "name": "Proj", "rootPaths": ["/old/proj"] },
            "p2": { "id": "p2", "name": "Else", "rootPaths": ["/elsewhere"] },
        },
        "electron-saved-workspace-roots": ["/old/proj", "/elsewhere"],
        "active-workspace-roots": "/old/proj",
        "project-order": ["p2", "p1"],
        "electron-workspace-root-labels": { "/old/proj": "My project", "/elsewhere": "Else" },
        "electron-owner-value:thread-workspace-root-hints:signed-out": { "t1": "/old/proj", "t4": "/elsewhere" },
        "thread-project-assignments": { "t1": { "projectKind": "local", "projectId": "p1" } },
        "unrelated": { "path": "/old/proj" },
    }));
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    fs::File::options()
        .write(true)
        .open(fixture.rollout("t1"))
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let untouched_line = fs::read_to_string(fixture.rollout("t1")).unwrap().lines().nth(2).unwrap().to_string();

    let request = remap("/old/proj/", "/new/place");
    let plan = plan_session_relocation(fixture.home(), &request).unwrap();
    let mut planned: Vec<_> = plan.items.iter().map(|item| (item.thread_id.as_str(), item.old_cwd.as_str(), item.new_cwd.as_str(), item.archived)).collect();
    planned.sort();
    assert_eq!(planned, vec![("t1", "/old/proj", "/new/place", false), ("t2", "/old/proj/sub", "/new/place/sub", true)]);
    assert_eq!(plan.project_roots.len(), 1, "同一个根在全局状态和会话库里各有一份，预览只列一次");
    assert_eq!((plan.project_roots[0].old_path.as_str(), plan.project_roots[0].new_path.as_str()), ("/old/proj", "/new/place"));
    assert_eq!(fixture.cwd("t1"), "/old/proj", "预览不改任何东西");

    let report = apply_session_relocation(fixture.home(), &request, &fixture.backups()).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!((report.moved, report.rollouts_rewritten, report.rollouts_missing), (2, 2, 0));

    // 会话库、目录库。
    assert_eq!(fixture.cwd("t1"), "/new/place");
    assert_eq!(fixture.cwd("t2"), "/new/place/sub");
    assert_eq!(fixture.cwd("t3"), "/old/project-other", "只是名字前缀相同的目录不动");
    assert_eq!(fixture.cwd("t4"), "/elsewhere");
    assert_eq!(fixture.catalog_cwd("t1"), "/new/place");
    assert_eq!(fixture.catalog_cwd("t2"), "/new/place/sub");
    assert_eq!(fixture.catalog_cwd("t3"), "/old/project-other");
    assert_eq!(fixture.catalog_revision(), 9, "目录版本号按改动行数推进");

    // rollout：元数据和每轮上下文里的路径都换了，对话正文原样保留，修改时间不变。
    let records = fixture.rollout_records("t1");
    assert_eq!(records[0]["payload"]["cwd"], "/new/place");
    assert_eq!(records[0]["payload"]["runtime_workspace_roots"], json!(["/new/place"]));
    assert_eq!(records[0]["payload"]["base_instructions"]["text"], "/old/proj is mentioned inside a sentence");
    assert_eq!(records[0]["payload"]["model_provider"], "openai");
    assert_eq!(records[1]["payload"]["cwd"], "/new/place/pkg");
    assert_eq!(records[1]["payload"]["file_system_sandbox_policy"]["entries"][0]["path"]["path"], "/new/place");
    assert_eq!(records[1]["payload"]["file_system_sandbox_policy"]["entries"][1]["path"]["path"], "/unrelated/cache");
    let text = fs::read_to_string(fixture.rollout("t1")).unwrap();
    assert_eq!(text.lines().nth(2).unwrap(), untouched_line);
    assert!(text.ends_with('\n'));
    assert_eq!(fs::metadata(fixture.rollout("t1")).unwrap().modified().unwrap(), modified);
    assert_eq!(fixture.rollout_records("t3")[0]["payload"]["cwd"], "/old/project-other");

    // 项目根：会话库的表和全局状态里的各种写法。
    let roots: Vec<String> = Connection::open(fixture.state_db())
        .unwrap()
        .prepare("SELECT path FROM project_roots ORDER BY project_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(roots, vec!["/new/place", "/elsewhere"]);
    let state = fixture.state();
    assert_eq!(state["local-projects"]["p1"]["rootPaths"], json!(["/new/place"]));
    assert_eq!(state["local-projects"]["p2"]["rootPaths"], json!(["/elsewhere"]));
    assert_eq!(state["electron-saved-workspace-roots"], json!(["/new/place", "/elsewhere"]));
    assert_eq!(state["active-workspace-roots"], "/new/place");
    assert_eq!(state["project-order"], json!(["p2", "p1"]));
    assert_eq!(state["electron-workspace-root-labels"], json!({ "/elsewhere": "Else", "/new/place": "My project" }));
    assert_eq!(state["electron-owner-value:thread-workspace-root-hints:signed-out"], json!({ "t1": "/new/place", "t4": "/elsewhere" }));
    assert_eq!(state["thread-project-assignments"]["t1"]["projectId"], "p1", "项目归属按 id，不用动");
    assert_eq!(state["unrelated"]["path"], "/old/proj", "不认识的键不碰");
    assert_eq!(
        fs::read(fixture.home().join(".codex-global-state.json.bak")).unwrap(),
        fs::read(fixture.home().join(".codex-global-state.json")).unwrap()
    );

    // 备份：清单、全局状态原文、每个 rollout 改动前的原行。
    let backup = PathBuf::from(report.backup_dir.clone().unwrap());
    let manifest: Value = serde_json::from_slice(&fs::read(backup.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["request"], json!({ "kind": "remap", "from": "/old/proj/", "to": "/new/place" }));
    assert_eq!(manifest["items"].as_array().unwrap().len(), 2);
    let saved_state: Value = serde_json::from_slice(&fs::read(backup.join("codex-global-state.json")).unwrap()).unwrap();
    assert_eq!(saved_state["local-projects"]["p1"]["rootPaths"], json!(["/old/proj"]));
    let originals: Vec<Value> = fs::read_to_string(backup.join("rollouts").join("t1.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(originals.iter().map(|entry| entry["line"].as_u64().unwrap()).collect::<Vec<_>>(), vec![1, 2]);
    let original: Value = serde_json::from_str(originals[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(original["payload"]["cwd"], "/old/proj");

    // 同一个请求再跑一次：已经搬完，什么都不改，也不再建备份。
    let again = apply_session_relocation(fixture.home(), &request, &fixture.backups()).unwrap();
    assert_eq!((again.moved, again.rollouts_rewritten, again.project_roots_updated), (0, 0, 0));
    assert!(again.backup_dir.is_none() && again.failures.is_empty());
}

#[test]
fn moving_selected_sessions_reassigns_them_to_the_project_at_the_target() {
    let fixture = Fixture::new();
    fixture.thread("t1", "/old/proj", false, Some(&rollout_records("/old/proj", "/old/proj/pkg")));
    fixture.thread("t2", "/old/proj", false, Some(&rollout_records("/old/proj", "/old/proj")));
    fixture.thread("t3", "/chats/2026/hello", false, None);
    fixture.thread("t4", "/new/place", false, None);
    Connection::open(fixture.state_db())
        .unwrap()
        .execute("INSERT INTO threads (id, rollout_path, title, cwd) VALUES ('t5', '', '', '')", [])
        .unwrap();
    fixture.write_state(json!({
        "local-projects": {
            "old": { "id": "old", "name": "Old", "rootPaths": ["/old/proj"] },
            "new": { "id": "new", "name": "New home", "rootPaths": ["/new/place/"] },
        },
        "thread-project-assignments": {
            "t1": { "projectKind": "local", "projectId": "old" },
            "t2": { "projectKind": "local", "projectId": "old" },
        },
        "thread-project-membership-host-ids": { "t1": "local", "t2": "local" },
        "projectless-thread-ids": ["t3", "t9"],
        "electron-owner-value:thread-workspace-root-hints:signed-out": { "t3": "/chats", "t2": "/old/proj" },
    }));

    // 目标目录带着末尾斜杠传进来，写进各处时去掉。
    let request = move_to(&["t1", "local:t3", "t4", "t5", "missing", "t1"], "/new/place/");
    let plan = plan_session_relocation(fixture.home(), &request).unwrap();
    assert_eq!(plan.items.iter().map(|item| item.thread_id.as_str()).collect::<Vec<_>>(), vec!["t1", "t3"]);
    assert!(plan.items.iter().all(|item| item.new_cwd == "/new/place"));
    assert_eq!(plan.target_project.as_deref(), Some("New home"));
    assert_eq!(plan.skipped.len(), 3, "{:?}", plan.skipped);
    assert!(plan.skipped.iter().any(|reason| reason.contains("已经在目标目录")));
    assert!(plan.skipped.iter().any(|reason| reason.contains("没有记录工作目录")));
    assert!(plan.skipped.iter().any(|reason| reason.starts_with("missing：")));
    assert!(plan.project_roots.is_empty());

    let report = apply_session_relocation(fixture.home(), &request, &fixture.backups()).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!((report.moved, report.rollouts_rewritten, report.rollouts_missing), (2, 1, 1));
    assert_eq!(report.skipped.len(), 3);
    assert_eq!(fixture.cwd("t1"), "/new/place");
    assert_eq!(fixture.cwd("t2"), "/old/proj", "没选中的会话留在原目录");
    assert_eq!(fixture.cwd("t3"), "/new/place");
    assert_eq!(fixture.catalog_cwd("t3"), "/new/place");
    let records = fixture.rollout_records("t1");
    assert_eq!(records[0]["payload"]["cwd"], "/new/place");
    assert_eq!(records[1]["payload"]["cwd"], "/new/place/pkg", "会话里切过的子目录跟着换");
    assert_eq!(fixture.rollout_records("t2")[0]["payload"]["cwd"], "/old/proj");

    let state = fixture.state();
    let assignment = json!({ "projectKind": "local", "projectId": "new" });
    assert_eq!(state["thread-project-assignments"]["t1"], assignment);
    assert_eq!(state["thread-project-assignments"]["t3"], assignment);
    assert_eq!(state["thread-project-assignments"]["t2"]["projectId"], "old");
    assert_eq!(state["thread-project-membership-host-ids"]["t3"], "local");
    assert_eq!(state["projectless-thread-ids"], json!(["t9"]));
    assert_eq!(
        state["electron-owner-value:thread-workspace-root-hints:signed-out"],
        json!({ "t3": "/new/place", "t2": "/old/proj" })
    );
    assert_eq!(state["local-projects"]["old"]["rootPaths"], json!(["/old/proj"]), "批量移动不改项目根");
}

#[test]
fn moving_to_a_folder_without_a_project_only_changes_the_working_directory() {
    let fixture = Fixture::new();
    fixture.thread("t1", "/old/proj", false, None);
    let original = json!({
        "local-projects": { "old": { "id": "old", "name": "Old", "rootPaths": ["/old/proj"] } },
        "thread-project-assignments": { "t1": { "projectKind": "local", "projectId": "old" } },
    });
    fixture.write_state(original.clone());
    let request = move_to(&["t1"], "/somewhere/else");
    assert_eq!(plan_session_relocation(fixture.home(), &request).unwrap().target_project, None);
    let report = apply_session_relocation(fixture.home(), &request, &fixture.backups()).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(fixture.cwd("t1"), "/somewhere/else");
    assert_eq!(fixture.state(), original, "没有对应项目时全局状态不动");
}

#[test]
fn works_without_global_state_or_catalog_and_reports_nothing_to_do() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.catalog_db()).unwrap();
    fixture.thread_without_catalog("t1", "/old/proj");
    let report = apply_session_relocation(fixture.home(), &remap("/old/proj", "/new/place"), &fixture.backups()).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!((report.moved, report.rollouts_missing), (1, 1));
    assert_eq!(fixture.cwd("t1"), "/new/place");
    assert!(!fixture.home().join(".codex-global-state.json").exists(), "原来没有全局状态文件就不新建");

    let nothing = apply_session_relocation(fixture.home(), &remap("/absent", "/new/other"), &fixture.backups()).unwrap();
    assert_eq!(nothing.moved, 0);
    assert!(nothing.backup_dir.is_none());
}

impl Fixture {
    fn thread_without_catalog(&self, id: &str, cwd: &str) {
        Connection::open(self.state_db())
            .unwrap()
            .execute(
                "INSERT INTO threads (id, rollout_path, title, cwd) VALUES (?1, ?2, ?3, ?4)",
                (id, self.rollout(id).to_string_lossy().as_ref(), format!("Thread {id}"), cwd),
            )
            .unwrap();
    }
}

#[test]
fn rejects_directories_that_cannot_be_relocated() {
    let fixture = Fixture::new();
    fixture.thread("t1", "/old/proj", false, None);
    for (from, to) in [
        ("/old/proj", "/old/proj/"),
        ("/old/proj", "/old/proj/inner"),
        ("/old/proj/inner", "/old/proj"),
        ("", "/new/place"),
        ("/old/proj", "  "),
        ("/", "/new/place"),
        ("/old/proj", "/"),
        ("C:\\", "D:\\new"),
    ] {
        assert!(plan_session_relocation(fixture.home(), &remap(from, to)).is_err(), "{from} -> {to}");
        assert!(apply_session_relocation(fixture.home(), &remap(from, to), &fixture.backups()).is_err());
    }
    assert!(plan_session_relocation(fixture.home(), &move_to(&["t1"], "")).is_err());
    assert_eq!(fixture.cwd("t1"), "/old/proj");
    assert!(!fixture.backups().exists(), "参数不对时什么都不写");
}

#[test]
fn windows_paths_match_ignoring_case_separators_and_long_path_prefix() {
    let fixture = Fixture::new();
    fixture.thread("t1", r"c:\Old\Proj", false, None);
    fixture.thread("t2", r"\\?\C:\old\proj\Sub Dir", false, None);
    fixture.thread("t3", r"C:\Old\Project2", false, None);
    fixture.thread("t4", "C:/old/proj/web", false, None);
    let plan = plan_session_relocation(fixture.home(), &remap(r"C:\Old\Proj", r"D:\Work\Proj\")).unwrap();
    let mut planned: Vec<_> = plan.items.iter().map(|item| (item.thread_id.as_str(), item.new_cwd.as_str())).collect();
    planned.sort();
    assert_eq!(planned, vec![("t1", r"D:\Work\Proj"), ("t2", r"D:\Work\Proj\Sub Dir"), ("t4", r"D:\Work\Proj\web")]);

    // Unix 路径区分大小写。
    let unix = Fixture::new();
    unix.thread("t1", "/Old/Proj", false, None);
    assert!(plan_session_relocation(unix.home(), &remap("/old/proj", "/new")).unwrap().items.is_empty());
}

#[test]
fn a_rollout_that_cannot_be_rewritten_leaves_that_session_untouched() {
    let fixture = Fixture::new();
    fixture.thread("t1", "/old/proj", false, Some(&rollout_records("/old/proj", "/old/proj")));
    fixture.thread("t2", "/old/proj", false, Some(&rollout_records("/old/proj", "/old/proj")));
    // t1 的 rollout 不是合法的 UTF-8，读不了。
    fs::write(fixture.rollout("t1"), [0xff, 0xfe, b'\n']).unwrap();
    let report = apply_session_relocation(fixture.home(), &remap("/old/proj", "/new/place"), &fixture.backups()).unwrap();
    assert_eq!(report.moved, 1);
    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    assert!(report.failures[0].starts_with("Thread t1："));
    assert_eq!(fixture.cwd("t1"), "/old/proj", "rollout 没改成，索引也不改，两边保持一致");
    assert_eq!(fixture.catalog_cwd("t1"), "/old/proj");
    assert_eq!(fixture.cwd("t2"), "/new/place");
}
