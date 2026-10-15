use super::*;
use rusqlite::params;
use tempfile::tempdir;

fn threads(path: &Path) -> Connection {
    let db = Connection::open(path).unwrap();
    db.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, rollout_path TEXT, updated_at_ms INTEGER)").unwrap();
    db
}

fn insert(db: &Connection, id: &str, title: &str, updated: Option<i64>) {
    db.execute(
        "INSERT INTO threads VALUES (?1, ?2, '', ?3)",
        params![id, title, updated],
    )
    .unwrap();
}

fn ids(page: &LocalSessionPage) -> Vec<&str> {
    assert!(page.errors.is_empty(), "{:?}", page.errors);
    page.sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect()
}

#[test]
fn deep_pages_reuse_index_and_deduplicate_before_slicing() {
    let temp = tempdir().unwrap();
    let paths = [
        temp.path().join("current.db"),
        temp.path().join("legacy.db"),
    ];
    let current = threads(&paths[0]);
    let legacy = threads(&paths[1]);
    current.execute_batch("BEGIN").unwrap();
    legacy.execute_batch("BEGIN").unwrap();
    for number in 0..1500 {
        let id = format!("t{number:04}");
        insert(&current, &id, "current", Some(number));
        if number % 2 == 0 {
            insert(&legacy, &id, "legacy", Some(number + 10000));
        }
    }
    current.execute_batch("COMMIT").unwrap();
    legacy.execute_batch("COMMIT").unwrap();
    let mut pager = LocalSessionPager::default();
    let expected = pager.list_page(&paths, 0, 2000);
    assert_eq!(expected.total_count, 1500);
    assert_eq!(expected.sessions.len(), 1500);
    assert!(!expected.has_more);
    let index_address = pager.databases[0].1.index.as_ptr();
    for offset in [0, 49, 700, 1400, 1490, 1500, usize::MAX] {
        let page = pager.list_page(&paths, offset, 50);
        assert!(page.errors.is_empty());
        assert_eq!(page.total_count, 1500);
        let wanted = expected
            .sessions
            .iter()
            .skip(offset)
            .take(50)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(page.sessions, wanted);
        assert_eq!(page.has_more, offset.saturating_add(50) < 1500);
    }
    assert_eq!(pager.databases[0].1.index.as_ptr(), index_address);
    assert!(
        expected
            .sessions
            .iter()
            .take(750)
            .all(|row| row.title == "legacy")
    );
}

#[test]
fn stable_order_handles_null_negative_times_and_source_ties() {
    let temp = tempdir().unwrap();
    let paths = [temp.path().join("a.db"), temp.path().join("b.db")];
    let first = threads(&paths[0]);
    let second = threads(&paths[1]);
    for (id, time) in [
        ("a", Some(0)),
        ("b", Some(0)),
        ("negative", Some(-1)),
        ("unknown", None),
    ] {
        insert(&first, id, "first", time);
        insert(&second, id, "second", time);
    }
    let mut pager = LocalSessionPager::default();
    let page = pager.list_page(&paths, 0, 10);
    assert_eq!(ids(&page), ["b", "a", "negative", "unknown"]);
    assert!(page.sessions.iter().all(|session| session.title == "first"));
    assert_eq!(ids(&pager.list_page(&paths, 2, 1)), ["negative"]);
    let reversed = [paths[1].clone(), paths[0].clone()];
    assert!(
        pager
            .list_page(&reversed, 0, 10)
            .sessions
            .iter()
            .all(|row| row.title == "second")
    );
}

#[test]
fn wal_commits_refresh_counts_order_and_details_without_main_file_changes() {
    let temp = tempdir().unwrap();
    let paths = [temp.path().join("state.db")];
    let writer = threads(&paths[0]);
    writer
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    insert(&writer, "a", "old", Some(1));
    let mut pager = LocalSessionPager::default();
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["a"]);
    let stamp = FileStamp::read(&paths[0], 100).unwrap().unwrap();
    insert(&writer, "b", "new", Some(2));
    assert!(stamp == FileStamp::read(&paths[0], 100).unwrap().unwrap());
    let page = pager.list_page(&paths, 0, 1);
    assert_eq!(ids(&page), ["b"]);
    assert_eq!(page.total_count, 2);
    assert!(page.has_more);
    writer
        .execute(
            "UPDATE threads SET title = 'edited', updated_at_ms = 3 WHERE id = 'a'",
            [],
        )
        .unwrap();
    let page = pager.list_page(&paths, 0, 1);
    assert_eq!(ids(&page), ["a"]);
    assert_eq!(page.sessions[0].title, "edited");
    writer
        .execute_batch("BEGIN; DELETE FROM threads; ROLLBACK;")
        .unwrap();
    assert_eq!(pager.list_page(&paths, 0, 10).total_count, 2);
    writer
        .execute("DELETE FROM threads WHERE id = 'a'", [])
        .unwrap();
    let page = pager.list_page(&paths, 0, 10);
    assert_eq!(ids(&page), ["b"]);
    assert_eq!(page.total_count, 1);
    writer
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["b"]);
}

