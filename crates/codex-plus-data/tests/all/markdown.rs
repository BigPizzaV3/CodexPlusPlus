use codex_plus_core::models::{ExportStatus, SessionRef};
use codex_plus_data::MarkdownExportService;
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn session(id: &str, title: &str) -> SessionRef {
    SessionRef::new(id, title).unwrap()
}

fn create_codex_thread_db(path: &Path, rollout_path: &Path, thread_id: &str, title: &str) {
    let db = Connection::open(path).unwrap();
    db.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, archived INTEGER, archived_at INTEGER)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads (id, rollout_path, title, archived, archived_at) VALUES (?1, ?2, ?3, 0, NULL)",
        (thread_id, rollout_path.to_string_lossy().to_string(), title),
    )
    .unwrap();
}

#[test]
fn markdown_exporter_exports_messages_images_and_sanitized_filename() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    let rollout_path = tmp.path().join("rollout.jsonl");
    fs::write(
        &rollout_path,
        concat!(
            "{\"type\":\"session_meta\",\"timestamp\":\"2026-05-10T13:00:00Z\"}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-05-10T13:12:06Z\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Hello\"},{\"type\":\"input_image\",\"image_url\":\"data:image/png;base64,AAAA\"},{\"type\":\"input_image\",\"image_url\":\"https://example.com/image.png\"}]}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-05-10T13:12:07Z\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",\"content\":[{\"type\":\"input_text\",\"text\":\"ignore\"}]}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"not-a-timestamp\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hi there\"}]}}\n"
        ),
    )
    .unwrap();
    create_codex_thread_db(&db_path, &rollout_path, "thread:1", "Bad<>:\"/\\|?* Title");

    let result =
        MarkdownExportService::new(Some(&db_path)).export(&session("local:thread:1", "Ignored"));

    assert_eq!(result.status, ExportStatus::Exported);
    assert_eq!(result.session_id, "thread:1");
    assert_eq!(result.filename, Some("Bad Title-thread-1.md".to_string()));
    let markdown = result.markdown.unwrap();
    assert!(markdown.starts_with("# Bad<>:\"/\\|?* Title\n\n### User\n"));
    assert!(markdown.contains("Hello"));
    assert_eq!(markdown.matches("> Image attachment").count(), 2);
    assert!(markdown.contains("[Image link](<https://example.com/image.png>)"));
    assert!(!markdown.contains("data:image/png;base64"));
    assert!(markdown.contains("### Assistant\n\nHi there\n"));
    assert!(!markdown.contains("ignore"));
}

#[test]
fn markdown_exporter_exports_automation_run_by_discovering_rollout_file() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join(".codex").join("db").join("codex.sqlite");
    let sessions_dir = tmp.path().join(".codex").join("sessions").join("2026");
    fs::create_dir_all(&sessions_dir).unwrap();
    let rollout_path = sessions_dir.join("rollout-thread-123.jsonl");
    fs::write(
        &rollout_path,
        concat!(
            "{\"type\":\"session_meta\",\"timestamp\":\"2026-05-10T13:00:00Z\",\"payload\":{\"id\":\"thread-123\",\"title\":\"Meta Title\"}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-05-10T13:12:06Z\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Hello from automation\"}]}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-05-10T13:12:07Z\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Automation reply\"}]}}\n",
        ),
    )
    .unwrap();
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let db = Connection::open(&db_path).unwrap();
    db.execute(
        "CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY, thread_title TEXT)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO automation_runs (thread_id, thread_title) VALUES ('thread-123', 'Automation Title')",
        [],
    )
    .unwrap();

    let result =
        MarkdownExportService::new(Some(&db_path)).export(&session("thread-123", "Ignored"));

    assert_eq!(result.status, ExportStatus::Exported);
    assert_eq!(result.session_id, "thread-123");
    assert_eq!(
        result.filename,
        Some("Automation Title-thread-123.md".to_string())
    );
    let markdown = result.markdown.unwrap();
    assert!(markdown.starts_with("# Automation Title\n\n### User\n"));
    assert!(markdown.contains("Hello from automation"));
    assert!(markdown.contains("### Assistant\n"));
    assert!(markdown.contains("Automation reply"));
}

#[test]
fn markdown_exporter_searches_candidate_databases() {
    let tmp = tempdir().unwrap();
    let codex_home = tmp.path().join(".codex");
    let first_db_path = codex_home.join("sqlite").join("codex-dev.db");
    let second_db_path = codex_home.join("sqlite").join("state.db");
    let sessions_dir = codex_home.join("sessions").join("2026");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::create_dir_all(first_db_path.parent().unwrap()).unwrap();

    let first_db = Connection::open(&first_db_path).unwrap();
    first_db
        .execute(
            "CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY, thread_title TEXT)",
            [],
        )
        .unwrap();
    first_db
        .execute(
            "INSERT INTO automation_runs (thread_id, thread_title) VALUES ('other-thread', 'Other')",
            [],
        )
        .unwrap();

    let second_db = Connection::open(&second_db_path).unwrap();
    second_db
        .execute(
            "CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY, thread_title TEXT)",
            [],
        )
        .unwrap();
    second_db
        .execute(
            "INSERT INTO automation_runs (thread_id, thread_title) VALUES ('thread-456', 'Second DB')",
            [],
        )
        .unwrap();

    fs::write(
        sessions_dir.join("rollout-thread-456.jsonl"),
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-456\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"From second db\"}]}}\n",
        ),
    )
    .unwrap();

    let result = codex_plus_data::export_markdown_from_paths(
        [first_db_path, second_db_path],
        &session("thread-456", "Ignored"),
    );

    assert_eq!(result.status, ExportStatus::Exported);
    assert_eq!(result.filename, Some("Second DB-thread-456.md".to_string()));
    assert!(result.markdown.unwrap().contains("From second db"));
}

