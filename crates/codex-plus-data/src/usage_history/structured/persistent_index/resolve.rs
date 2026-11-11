//! 从已提交来源候选恢复宿主原有的累计计数归并。
use super::{ResolvedEvent, checkpoint, scan::decode_source};
use crate::usage_history::structured::{
    FileState, Index, Snapshot, UsageEvent, count_resolution, index_budget,
};
use anyhow::{Context, ensure};
use chrono::DateTime;
use rusqlite::{OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

// 覆盖归并哈希表、图节点、边、累计基线和结果向量；字符串与两个事件副本另外计量。
const EVENT_WORK_BYTES: usize = 1024;
const OBSERVATION_LINK_BYTES: usize = 128;

pub(super) fn check_session_checkpoints(tx: &Transaction<'_>, session: &str) -> anyhow::Result<()> {
    let unindexed: Option<String> = tx
        .prepare_cached(
            "SELECT u.error
             FROM usage_unindexed_checkpoints u
             JOIN usage_sources s ON s.source = u.source
             WHERE s.scope_session = ?1
                OR EXISTS (
                    SELECT 1 FROM usage_observations o
                    WHERE o.source = u.source AND o.session = ?1
                )
                OR EXISTS (
                    SELECT 1 FROM usage_count_gaps g
                    WHERE g.source = u.source AND g.session = ?1
                )
                OR EXISTS (
                    SELECT 1 FROM usage_scope_parents p
                    WHERE p.source = u.source AND (p.session = ?1 OR p.parent = ?1)
                )
                OR EXISTS (
                    SELECT 1 FROM usage_scope_aliases a
                    WHERE a.source = u.source AND a.session = ?1
                )
             LIMIT 1",
        )?
        .query_row([session], |row| row.get(0))
        .optional()?;
    if let Some(error) = unindexed {
        anyhow::bail!("usage source requires checkpoint recovery: {error}");
    }
    Ok(())
}

pub(super) fn resolve(
    tx: &Transaction<'_>,
    sessions: &BTreeSet<String>,
    provider_names: &BTreeMap<String, String>,
) -> anyhow::Result<Vec<ResolvedEvent>> {
    resolve_with_limit(
        tx,
        sessions,
        provider_names,
        index_budget::MAX_RETAINED_BYTES,
    )
}

fn resolve_with_limit(
    tx: &Transaction<'_>,
    sessions: &BTreeSet<String>,
    provider_names: &BTreeMap<String, String>,
    memory_limit: usize,
) -> anyhow::Result<Vec<ResolvedEvent>> {
    let mut resolved = Vec::<ResolvedEvent>::new();
    let mut result_heap = 0usize;
    for session in sessions {
        let retained = result_heap + resolved.capacity() * std::mem::size_of::<ResolvedEvent>();
        let available = memory_limit
            .checked_sub(retained)
            .context("usage resolution result exceeds memory budget")?;
        let events = resolve_session(tx, session, provider_names, available)?;
        let added_heap: usize = events
            .iter()
            .map(|event| {
                [
                    &event.session,
                    &event.day,
                    &event.model,
                    &event.provider,
                    &event.role,
                ]
                .iter()
                .map(|text| text.capacity())
                .sum::<usize>()
                    + event.provider_source.as_ref().map_or(0, String::capacity)
            })
            .sum();
        // 合并时旧分配、新分配和本会话返回向量可能短暂同时存在。
        let merge_bytes = retained
            + added_heap
            + (resolved.len() + events.len() + events.capacity())
                * std::mem::size_of::<ResolvedEvent>();
        ensure!(
            merge_bytes <= memory_limit,
            "usage resolution result exceeds memory budget ({memory_limit} bytes)"
        );
        resolved.try_reserve_exact(events.len())?;
        result_heap += added_heap;
        resolved.extend(events);
    }
    Ok(resolved)
}

fn resolve_session(
    tx: &Transaction<'_>,
    session: &str,
    provider_names: &BTreeMap<String, String>,
    memory_limit: usize,
) -> anyhow::Result<Vec<ResolvedEvent>> {
    check_session_checkpoints(tx, session)?;
    let mut files = BTreeMap::<PathBuf, FileState>::new();
    let mut known_collisions = HashSet::<[u8; 32]>::new();
    let mut observations = 0usize;
    let mut charged: usize = provider_names
        .iter()
        .map(|(key, value)| key.capacity() + value.capacity() + 128)
        .sum();
    let check = |bytes| {
        ensure!(
            bytes <= memory_limit,
            "usage session {session} exceeds resolution memory budget ({memory_limit} bytes)"
        );
        Ok::<_, anyhow::Error>(())
    };
    check(charged)?;
    {
        let mut gap_sources = tx.prepare_cached(
            "SELECT s.source, s.parser_state FROM usage_count_gaps g
             JOIN usage_sources s ON s.source = g.source WHERE g.session = ?1",
        )?;
        let mut gap_rows = gap_sources.query([session])?;
        while let Some(row) = gap_rows.next()? {
            let source: Vec<u8> = row.get(0)?;
            let path = decode_source(&source)?;
            if !files.contains_key(&path) {
                let parser_state: Vec<u8> = row.get(1)?;
                check(charged.saturating_add(parser_state.len()))?;
                let state = checkpoint::restore(&parser_state)?;
                charged += state.retained_charge + path.as_os_str().len() * 2 + 512;
                check(charged)?;
                files.insert(path, state);
            }
        }
    }
    {
        let mut sources = tx.prepare_cached(
            "SELECT source, COUNT(*) FROM usage_observations
             WHERE session = ?1 GROUP BY source ORDER BY source",
        )?;
        let mut source_rows = sources.query([session])?;
        while let Some(source_row) = source_rows.next()? {
            let source: Vec<u8> = source_row.get(0)?;
            let count: usize = source_row.get(1)?;
            let path = decode_source(&source)?;
            if !files.contains_key(&path) {
                let parser_state: Vec<u8> = tx.query_row(
                    "SELECT parser_state FROM usage_sources WHERE source = ?1",
                    [&source],
                    |row| row.get(0),
                )?;
                check(charged.saturating_add(parser_state.len()))?;
                let state = checkpoint::restore(&parser_state)?;
                charged += state.retained_charge + path.as_os_str().len() * 2 + 512;
                check(charged)?;
                files.insert(path.clone(), state);
            }
            let state = files.get_mut(&path).unwrap();
            check(charged.saturating_add(count.saturating_mul(std::mem::size_of::<UsageEvent>())))?;
            state.events.try_reserve_exact(count)?;
            charged += state.events.capacity() * std::mem::size_of::<UsageEvent>();
            check(charged)?;
            let mut statement = tx.prepare_cached(
                "SELECT event_id, payload FROM usage_observations
                 WHERE source = ?1 AND session = ?2 ORDER BY position",
            )?;
            let mut rows = statement.query(rusqlite::params![source, session])?;
            while let Some(row) = rows.next()? {
                let id: [u8; 32] = row
                    .get_ref(0)?
                    .as_blob()?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid stored usage identity"))?;
                let payload = row.get_ref(1)?.as_blob()?;
                check(charged.saturating_add(checkpoint::observation_size(payload)?))?;
                let event = checkpoint::restore_event(id, session.to_owned(), payload)?;
                ensure!(
                    event.original_id() == id,
                    "usage observation identity mismatch"
                );
                // 只折叠完全相同的相邻副本；不同 raw delta、缓存已知性或上下文仍参与归并。
                if state.events.last() == Some(&event) {
                    continue;
                }
                charged += std::mem::size_of::<UsageEvent>()
                    + index_budget::event_heap(&event) * 3
                    + EVENT_WORK_BYTES
                    + OBSERVATION_LINK_BYTES;
                check(charged)?;
                observations += 1;
                state.events.push(event);
            }
        }
    }
    {
        let mut statement = tx.prepare_cached(
            "SELECT DISTINCT f.id
             FROM usage_observations o
             JOIN usage_collision_families f ON f.id = o.event_id
             WHERE o.session = ?1",
        )?;
        let rows = statement.query_map([session], |row| row.get::<_, Vec<u8>>(0))?;
        for row in rows {
            known_collisions.insert(
                row?.try_into()
                    .map_err(|_| anyhow::anyhow!("invalid collision identity"))?,
            );
        }
    }
    let mut turns = HashMap::<[u8; 32], (bool, HashSet<String>)>::new();
    for event in files
        .values()
        .filter(|state| !state.identity_conflict && !state.session.is_empty())
        .flat_map(|state| &state.events)
    {
        let family = turns
            .entry(event.id)
            .or_insert_with(|| (event.inherited, HashSet::new()));
        if !family.0 && event.inherited {
            continue;
        }
        if family.0 && !event.inherited {
            family.0 = false;
            family.1.clear();
        }
        if !event.turn.is_empty() {
            family.1.insert(event.turn.clone());
        }
    }
    for (id, (_, turns)) in turns {
        if turns.len() > 1 {
            tx.execute(
                "INSERT OR IGNORE INTO usage_collision_families(id) VALUES (?1)",
                [id.as_slice()],
            )?;
            known_collisions.insert(id);
        }
    }
    for event in files.values_mut().flat_map(|state| &mut state.events) {
        let original = event.id;
        if known_collisions.contains(&original) {
            if event.turn.is_empty() {
                event.context_conflicts.turn = true;
                event.count_uncertain = true;
            } else {
                let mut digest = Sha256::new();
                digest.update(b"usage-turn-collision-v1\0");
                digest.update(original);
                digest.update(event.turn.as_bytes());
                event.id = digest.finalize().into();
            }
        }
    }
    drop(known_collisions);
    let index = Index {
        files,
        provider_names: provider_names.clone(),
        ..Index::default()
    };
    let mut snapshot = Snapshot {
        events: Vec::with_capacity(observations),
        ..Snapshot::default()
    };
    let mut seen = HashMap::<[u8; 32], usize>::new();
    for state in index
        .files
        .values()
        .filter(|state| !state.identity_conflict)
    {
        for event in &state.events {
            if let Some(&position) = seen.get(&event.id) {
                let previous: &mut UsageEvent = &mut snapshot.events[position];
                if previous.inherited && !event.inherited {
                    *previous = event.clone();
                } else if previous.inherited == event.inherited {
                    previous.count_uncertain |= event.count_uncertain;
                    previous.count_incomplete |= event.count_incomplete;
                    previous.total.cached_known &= event.total.cached_known;
                    previous.delta.cached_known &= event.delta.cached_known;
                    if let (Some(last), Some(other)) = (&mut previous.last, event.last) {
                        last.cached_known &= other.cached_known;
                    }
                    previous.merge_duplicate_context(event, &index.provider_names);
                }
            } else {
                seen.insert(event.id, snapshot.events.len());
                snapshot.events.push(event.clone());
            }
        }
    }
    drop(seen);
    count_resolution::resolve(&index, &mut snapshot);
    let mut resolved = Vec::with_capacity(snapshot.events.len());
    for event in snapshot.events.into_iter().filter(|event| {
        event.delta.total_tokens > 0
            || event.count_uncertain
            || event.count_incomplete
            || event.context_conflicts.any()
    }) {
        let timestamp_ms = DateTime::parse_from_rfc3339(&event.timestamp)
            .context("invalid usage event timestamp")?
            .timestamp_millis();
        let conflicts = u8::from(event.context_conflicts.model)
            | u8::from(event.context_conflicts.provider) << 1
            | u8::from(event.context_conflicts.role) << 2
            | u8::from(event.context_conflicts.turn) << 3;
        resolved.push(ResolvedEvent {
            id: event.id,
            session: event.session,
            day: event.date.to_string(),
            timestamp_ms,
            model: event.model,
            provider: event.provider,
            provider_source: event.provider_source,
            role: event.role,
            context_conflicts: conflicts,
            count_uncertain: event.count_uncertain,
            count_incomplete: event.count_incomplete,
            input: event.delta.input_tokens,
            cached: event
                .delta
                .cached_known
                .then_some(event.delta.cached_input_tokens),
            output: event.delta.output_tokens,
            reasoning: event.delta.reasoning_output_tokens,
        });
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_history::structured::persistent_index::{PersistentIndex, scan};
    use chrono::NaiveDate;
    use rusqlite::params;
    use serde_json::json;

    #[test]
    fn resolution_budget_counts_strings_and_previous_session_results() {
        let directory = tempfile::tempdir().unwrap();
        let mut index = PersistentIndex::open(&directory.path().join("index.sqlite")).unwrap();
        let sessions = [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
        ];
        for session in sessions {
            let path = directory.path().join(format!("rollout-{session}.jsonl"));
            let header = json!({"type":"session_meta","payload":{
                "id":session,"model_provider":"p".repeat(20 * 1024)
            }});
            let context = json!({"type":"turn_context","timestamp":"2026-11-10T00:00:00Z",
                "payload":{"model":"test-model"}});
            let event = json!({"type":"event_msg","timestamp":"2026-11-10T00:00:00Z","payload":{
                "type":"token_count","info":{
                    "total_token_usage":{"input_tokens":1,"output_tokens":0},
                    "last_token_usage":{"input_tokens":1,"output_tokens":0}
                }
            }});
            std::fs::write(&path, format!("{header}\n{context}\n{event}\n")).unwrap();
            let path = std::fs::canonicalize(path).unwrap();
            let mut state = scan::restore_source(&index, &path).unwrap();
            assert!(
                scan::scan_source(
                    &mut index,
                    &mut state,
                    &path,
                    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                    &mut index_budget::Budget::new(std::iter::empty()),
                    |tx, sessions| resolve(tx, sessions, &BTreeMap::new()),
                )
                .unwrap()
            );
        }
        let tx = index.db.transaction().unwrap();
        for session in sessions {
            let events = resolve_with_limit(
                &tx,
                &BTreeSet::from([session.into()]),
                &BTreeMap::new(),
                112 * 1024,
            )
            .unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].provider.len(), 20 * 1024);
            assert_eq!(events[0].model, "test-model");
        }
        let error = resolve_with_limit(
            &tx,
            &sessions.map(str::to_owned).into_iter().collect(),
            &BTreeMap::new(),
            112 * 1024,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("memory budget"));
    }

    #[test]
    fn resolution_budget_failure_preserves_cursor_and_other_sources_progress() {
        let directory = tempfile::tempdir().unwrap();
        let mut index = PersistentIndex::open(&directory.path().join("index.sqlite")).unwrap();
        for (session, provider_length, succeeds) in [
            ("11111111-1111-4111-8111-111111111111", 32 * 1024, false),
            ("22222222-2222-4222-8222-222222222222", 4, true),
        ] {
            let path = directory.path().join(format!("rollout-{session}.jsonl"));
            let header = json!({"type":"session_meta","payload":{
                "id":session,"model_provider":"p".repeat(provider_length)
            }});
            let context = json!({"type":"turn_context","timestamp":"2026-11-10T00:00:00Z",
                "payload":{"model":"test-model"}});
            let event = json!({"type":"event_msg","timestamp":"2026-11-10T00:00:00Z","payload":{
                "type":"token_count","info":{
                    "total_token_usage":{"input_tokens":1,"output_tokens":0},
                    "last_token_usage":{"input_tokens":1,"output_tokens":0}
                }
            }});
            std::fs::write(&path, format!("{header}\n{context}\n{event}\n")).unwrap();
            let path = std::fs::canonicalize(path).unwrap();
            let mut state = scan::restore_source(&index, &path).unwrap();
            let result = scan::scan_source(
                &mut index,
                &mut state,
                &path,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &mut index_budget::Budget::new(std::iter::empty()),
                |tx, sessions| resolve_with_limit(tx, sessions, &BTreeMap::new(), 96 * 1024),
            );
            if succeeds {
                assert!(result.unwrap());
                assert!(state.offset > 0);
            } else {
                assert!(format!("{:#}", result.unwrap_err()).contains("memory budget"));
                assert_eq!(state.offset, 0);
                assert!(
                    index
                        .source_state(&scan::encode_source(&path))
                        .unwrap()
                        .is_none()
                );
                assert_eq!(index.revision().unwrap(), 0);
            }
        }
        let totals: (i64, i64) = index
            .db
            .query_row(
                "SELECT SUM(input), SUM(records) FROM usage_daily",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(totals, (1, 1));
    }

    #[test]
    fn invalid_checkpoint_only_blocks_related_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let mut index = PersistentIndex::open(&directory.path().join("index.sqlite")).unwrap();
        index
            .db
            .execute(
                "INSERT INTO usage_sources(source, identity, offset, parser_state, complete,
                    scope_session, scope_indexed, scope_valid, execution_indexed, execution_valid)
                 VALUES (X'61', X'01', 0, X'FF', 0, 'bad-session', 1, 1, 1, 1)",
                [],
            )
            .unwrap();
        index
            .db
            .execute(
                "INSERT INTO usage_unindexed_checkpoints(source, error) VALUES (X'61', ?1)",
                ["invalid checkpoint"],
            )
            .unwrap();
        index
            .db
            .execute(
                "INSERT INTO usage_scope_parents(source, session, parent) VALUES (X'61', ?1, ?2)",
                params!["bad-session", "parent-session"],
            )
            .unwrap();

        let tx = index.db.transaction().unwrap();
        let good = BTreeSet::from(["other-session".to_owned()]);
        assert!(resolve(&tx, &good, &BTreeMap::new()).unwrap().is_empty());
        for related in ["bad-session", "parent-session"] {
            let sessions = BTreeSet::from([related.to_owned()]);
            let error = resolve(&tx, &sessions, &BTreeMap::new()).unwrap_err();
            assert!(format!("{error:#}").contains("invalid checkpoint"));
        }
    }
}
