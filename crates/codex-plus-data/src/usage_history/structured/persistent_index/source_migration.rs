use super::PersistentIndex;

#[cfg(windows)]
use super::{BUDGET_BLOCKED, scan};
#[cfg(windows)]
use anyhow::ensure;
#[cfg(windows)]
use rusqlite::{OptionalExtension, params};
#[cfg(windows)]
use std::time::{Duration, Instant};

impl PersistentIndex {
    #[cfg(not(windows))]
    pub(super) fn migrate_sources(&mut self) -> anyhow::Result<bool> {
        self.source_migration_complete = true;
        Ok(true)
    }

    #[cfg(windows)]
    pub(super) fn migrate_sources(&mut self) -> anyhow::Result<bool> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        let started = Instant::now();
        // 每轮只在后台做少量事务；空键表示仍需查找旧来源或完成最后一次页整理。
        loop {
            let migration: Option<(Vec<u8>, Vec<u8>)> = self
                .db
                .query_row(
                    "SELECT old_source, new_source FROM usage_source_migration WHERE singleton = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let scanning = match &migration {
                Some((old, new)) if old.is_empty() => {
                    ensure!(new.is_empty(), "invalid source migration state");
                    true
                }
                Some(_) => false,
                None => {
                    self.source_migration_complete = true;
                    return Ok(true);
                }
            };
            let (old, new) = if !scanning {
                migration.unwrap()
            } else {
                let mut sources = self.db.prepare(
                    "SELECT s.source FROM usage_sources s
                         WHERE substr(s.source, 1, 1) = X'57'
                         ORDER BY
                           (SELECT COUNT(*) FROM usage_observations o WHERE o.source = s.source) +
                           (SELECT COUNT(*) FROM execution_observations e WHERE e.source = s.source)
                           DESC",
                )?;
                let mut rows = sources.query([])?;
                let mut next = None;
                while let Some(row) = rows.next()? {
                    let old: Vec<u8> = row.get(0)?;
                    let new = scan::encode_source(&scan::decode_source(&old)?);
                    if old != new {
                        next = Some((old, new));
                        break;
                    }
                }
                drop(rows);
                drop(sources);
                let Some((old, new)) = next else {
                    self.repack_usage_payloads()?;
                    self.db
                        .execute("DELETE FROM usage_source_migration WHERE singleton = 1", [])?;
                    self.source_migration_complete = true;
                    return Ok(true);
                };
                let tx = self.db.transaction()?;
                tx.execute(
                    "INSERT INTO usage_sources(
                            source, identity, offset, parser_state, complete, scope_session,
                            scope_indexed, scope_valid, execution_indexed, execution_valid,
                            usage_cutoff
                         )
                         SELECT ?2, identity, offset, parser_state, complete, scope_session,
                            scope_indexed, scope_valid, execution_indexed, execution_valid,
                            usage_cutoff
                         FROM usage_sources WHERE source = ?1",
                    params![old, new],
                )?;
                tx.execute(
                    "UPDATE usage_source_migration
                         SET old_source = ?1, new_source = ?2 WHERE singleton = 1",
                    params![old, new],
                )?;
                tx.commit()?;
                (old, new)
            };

