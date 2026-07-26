use codex_plus_core::models::{ImageOutputStatus, SessionRef};
use codex_plus_data::{ImageOutputService, image_outputs_from_paths};
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn session(id: &str, title: &str) -> SessionRef {
    SessionRef::new(id, title).unwrap()
}

fn create_codex_thread_db(path: &Path, rollout_path: &Path, thread_id: &str) {
    let db = Connection::open(path).unwrap();
    db.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads (id, rollout_path, title) VALUES (?1, ?2, 'Image Thread')",
        (thread_id, rollout_path.to_string_lossy().to_string()),
    )
    .unwrap();
}

#[test]
fn image_outputs_loads_image_generation_call_result() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    let rollout_path = tmp.path().join("rollout.jsonl");
    fs::write(
        &rollout_path,
        concat!(
            "{\"type\":\"session_meta\",\"timestamp\":\"2026-07-25T12:00:00Z\",\"payload\":{\"id\":\"thread-1\"}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-07-25T12:02:00Z\",\"payload\":{\"type\":\"image_generation_call\",\"id\":\"ig_1\",\"status\":\"completed\",\"revised_prompt\":\"draw a fox\",\"result\":\"iVBORw0KGgoAAAANSUhEUgAAAAE=\",\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-1\"}}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-07-25T12:03:00Z\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"done\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-1\"}}}\n"
        ),
    )
    .unwrap();
    create_codex_thread_db(&db_path, &rollout_path, "thread-1");

    let result = ImageOutputService::new(Some(&db_path)).load(&session("local:thread-1", ""));

    assert_eq!(result.status, ImageOutputStatus::Found);
    assert_eq!(result.session_id, "thread-1");
    assert_eq!(result.images.len(), 1);
    let image = &result.images[0];
    assert_eq!(image.id, "ig_1");
    assert_eq!(image.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(image.assistant_text.as_deref(), Some("done"));
    assert_eq!(image.output_format, "png");
    assert_eq!(image.revised_prompt.as_deref(), Some("draw a fox"));
    assert_eq!(image.timestamp.as_deref(), Some("2026-07-25T12:02:00Z"));
    assert_eq!(
        image.data_url,
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAE="
    );
}

#[test]
fn image_outputs_searches_candidate_databases_and_discovers_automation_rollout() {
    let tmp = tempdir().unwrap();
    let codex_home = tmp.path().join(".codex");
    let first_db_path = codex_home.join("sqlite").join("first.sqlite");
    let second_db_path = codex_home.join("sqlite").join("second.sqlite");
    let sessions_dir = codex_home.join("archived_sessions").join("2026");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::create_dir_all(first_db_path.parent().unwrap()).unwrap();

    let first_db = Connection::open(&first_db_path).unwrap();
    first_db
        .execute(
            "CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY)",
            [],
        )
        .unwrap();
    first_db
        .execute(
            "INSERT INTO automation_runs (thread_id) VALUES ('other-thread')",
            [],
        )
        .unwrap();

    let second_db = Connection::open(&second_db_path).unwrap();
    second_db
        .execute(
            "CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY)",
            [],
        )
        .unwrap();
    second_db
        .execute(
            "INSERT INTO automation_runs (thread_id) VALUES ('thread-2')",
            [],
        )
        .unwrap();

    fs::write(
        sessions_dir.join("rollout-thread-2.jsonl"),
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-2\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"image_generation_call\",\"id\":\"ig_2\",\"output_format\":\"webp\",\"result\":\"UklGRaaaa\"}}\n"
        ),
    )
    .unwrap();

    let result = image_outputs_from_paths(
        [first_db_path, second_db_path],
        &session("thread-2", "Ignored"),
    );

    assert_eq!(result.status, ImageOutputStatus::Found);
    assert_eq!(result.images.len(), 1);
    assert_eq!(
        result.images[0].data_url,
        "data:image/webp;base64,UklGRaaaa"
    );
}

#[test]
fn image_outputs_returns_found_with_empty_images_when_rollout_has_no_generation_results() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    let rollout_path = tmp.path().join("rollout.jsonl");
    fs::write(
        &rollout_path,
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"no image\"}]}}\n",
    )
    .unwrap();
    create_codex_thread_db(&db_path, &rollout_path, "thread-3");

    let result = ImageOutputService::new(Some(&db_path)).load(&session("thread-3", ""));

    assert_eq!(result.status, ImageOutputStatus::Found);
    assert!(result.images.is_empty());
}