#[test]
fn read_snapshot_version_does_not_skip_a_concurrent_wal_commit() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("state.db");
    let writer = threads(&path);
    writer.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    let reader = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let before: i64 = reader
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    let snapshot = reader.unchecked_transaction().unwrap();
    snapshot
        .query_row("SELECT COUNT(*) FROM sqlite_schema", [], |_| Ok(()))
        .unwrap();
    insert(&writer, "a", "new", Some(1));
    let during: i64 = snapshot
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, during);
    let count: i64 = snapshot
        .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    drop(snapshot);
    let after: i64 = reader
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    assert_ne!(before, after);
}

#[test]
fn automation_duplicates_do_not_starve_later_pages() {
    let temp = tempdir().unwrap();
    let paths = [temp.path().join("automation.db")];
    let writer = Connection::open(&paths[0]).unwrap();
    writer.execute_batch("CREATE TABLE automation_runs (thread_id TEXT, thread_title TEXT, source_cwd TEXT, status TEXT, updated_at INTEGER, created_at INTEGER)").unwrap();
    writer.execute_batch("BEGIN").unwrap();
    for number in 0..200 {
        writer
            .execute(
                "INSERT INTO automation_runs VALUES ('same', ?1, '/cwd', 'ARCHIVED', ?2, 1)",
                params![format!("run {number}"), number + 10],
            )
            .unwrap();
    }
    writer.execute_batch("INSERT INTO automation_runs VALUES ('old', 'old', '', '', NULL, 1), ('', 'ignored', '', '', 999, 999)").unwrap();
    writer.execute_batch("COMMIT").unwrap();
    let mut pager = LocalSessionPager::default();
    let first = pager.list_page(&paths, 0, 1);
    assert_eq!(ids(&first), ["same"]);
    assert_eq!(first.total_count, 2);
    assert_eq!(first.sessions[0].title, "run 199");
    assert!(first.sessions[0].archived);
    assert!(first.has_more);
    let second = pager.list_page(&paths, 1, 1);
    assert_eq!(ids(&second), ["old"]);
    assert!(!second.has_more);
}

#[test]
fn schema_changes_and_child_references_invalidate_the_index() {
    let temp = tempdir().unwrap();
    let paths = [temp.path().join("state.db")];
    let writer = threads(&paths[0]);
    writer.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    insert(&writer, "parent", "parent", Some(1));
    insert(&writer, "child", "child", Some(2));
    let mut pager = LocalSessionPager::default();
    assert_eq!(pager.list_page(&paths, 0, 10).total_count, 2);
    writer.execute_batch("CREATE TABLE thread_spawn_edges(child_thread_id TEXT); INSERT INTO thread_spawn_edges VALUES ('child');").unwrap();
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["parent"]);
    writer.execute_batch("DELETE FROM thread_spawn_edges; CREATE TABLE agent_job_items(assigned_thread_id TEXT); INSERT INTO agent_job_items VALUES ('child');").unwrap();
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["parent"]);
    writer.execute_batch("DROP TABLE agent_job_items").unwrap();
    assert_eq!(pager.list_page(&paths, 0, 10).total_count, 2);
}

#[test]
fn missing_corrupt_and_locked_databases_keep_healthy_results_and_recover() {
    let temp = tempdir().unwrap();
    let paths = [
        temp.path().join("good.db"),
        temp.path().join("bad.db"),
        temp.path().join("missing.db"),
    ];
    let writer = threads(&paths[0]);
    insert(&writer, "good", "good", Some(1));
    std::fs::write(&paths[1], b"not a database").unwrap();
    let mut pager = LocalSessionPager::default();
    let partial = pager.list_page(&paths, 0, 10);
    assert_eq!(partial.sessions.len(), 1);
    assert_eq!(partial.total_count, 1);
    assert_eq!(partial.errors.len(), 1);
    assert!(partial.errors[0].contains("bad.db"));
    // 替换不支持的库后再次查询，不缓存错误或旧的总数。
    pager.list_page(&paths[..1], 0, 10);
    std::fs::remove_file(&paths[1]).unwrap();
    let recovered = threads(&paths[1]);
    insert(&recovered, "recovered", "recovered", Some(2));
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["recovered", "good"]);
    let new = threads(&paths[2]);
    insert(&new, "new", "new", Some(3));
    assert_eq!(pager.list_page(&paths, 0, 10).total_count, 3);
    recovered.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let partial = pager.list_page(&paths, 0, 10);
    assert_eq!(partial.errors.len(), 1);
    assert_eq!(partial.total_count, 2);
    recovered.execute_batch("ROLLBACK").unwrap();
    assert_eq!(pager.list_page(&paths, 0, 10).total_count, 3);
}

#[test]
fn detail_decoding_failure_keeps_count_and_other_database_results() {
    let temp = tempdir().unwrap();
    let paths = [temp.path().join("good.db"), temp.path().join("bad.db")];
    let good = threads(&paths[0]);
    let bad = threads(&paths[1]);
    insert(&good, "good", "good", Some(1));
    bad.execute("INSERT INTO threads VALUES ('bad', x'1234', '', 2)", [])
        .unwrap();
    let mut pager = LocalSessionPager::default();
    let partial = pager.list_page(&paths, 0, 10);
    assert_eq!(partial.sessions.len(), 1);
    assert_eq!(partial.total_count, 2);
    assert_eq!(partial.errors.len(), 1);
    bad.execute("UPDATE threads SET title = 'fixed'", [])
        .unwrap();
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["bad", "good"]);
}