            let pages: u64 = self
                .db
                .pragma_query_value(None, "page_count", |row| row.get(0))?;
            let free: u64 = self
                .db
                .pragma_query_value(None, "freelist_count", |row| row.get(0))?;
            let maximum: u64 = self
                .db
                .pragma_query_value(None, "max_page_count", |row| row.get(0))?;
            let available = maximum.saturating_sub(pages).saturating_add(free);
            // 接近页上限时减小批次，避免在迁移本身耗尽剩余空间。
            let batch = if available < 32 {
                16
            } else if available < 64 {
                32
            } else if available < 128 {
                64
            } else {
                256
            };
            let tx = self.db.transaction()?;
            let mut changed = false;
            for sql in [
                "UPDATE usage_observations SET source = ?1 WHERE source = ?2 AND position IN
                    (SELECT position FROM usage_observations WHERE source = ?2 ORDER BY position LIMIT ?3)",
                "UPDATE execution_observations SET source = ?1 WHERE source = ?2 AND start IN
                    (SELECT start FROM execution_observations WHERE source = ?2 ORDER BY start LIMIT ?3)",
                "UPDATE usage_scope_parents SET source = ?1 WHERE source = ?2 AND (session, parent) IN
                    (SELECT session, parent FROM usage_scope_parents WHERE source = ?2 LIMIT ?3)",
                "UPDATE usage_scope_aliases SET source = ?1 WHERE source = ?2 AND (alias, session) IN
                    (SELECT alias, session FROM usage_scope_aliases WHERE source = ?2 LIMIT ?3)",
                "UPDATE usage_count_gaps SET source = ?1 WHERE source = ?2 AND session IN
                    (SELECT session FROM usage_count_gaps WHERE source = ?2 LIMIT ?3)",
                "UPDATE usage_unindexed_checkpoints SET source = ?1 WHERE source = ?2 AND source IN
                    (SELECT source FROM usage_unindexed_checkpoints WHERE source = ?2 LIMIT ?3)",
                "UPDATE usage_sealed_sessions SET source = ?1 WHERE source = ?2 AND session IN
                    (SELECT session FROM usage_sealed_sessions WHERE source = ?2 LIMIT ?3)",
                "UPDATE execution_compacted_sources SET source = ?1 WHERE source = ?2 AND (day, session) IN
                    (SELECT day, session FROM execution_compacted_sources WHERE source = ?2 LIMIT ?3)",
            ] {
                if tx.execute(sql, params![new, old, batch])? != 0 {
                    changed = true;
                    break;
                }
            }
            if !changed {
                tx.execute("DELETE FROM usage_sources WHERE source = ?1", [&old])?;
                tx.execute(
                    "UPDATE usage_source_migration SET old_source = X'', new_source = X''
                     WHERE singleton = 1",
                    [],
                )?;
            }
            tx.commit()?;

