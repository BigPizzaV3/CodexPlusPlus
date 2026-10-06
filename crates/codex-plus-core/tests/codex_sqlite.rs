use codex_plus_core::codex_sqlite::{
    backfill_thread_models_to_default, sanitize_historical_model_suffixes,
};
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

    let result = sanitize_historical_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 1);
    assert_eq!(result.updated, 1);

    let conn = Connection::open(&db_path).unwrap();
    let model: String = conn
        .query_row("SELECT model FROM threads WHERE id = 't1'", [], |row| {
            row.get(0)
        })
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

    let result = sanitize_historical_model_suffixes(&home).unwrap();
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

    let result = sanitize_historical_model_suffixes(&home).unwrap();
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

    let result = sanitize_historical_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 0);
    assert_eq!(result.updated, 0);
}

#[test]
fn sanitize_cleans_suffix_from_logs() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();

    // logs_2.sqlite 不需要 threads 表，只需要 logs 表。
    let logs_path = home.join("logs_2.sqlite");
    let conn = Connection::open(&logs_path).unwrap();
    conn.execute(
        "CREATE TABLE logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts INTEGER NOT NULL,
            ts_nanos INTEGER NOT NULL,
            level TEXT NOT NULL,
            target TEXT NOT NULL,
            feedback_log_body TEXT,
            module_path TEXT,
            file TEXT,
            line INTEGER,
            thread_id TEXT,
            process_uuid TEXT,
            estimated_bytes INTEGER NOT NULL DEFAULT 0
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO logs (ts, ts_nanos, level, target, feedback_log_body)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        [
            "1",
            "1",
            "INFO",
            "codex_models_manager::cache",
            r#"session_loop{model="deepseek-v4-flash[1M]"}: Unknown model deepseek-v4-flash[1M] is used."#,
        ],
    )
    .unwrap();
    drop(conn);

    let result = sanitize_historical_model_suffixes(&home).unwrap();
    // threads 表为空，所以 scanned/updated 都是 0；但日志应被清理。
    assert_eq!(result.scanned, 0);
    assert_eq!(result.updated, 0);

    let conn = Connection::open(&logs_path).unwrap();
    let body: String = conn
        .query_row(
            "SELECT feedback_log_body FROM logs WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !body.contains("[1M]"),
        "expected suffix to be stripped from logs, got: {body}"
    );
    assert!(body.contains("deepseek-v4-flash"));
}

fn create_logs_table(conn: &Connection) {
    conn.execute(
        "CREATE TABLE logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts INTEGER NOT NULL,
            ts_nanos INTEGER NOT NULL,
            level TEXT NOT NULL,
            target TEXT NOT NULL,
            feedback_log_body TEXT,
            module_path TEXT,
            file TEXT,
            line INTEGER,
            thread_id TEXT,
            process_uuid TEXT,
            estimated_bytes INTEGER NOT NULL DEFAULT 0
        )",
        [],
    )
    .unwrap();
}

/// issue #2244：日志清理改成按 rowid 分批。命中行数跨过多批时，
/// 结果必须与不分批一致——每一批都要被处理，不能只清第一批。
#[test]
fn sanitize_cleans_logs_across_multiple_batches() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();

    let logs_path = home.join("logs_2.sqlite");
    let mut conn = Connection::open(&logs_path).unwrap();
    create_logs_table(&conn);

    // 每批 500 行，这里给出 1201 行命中（第 1201 行落在第三批），
    // 再穿插若干不含 '[' 的行确保分批游标只走在命中行上。
    {
        let tx = conn.transaction().unwrap();
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO logs (ts, ts_nanos, level, target, feedback_log_body)
                     VALUES ('1', '1', 'INFO', 'codex_models_manager::cache', ?1)",
                )
                .unwrap();
            for index in 0..1201 {
                insert
                    .execute([format!("model=glm-5.2[1M] index={index}")])
                    .unwrap();
                if index % 100 == 0 {
                    insert.execute([format!("plain line {index}")]).unwrap();
                }
            }
        }
        tx.commit().unwrap();
    }

    let result = sanitize_historical_model_suffixes(&home).unwrap();
    assert_eq!(result.scanned, 0);
    assert_eq!(result.updated, 0);

    let conn = Connection::open(&logs_path).unwrap();
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE feedback_log_body LIKE '%[1M]%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0, "跨批的命中行也必须被清理干净");

    // 无关行不能被误改。
    let plain: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM logs WHERE feedback_log_body = 'plain line 0'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(plain, 1);
}