#[test]
fn restoring_same_size_database_is_detected_even_when_timestamps_are_preserved() {
    let temp = tempdir().unwrap();
    let paths = [temp.path().join("state.db")];
    let replacement = temp.path().join("restored.db");
    let original = threads(&paths[0]);
    insert(&original, "old", "old", Some(1));
    drop(original);
    let restored = threads(&replacement);
    insert(&restored, "new", "new", Some(2));
    drop(restored);
    let mut pager = LocalSessionPager::default();
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["old"]);
    let stamp = FileStamp::read(&paths[0], 100).unwrap().unwrap();
    // 备份还原可能保留长度和时间戳，也可能恰好保留 SQLite 的变更计数器。
    std::fs::copy(&replacement, &paths[0]).unwrap();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&paths[0])
        .unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(stamp.modified.unwrap()))
        .unwrap();
    drop(file);
    assert!(stamp == FileStamp::read(&paths[0], 100).unwrap().unwrap());
    pager.databases[0].1.refreshed_at = Some(Instant::now() - INDEX_MAX_AGE);
    let page = pager.list_page(&paths, 0, 10);
    assert_eq!(ids(&page), ["new"]);
    assert_eq!(page.total_count, 1);
}

#[test]
fn query_releases_file_handles_so_database_can_be_renamed_deleted_and_recreated() {
    let temp = tempdir().unwrap();
    let paths = [temp.path().join("state.db")];
    let writer = threads(&paths[0]);
    insert(&writer, "old", "old", Some(1));
    drop(writer);
    let mut pager = LocalSessionPager::default();
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["old"]);
    let backup = temp.path().join("backup.db");
    std::fs::rename(&paths[0], &backup).unwrap();
    let missing = pager.list_page(&paths, 0, 10);
    assert_eq!(missing.total_count, 0);
    assert!(missing.errors.is_empty());
    let writer = threads(&paths[0]);
    insert(&writer, "new", "new", Some(2));
    drop(writer);
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["new"]);
    std::fs::remove_file(&paths[0]).unwrap();
    std::fs::rename(&backup, &paths[0]).unwrap();
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["old"]);
}

#[test]
fn commit_after_snapshot_does_not_cache_old_index_under_new_fingerprint() {
    let temp = tempdir().unwrap();
    let paths = [temp.path().join("state.db")];
    let writer = threads(&paths[0]);
    writer.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    insert(&writer, "a", "old", Some(1));
    let mut pager = LocalSessionPager::default();
    assert_eq!(ids(&pager.list_page(&paths, 0, 10)), ["a"]);
    // 保持主写连接存活，避免测试提交后关闭最后一个写连接而触发 checkpoint。
    let concurrent = Connection::open(&paths[0]).unwrap();
    pager.after_snapshot = Some(Box::new(move || insert(&concurrent, "b", "new", Some(2))));
    let snapshot = pager.list_page(&paths, 0, 10);
    assert_eq!(ids(&snapshot), ["a"]);
    assert_eq!(snapshot.total_count, 1);
    assert!(pager.databases[0].1.stamp.is_none());
    let fresh = pager.list_page(&paths, 0, 10);
    assert_eq!(ids(&fresh), ["b", "a"]);
    assert_eq!(fresh.total_count, 2);
}

#[test]
fn wal_reuse_with_unchanged_size_and_timestamps_is_detected_by_index_header() {
    let temp = tempdir().unwrap();
    let paths = [temp.path().join("state.db")];
    let writer = threads(&paths[0]);
    writer
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0")
        .unwrap();
    for number in 0..10 {
        insert(&writer, &format!("t{number}"), "old", Some(number));
    }
    writer
        .execute_batch("PRAGMA wal_checkpoint(RESTART)")
        .unwrap();
    insert(&writer, "a", "new", Some(11));
    let mut pager = LocalSessionPager::default();
    assert_eq!(pager.list_page(&paths, 0, 20).total_count, 11);
    let before = DatabaseStamp::read(&paths[0]).unwrap().unwrap();
    insert(&writer, "b", "new", Some(12));
    for (suffix, stamp) in [
        ("-wal", before.wal.as_ref().unwrap()),
        ("-shm", before.shm.as_ref().unwrap()),
    ] {
        let path = PathBuf::from(format!("{}{suffix}", paths[0].display()));
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(stamp.modified.unwrap()))
            .unwrap();
    }
    let after = DatabaseStamp::read(&paths[0]).unwrap().unwrap();
    assert!(before.main == after.main);
    assert!(before.wal == after.wal);
    assert!(before.shm != after.shm);
    let fresh = pager.list_page(&paths, 0, 20);
    assert!(fresh.errors.is_empty());
    assert_eq!(fresh.total_count, 12);
    assert_eq!(fresh.sessions[0].id, "b");
}