            let pages: u64 = self
                .db
                .pragma_query_value(None, "page_count", |row| row.get(0))?;
            let free: u64 = self
                .db
                .pragma_query_value(None, "freelist_count", |row| row.get(0))?;
            if maximum.saturating_sub(pages).saturating_add(free) < 32 {
                self.repack_usage_payloads()?;
                return Ok(false);
            }
            if started.elapsed() >= Duration::from_millis(100) {
                return Ok(false);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::usage_history::structured::persistent_index::scan;
    use rusqlite::params;
    use std::os::windows::ffi::OsStrExt;

    #[test]
    fn migrates_full_index_after_interruption_without_changing_records() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("usage.sqlite");
        let path = directory.path().join(
            "sessions/2026/11/11/rollout-2026-11-11-00000000-0000-4000-8000-000000000010.jsonl",
        );
        let mut old = vec![b'W'];
        for unit in path.as_os_str().encode_wide() {
            old.extend_from_slice(&unit.to_le_bytes());
        }
        let new = scan::encode_source(&path);
        assert_ne!(old, new);
        let mut index = PersistentIndex::open(&database).unwrap();
        let tx = index.db.transaction().unwrap();
        tx.execute(
            "INSERT INTO usage_sources
             (source, identity, offset, parser_state, complete, scope_session, scope_indexed,
              scope_valid, execution_indexed, execution_valid, usage_cutoff)
             VALUES (?1, X'01', 12345, X'7B7D', 1, 'session-a', 1, 1, 1, 1, NULL)",
            [&old],
        )
        .unwrap();
        for position in 0..4569_i64 {
            tx.execute(
                "INSERT INTO usage_observations(source, position, event_id, session, payload)
                 VALUES (?1, ?2, ?3, 'session-a', ?4)",
                params![old, position, position.to_le_bytes(), vec![b'x'; 400]],
            )
            .unwrap();
        }
        for position in 0..5936_i64 {
            tx.execute(
                "INSERT INTO execution_observations
                 (source, start, end, session, call_id, timestamp_ms, is_output, name, namespace, call_kind)
                 VALUES (?1, ?2, ?3, 'session-a', ?4, 1000, 0, 'skill', '', 'skill')",
                params![old, position, position + 1, format!("call-{position}")],
            ).unwrap();
        }
        tx.execute(
            "INSERT INTO usage_scope_parents VALUES (?1, 'session-a', '')",
            [&old],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO usage_scope_aliases VALUES (?1, 'alias-a', 'session-a')",
            [&old],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO usage_count_gaps VALUES (?1, 'session-a')",
            [&old],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO usage_unindexed_checkpoints VALUES (?1, 'gap')",
            [&old],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO usage_sealed_sessions VALUES ('session-a', ?1, '2026-01-01', 1, NULL)",
            [&old],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO execution_compacted_sources VALUES (?1, '2026-01-01', 'session-a')",
            [&old],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO usage_daily
             VALUES ('2026-11-11', 'session-a', 'model', 'provider', '', 'main',
                     0, 120, 20, 20, 30, 0, 2, 0, 2)",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO usage_source_migration VALUES (1, X'', X'')",
            [],
        )
        .unwrap();
        tx.execute_batch(
            "CREATE TRIGGER reject_source_migration BEFORE UPDATE OF source ON usage_observations
             BEGIN SELECT RAISE(ABORT, 'synthetic migration failure'); END;",
        )
        .unwrap();
        tx.commit().unwrap();

        let pages: u64 = index
            .db
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap();
        let limit = pages + 18;
        index
            .db
            .pragma_update(None, "max_page_count", limit)
            .unwrap();
        let error = index.migrate_sources().unwrap_err();
        assert!(format!("{error:#}").contains("synthetic migration failure"));
        assert!(index.source_state(&old).unwrap().is_some());
        assert!(index.source_state(&new).unwrap().is_some());
        assert_eq!(
            index
                .db
                .query_row("SELECT COUNT(*) FROM usage_source_migration", [], |row| row
                    .get::<_, i64>(0),)
                .unwrap(),
            1
        );
        drop(index);

        let mut index = PersistentIndex::open(&database).unwrap();
        index
            .db
            .pragma_update(None, "max_page_count", limit)
            .unwrap();
        index
            .db
            .execute_batch("DROP TRIGGER reject_source_migration")
            .unwrap();
        for _ in 0..200 {
            if index.migrate_sources().unwrap() {
                break;
            }
        }
        assert!(index.source_migration_complete);
        assert!(index.source_state(&old).unwrap().is_none());
        let saved = index.source_state(&new).unwrap().unwrap();
        assert_eq!(saved.offset, 12345);
        assert!(saved.complete);
        for (table, expected) in [
            ("usage_observations", 4569),
            ("execution_observations", 5936),
            ("usage_scope_parents", 1),
            ("usage_scope_aliases", 1),
            ("usage_count_gaps", 1),
            ("usage_unindexed_checkpoints", 1),
            ("usage_sealed_sessions", 1),
            ("execution_compacted_sources", 1),
        ] {
            assert_eq!(
                index
                    .db
                    .query_row(
                        &format!("SELECT COUNT(*) FROM {table} WHERE source = ?1"),
                        [&new],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                expected,
                "{table}"
            );
        }
        assert_eq!(
            index
                .db
                .query_row("SELECT input + output FROM usage_daily", [], |row| row
                    .get::<_, i64>(0),)
                .unwrap(),
            150
        );
        assert_eq!(
            index
                .db
                .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get::<_, i64>(0)
                },)
                .unwrap(),
            0
        );
        assert_eq!(
            index
                .db
                .pragma_query_value(None, "max_page_count", |row| row.get::<_, u64>(0))
                .unwrap(),
            limit
        );
    }
}