#[test]
fn markdown_exporter_returns_failed_for_missing_or_empty_rollout() {
    let tmp = tempdir().unwrap();
    let missing_db = tmp.path().join("missing.sqlite");
    let missing_rollout = tmp.path().join("missing.jsonl");
    create_codex_thread_db(&missing_db, &missing_rollout, "t1", "Codex Thread");

    let result = MarkdownExportService::new(None::<&Path>).export(&session("t1", "Codex Thread"));
    assert_eq!(result.status, ExportStatus::Failed);

    let result =
        MarkdownExportService::new(Some(&missing_db)).export(&session("t1", "Codex Thread"));
    assert_eq!(result.status, ExportStatus::Failed);

    let empty_db = tmp.path().join("empty.sqlite");
    let empty_rollout = tmp.path().join("empty.jsonl");
    fs::write(
        &empty_rollout,
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",\"content\":[{\"type\":\"input_text\",\"text\":\"ignore\"}]}}\n",
    )
    .unwrap();
    create_codex_thread_db(&empty_db, &empty_rollout, "t1", "Codex Thread");

    let result = MarkdownExportService::new(Some(&empty_db)).export(&session("t1", "Codex Thread"));

    assert_eq!(result.status, ExportStatus::Failed);
}

fn write_codex_rollout(path: &Path, thread_id: &str, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            concat!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\"}}}}\n",
                "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"{text}\"}}]}}}}\n",
            ),
            id = thread_id,
            text = text,
        ),
    )
    .unwrap();
}

// #167：threads 表存在但没有该会话的行，rollout 仍在 sessions/ 下时应照常导出。
#[test]
fn markdown_exporter_falls_back_to_sessions_dir_when_threads_row_missing() {
    let tmp = tempdir().unwrap();
    let codex_home = tmp.path().join(".codex");
    let db_path = codex_home.join("state_5.sqlite");
    let thread_id = "019e1f2a-aaaa-7bbb-8ccc-0123456789ab";
    fs::create_dir_all(&codex_home).unwrap();
    create_codex_thread_db(
        &db_path,
        &codex_home.join("sessions").join("other.jsonl"),
        "other-thread",
        "Other",
    );
    write_codex_rollout(
        &codex_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("19")
            .join(format!("rollout-2026-05-19T06-00-00-{thread_id}.jsonl")),
        thread_id,
        "Hello without threads row",
    );

    let result = MarkdownExportService::new(Some(&db_path))
        .export(&session(&format!("local:{thread_id}"), "Sidebar Title"));

    assert_eq!(result.status, ExportStatus::Exported, "{}", result.message);
    assert_eq!(result.session_id, thread_id);
    assert_eq!(
        result.filename,
        Some(format!("Sidebar Title-{thread_id}.md"))
    );
    assert!(
        result
            .markdown
            .unwrap()
            .contains("Hello without threads row")
    );
}

// #167：threads 行记录的 rollout_path 已失效，实际文件被移到 archived_sessions/ 时应能找到。
#[test]
fn markdown_exporter_recovers_stale_rollout_path_from_archived_sessions() {
    let tmp = tempdir().unwrap();
    let codex_home = tmp.path().join(".codex");
    let db_path = codex_home.join("state_5.sqlite");
    let thread_id = "019e1f2a-dddd-7eee-8fff-0123456789ab";
    let file_name = format!("rollout-2026-05-19T06-00-00-{thread_id}.jsonl");
    fs::create_dir_all(&codex_home).unwrap();
    create_codex_thread_db(
        &db_path,
        &codex_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("19")
            .join(&file_name),
        thread_id,
        "Archived Title",
    );
    write_codex_rollout(
        &codex_home.join("archived_sessions").join(&file_name),
        thread_id,
        "Hello from archive",
    );

    let result = MarkdownExportService::new(Some(&db_path)).export(&session(thread_id, "Ignored"));

    assert_eq!(result.status, ExportStatus::Exported, "{}", result.message);
    assert_eq!(
        result.filename,
        Some(format!("Archived Title-{thread_id}.md"))
    );
    assert!(result.markdown.unwrap().contains("Hello from archive"));
}

// 两处都找不到时仍报「未找到对应会话」；文件名含 id 但 session_meta 不符的文件不算命中。
#[test]
fn markdown_exporter_still_reports_missing_when_rollout_not_found() {
    let tmp = tempdir().unwrap();
    let codex_home = tmp.path().join(".codex");
    let db_path = codex_home.join("state_5.sqlite");
    fs::create_dir_all(&codex_home).unwrap();
    create_codex_thread_db(
        &db_path,
        &codex_home.join("sessions").join("other.jsonl"),
        "other-thread",
        "Other",
    );
    write_codex_rollout(
        &codex_home
            .join("sessions")
            .join("rollout-2026-05-19T06-00-00-missing-thread-extra.jsonl"),
        "missing-thread-extra",
        "Not this one",
    );

    let result =
        MarkdownExportService::new(Some(&db_path)).export(&session("missing-thread", "Ignored"));

    assert_eq!(result.status, ExportStatus::Failed);
    assert_eq!(result.message, "未找到对应会话");

    let result = codex_plus_data::export_markdown_from_paths(
        [db_path],
        &session("missing-thread", "Ignored"),
    );
    assert_eq!(result.status, ExportStatus::Failed);
    assert_eq!(result.message, "未找到对应会话");
}
