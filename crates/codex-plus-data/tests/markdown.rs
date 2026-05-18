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
