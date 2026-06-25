use codex_plus_core::codex_sqlite::sanitize_thread_model_suffixes;
use rusqlite::Connection;

fn create_threads_table(conn: &Connection) {
    conn.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            model TEXT,
            updated_at INTEGER
        )",
        [],
    )
    .unwrap();
}

#[test]
fn sanitize_strips_suffix_from_thread_model() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    conn.execute(
        "INSERT INTO threads (id, model, updated_at) VALUES (?1, ?2, ?3)",
        ["t1", "deepseek/deepseek-v4-flash[1M]", "1000"],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_thread_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 1);
    assert_eq!(result.updated, 1);

    let conn = Connection::open(&db_path).unwrap();
    let model: String = conn
        .query_row("SELECT model FROM threads WHERE id = 't1'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(model, "deepseek/deepseek-v4-flash");
}

#[test]
fn sanitize_skips_models_without_suffix() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    conn.execute(
        "INSERT INTO threads (id, model, updated_at) VALUES (?1, ?2, ?3)",
        ["t1", "gpt-5.5", "1000"],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_thread_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 0);
    assert_eq!(result.updated, 0);
}

#[test]
fn sanitize_skips_invalid_suffixes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    conn.execute(
        "INSERT INTO threads (id, model, updated_at) VALUES (?1, ?2, ?3)",
        ["t1", "foo[bar]", "1000"],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_thread_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 1);
    assert_eq!(result.updated, 0);
}

#[test]
fn sanitize_handles_null_model() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    conn.execute(
        "INSERT INTO threads (id, model, updated_at) VALUES (?1, ?2, ?3)",
        rusqlite::params!["t1", rusqlite::types::Null, "1000"],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_thread_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 0);
    assert_eq!(result.updated, 0);
}