/// issue #2244：没有命中行时短路返回——不打开写事务、不动表。
#[test]
fn sanitize_leaves_clean_logs_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();

    let logs_path = home.join("logs_2.sqlite");
    let conn = Connection::open(&logs_path).unwrap();
    create_logs_table(&conn);
    conn.execute(
        "INSERT INTO logs (ts, ts_nanos, level, target, feedback_log_body)
         VALUES ('1', '1', 'INFO', 't', 'no brackets here')",
        [],
    )
    .unwrap();
    drop(conn);

    sanitize_historical_model_suffixes(&home).unwrap();

    let conn = Connection::open(&logs_path).unwrap();
    let body: String = conn
        .query_row("SELECT feedback_log_body FROM logs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(body, "no brackets here");
}

/// issue #2081：回填只改「仍等于旧默认模型」的会话；
/// 用户手动改过模型的会话必须保留。
#[test]
fn backfill_only_rewrites_threads_still_on_previous_default() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    for (id, model) in [
        ("old-default", "gpt-5.4"),
        ("user-picked", "claude-opus-5"),
        ("already-new", "glm-5.2"),
    ] {
        conn.execute(
            "INSERT INTO threads (id, model, updated_at) VALUES (?1, ?2, ?3)",
            [id, model, "1000"],
        )
        .unwrap();
    }
    drop(conn);

    let updated = backfill_thread_models_to_default(&home, "gpt-5.4", "glm-5.2").unwrap();
    assert_eq!(updated, 1);

    let conn = Connection::open(&db_path).unwrap();
    let model_of = |id: &str| -> String {
        conn.query_row("SELECT model FROM threads WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .unwrap()
    };
    assert_eq!(model_of("old-default"), "glm-5.2");
    assert_eq!(model_of("user-picked"), "claude-opus-5");
    assert_eq!(model_of("already-new"), "glm-5.2");
}

/// 空值或新旧相同都不该触碰数据库。
#[test]
fn backfill_is_noop_for_empty_or_identical_models() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    create_threads_table(&conn);
    conn.execute(
        "INSERT INTO threads (id, model, updated_at) VALUES ('t1', 'gpt-5.4', '1000')",
        [],
    )
    .unwrap();
    drop(conn);

    assert_eq!(
        backfill_thread_models_to_default(&home, "", "glm-5.2").unwrap(),
        0
    );
    assert_eq!(
        backfill_thread_models_to_default(&home, "gpt-5.4", "").unwrap(),
        0
    );
    assert_eq!(
        backfill_thread_models_to_default(&home, "gpt-5.4", "gpt-5.4").unwrap(),
        0
    );

    let conn = Connection::open(&db_path).unwrap();
    let model: String = conn
        .query_row("SELECT model FROM threads WHERE id = 't1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(model, "gpt-5.4");
}

/// 没有 threads 表时安静返回 0，而不是报错打断切换流程。
#[test]
fn backfill_skips_database_without_threads_table() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let db_path = home.join("state_5.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute("CREATE TABLE unrelated (id TEXT PRIMARY KEY)", [])
        .unwrap();
    drop(conn);

    assert_eq!(
        backfill_thread_models_to_default(&home, "gpt-5.4", "glm-5.2").unwrap(),
        0
    );
}
