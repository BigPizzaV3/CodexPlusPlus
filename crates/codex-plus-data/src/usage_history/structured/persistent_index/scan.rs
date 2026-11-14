use super::{Observation, PersistentIndex, Resolution, ResolvedEvent, SourceBatch, checkpoint};
use crate::usage_history::structured::{FileState, index_budget};
use anyhow::{Context, ensure};
use chrono::{Local, NaiveDate};
use rusqlite::Transaction;
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub(in crate::usage_history::structured) fn encode_source(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;

    if let Some(path) = path.to_str() {
        let mut encoded = vec![b'V'];
        encoded.extend_from_slice(path.as_bytes());
        return encoded;
    }
    let mut encoded = vec![b'W'];
    for unit in path.as_os_str().encode_wide() {
        encoded.extend_from_slice(&unit.to_le_bytes());
    }
    encoded
}

#[cfg(unix)]
pub(in crate::usage_history::structured) fn encode_source(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    let mut encoded = vec![b'U'];
    encoded.extend_from_slice(path.as_os_str().as_bytes());
    encoded
}

#[cfg(windows)]
pub(in crate::usage_history::structured) fn decode_source(
    source: &[u8],
) -> anyhow::Result<PathBuf> {
    use anyhow::ensure;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    if source.first() == Some(&b'V') {
        return Ok(PathBuf::from(
            std::str::from_utf8(&source[1..]).context("invalid source path")?,
        ));
    }
    ensure!(
        source.first() == Some(&b'W') && source.len() % 2 == 1,
        "invalid source path"
    );
    let units: Vec<_> = source[1..]
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    Ok(PathBuf::from(OsString::from_wide(&units)))
}

#[cfg(unix)]
pub(in crate::usage_history::structured) fn decode_source(
    source: &[u8],
) -> anyhow::Result<PathBuf> {
    use anyhow::ensure;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    ensure!(source.first() == Some(&b'U'), "invalid source path");
    Ok(PathBuf::from(OsString::from_vec(source[1..].to_vec())))
}

pub(in crate::usage_history::structured) fn encode_identity(identity: (u64, u64)) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(16);
    encoded.extend_from_slice(&identity.0.to_le_bytes());
    encoded.extend_from_slice(&identity.1.to_le_bytes());
    encoded
}

pub(super) fn restore_source(index: &PersistentIndex, path: &Path) -> anyhow::Result<FileState> {
    let source = encode_source(path);
    if !index.budget_blocked {
        let paused: bool = index.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM usage_sealed_sessions
             WHERE source = ?1 AND review_reason IS NOT NULL)",
            [&source],
            |row| row.get(0),
        )?;
        if paused {
            return Err(super::SealedReviewRequired.into());
        }
    }
    match index.source_state(&source)? {
        Some(saved) => {
            ensure!(
                saved.scope_indexed && saved.execution_indexed,
                "source requires rescan to populate the new index"
            );
            let state = checkpoint::restore(&saved.parser_state)?;
            ensure!(
                state.offset == saved.offset,
                "source checkpoint offset mismatch"
            );
            if needs_inheritance_repair(&state, path)? {
                return Ok(FileState { repair_offset: Some(saved.offset), ..FileState::default() });
            }
            Ok(state)
        }
        None => Ok(FileState::default()),
    }
}

pub(super) fn needs_inheritance_repair(state: &FileState, path: &Path) -> anyhow::Result<bool> {
    if state.identity_conflict || state.history_boundary.parent.is_none()
        || state.history_boundary.start.is_some() || path.extension().is_some_and(|ext| ext == "zst")
    {
        return Ok(false);
    }
    let mut first = Vec::new();
    BufReader::new(std::fs::File::open(path)?.take(64 * 1024)).read_until(b'\n', &mut first)?;
    if first.last() != Some(&b'\n') {
        return Ok(false);
    }
    let meta: serde_json::Value = serde_json::from_slice(&first).context("cannot read usage source header")?;
    let payload = &meta["payload"];
    let session = payload["id"].as_str().and_then(crate::usage_history::normalize_session_id);
    Ok(meta["type"] == "session_meta" && session.as_deref() == Some(&state.session)
        && crate::usage_history::UsageHistoryBoundary::from_meta(
            payload, &state.session, meta["ordinal"].as_u64(),
        ).parent.is_none())
}

pub(super) fn scan_source(
    index: &mut PersistentIndex,
    state: &mut FileState,
    path: &Path,
    cutoff: NaiveDate,
    budget: &mut index_budget::Budget,
    resolve: impl FnOnce(&Transaction<'_>, &BTreeSet<String>) -> anyhow::Result<Vec<ResolvedEvent>>,
) -> anyhow::Result<bool> {
    scan_source_resolved(index, state, path, cutoff, budget, |tx, sessions, _| {
        resolve(tx, sessions).map(Resolution::Full)
    })
}

pub(super) fn scan_source_resolved(
    index: &mut PersistentIndex,
    state: &mut FileState,
    path: &Path,
    cutoff: NaiveDate,
    budget: &mut index_budget::Budget,
    resolve: impl FnOnce(
        &Transaction<'_>,
        &BTreeSet<String>,
        &SourceBatch,
    ) -> anyhow::Result<Resolution>,
) -> anyhow::Result<bool> {
    let repair_offset = state.repair_offset;
    ensure!(
        repair_offset.is_some() || (state.events.is_empty()
            && state.event_positions.is_empty()
            && state.execution_observations.is_empty()),
        "uncommitted source observations"
    );
    let source = encode_source(path);
    let previous = index.source_state(&source)?;
    let expected_offset = previous.as_ref().map_or(0, |saved| saved.offset);
    ensure!(
        repair_offset.unwrap_or_else(|| checkpoint::committed_offset(state)) == expected_offset,
        "source state is not at the committed offset"
    );
    let saved = checkpoint::save(state)?;
    let result = (|| {
        let old_stamp = state.stamp;
        let was_invalid = state.invalid_counts;
        let read_before = state.bytes_read;
        state.capture_positions = true;
        let completed = state.read_with_budget(path, cutoff, budget)?;
        if repair_offset.is_some() && !completed {
            ensure!(state.bytes_read != read_before || budget.read_left == 0 || budget.source_left == 0,
                "usage inheritance replay is blocked by an incomplete line or the scan budget");
            // Do not publish a partially replayed source or advance its committed cursor.
            state.repair_offset = repair_offset;
            return Ok(false);
        }
        let Some(stamp) = state.stamp else {
            return Ok(completed);
        };
        let identity = encode_identity(stamp.2.context("source identity is unavailable")?);
        let compressed = path.extension().is_some_and(|extension| extension == "zst");
        if repair_offset.is_some() {
            ensure!(previous.as_ref().is_some_and(|saved| saved.identity == identity)
                && checkpoint::committed_offset(state) >= expected_offset
                && state.history_boundary.parent.is_none()
                && state.issues == 0 && !state.execution_incomplete,
                "cannot replace the old usage checkpoint with an incomplete replay");
        }
        let replace = repair_offset.is_some() || previous.is_some()
            && (state.identity_conflict
                || previous
                    .as_ref()
                    .is_some_and(|saved| saved.identity != identity)
                || old_stamp.is_some_and(|old| {
                    old != stamp
                        && (compressed
                            || was_invalid
                            || old.0 > stamp.0
                            || old.2 != stamp.2
                            || old.0 == stamp.0 && old.1 != stamp.1)
                }));
        let events = std::mem::take(&mut state.events);
        let positions = std::mem::take(&mut state.event_positions);
        let execution_observations = std::mem::take(&mut state.execution_observations);
        ensure!(
            events.len() == positions.len(),
            "usage event position count mismatch"
        );
        let observations = if state.identity_conflict {
            Vec::new()
        } else {
            events
                .iter()
                .zip(positions)
                .map(|(event, position)| {
                    Ok(Observation {
                        position,
                        event_id: event.id,
                        session: event.session.clone(),
                        payload: checkpoint::save_event(event)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?
        };
        if !replace
            && previous.as_ref().is_some_and(|saved| {
                saved.offset == checkpoint::committed_offset(state) && saved.complete == completed
            })
            && observations.is_empty()
            && execution_observations.is_empty()
        {
            return Ok(completed);
        }
        let scope_valid =
            !state.identity_conflict && !state.session.is_empty() && state.issues == 0;
        let mut scope_parents = Vec::new();
        if scope_valid {
            for (session, parents) in &state.scope_parent_candidates {
                for parent in parents {
                    scope_parents.push((session.clone(), parent.clone()));
                }
            }
            scope_parents.sort();
        }
        let mut scope_aliases = Vec::new();
        if scope_valid {
            if let Some(alias) = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.strip_suffix(".zst").unwrap_or(name))
                .and_then(|name| name.strip_suffix(".jsonl"))
                .and_then(|name| name.get(name.len().saturating_sub(36)..))
                .and_then(super::super::super::normalize_session_id)
            {
                scope_aliases.push((alias, state.session.clone()));
            }
        }
        let mut batch = SourceBatch {
            source,
            identity,
            expected_offset,
            next_offset: checkpoint::committed_offset(state),
            parser_state: checkpoint::save(state)?,
            complete: completed,
            usage_cutoff: cutoff,
            replace,
            observations,
            execution_observations: if state.identity_conflict {
                Vec::new()
            } else {
                execution_observations
            },
            scope_session: state.session.clone(),
            scope_valid,
            scope_parents,
            scope_aliases,
            execution_valid: scope_valid && !state.execution_incomplete,
            count_gap_sessions: (state.unlocalized_invalid_counts || state.pending_count_gap)
                .then(|| {
                    std::iter::once(&state.session)
                        .chain(state.inherited_contexts.iter().map(|context| &context.session))
                        .filter(|session| !session.is_empty())
                        .cloned()
                        .collect()
                }),
            now_ms: Local::now().timestamp_millis(),
        };
        index.commit_batch_resolved(&mut batch, |tx, sessions, batch| {
            let events = resolve(tx, sessions, batch)?;
            let after = std::fs::symlink_metadata(path)?;
            ensure!(after.is_file(), "source changed while scanning");
            ensure!(
                std::fs::canonicalize(path)? == path
                    && super::super::observed_file_identity(path, &after)? == stamp.2,
                "source identity changed while scanning"
            );
            ensure!(
                after.len() == stamp.0 && after.modified().ok() == stamp.1,
                "source changed while scanning"
            );
            Ok(events)
        })?;
        state.repair_offset = None;
        Ok(completed)
    })();
    match result {
        Ok(completed) => Ok(completed),
        Err(error) if repair_offset.is_some() => {
            *state = FileState { repair_offset, ..FileState::default() };
            Err(error)
        }
        Err(error) => {
            *state = FileState::default();
            match checkpoint::restore(&saved) {
                Ok(restored) => {
                    *state = restored;
                    Err(error)
                }
                Err(restore_error) => Err(error.context(format!(
                    "source checkpoint restore failed: {restore_error:#}"
                ))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::io::Write;

    const SESSION: &str = "00000000-0000-4000-8000-000000000010";

    #[test]
    fn inherited_source_replay_keeps_old_checkpoint_until_atomic_replacement() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("usage.sqlite");
        let path = root.path().join("rollout-child.jsonl");
        let parent = "00000000-0000-4000-8000-000000000011";
        let time = Local::now().to_rfc3339();
        let cutoff = Local::now().date_naive();
        let mut header = json!({"type":"session_meta","ordinal":0,"payload":{
            "id":SESSION,"parent_thread_id":parent,"model_provider":"provider-a",
            "source":{"subagent":{"thread_spawn":{"parent_thread_id":SESSION}}}
        }});
        let context = json!({"type":"turn_context","ordinal":1,"timestamp":time,
            "payload":{"model":"test-model"}});
        let event = json!({"type":"event_msg","ordinal":2,"timestamp":time,"payload":{
            "type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":1},
            "last_token_usage":{"input_tokens":100,"output_tokens":1}}
        }});
        std::fs::write(&path, format!("{header}\n{context}\n{event}\n")).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let key = encode_source(&path);
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        let resolve = |tx: &Transaction<'_>, sessions: &BTreeSet<String>| {
            super::super::resolve::resolve(tx, sessions, &BTreeMap::new())
        };
        let budget = || index_budget::Budget::new(std::iter::empty());
        assert!(scan_source(&mut index, &mut state, &path, cutoff, &mut budget(), resolve).unwrap());
        let old = index.source_state(&key).unwrap().unwrap();
        let revision = index.revision().unwrap();
        header["payload"]["source"]["subagent"]["thread_spawn"]["parent_thread_id"] = json!(parent);
        std::fs::write(&path, format!("{header}\n{context}\n{event}\n")).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert_eq!(state.repair_offset, Some(old.offset));
        let mut small = budget();
        small.read_left = 100;
        small.source_left = 100;
        assert!(!scan_source(&mut index, &mut state, &path, cutoff, &mut small, resolve).unwrap());
        assert_eq!(index.source_state(&key).unwrap().unwrap(), old);
        assert_eq!(index.revision().unwrap(), revision);
        drop(state);
        drop(index);
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert!(scan_source(&mut index, &mut state, &path, cutoff, &mut budget(), |_, _| {
            anyhow::bail!("synthetic commit failure")
        }).is_err());
        assert_eq!(index.source_state(&key).unwrap().unwrap(), old);
        assert!(scan_source(&mut index, &mut state, &path, cutoff, &mut budget(), resolve).unwrap());
        assert_eq!(state.repair_offset, None);
        let row = index.db.query_row(
            "SELECT count(*), sum(input), min(model), min(provider), sum(count_incomplete) FROM usage_events",
            [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?)),
        ).unwrap();
        assert_eq!(row, (1, 100, "test-model".into(), "provider-a".into(), 0));
        let state = restore_source(&index, &path).unwrap();
        assert_eq!(state.repair_offset, None);
        assert_eq!(index.revision().unwrap(), revision + 1);
    }

    fn usage(input: u64) -> String {
        json!({
            "type":"event_msg",
            "timestamp":format!("2026-11-10T00:00:{input:02}Z"),
            "payload":{"type":"token_count","info":{
                "total_token_usage":{"input_tokens":input,"output_tokens":1}
            }}
        })
        .to_string()
    }

    fn resolve(
        tx: &Transaction<'_>,
        sessions: &BTreeSet<String>,
    ) -> anyhow::Result<Vec<ResolvedEvent>> {
        let mut events = BTreeMap::new();
        for session in sessions {
            let mut statement = tx.prepare(
                "SELECT event_id, payload FROM usage_observations
                 WHERE session = ?1 ORDER BY source, position",
            )?;
            let mut rows = statement.query([session])?;
            while let Some(row) = rows.next()? {
                let id: Vec<u8> = row.get(0)?;
                let id: [u8; 32] = id.try_into().map_err(|_| anyhow::anyhow!("invalid id"))?;
                let payload: Vec<u8> = row.get(1)?;
                let event = checkpoint::restore_event(id, session.clone(), &payload)?;
                events.entry(id).or_insert(ResolvedEvent {
                    id,
                    session: session.clone(),
                    day: event.date.to_string(),
                    timestamp_ms: chrono::DateTime::parse_from_rfc3339(&event.timestamp)?
                        .timestamp_millis(),
                    model: event.model,
                    provider: event.provider,
                    provider_source: event.provider_source,
                    role: event.role,
                    context_conflicts: 0,
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
        }
        Ok(events.into_values().collect())
    }

    fn scan(
        index: &mut PersistentIndex,
        state: &mut FileState,
        path: &Path,
    ) -> anyhow::Result<bool> {
        let mut budget = index_budget::Budget::new(std::iter::empty());
        scan_source(
            index,
            state,
            path,
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            &mut budget,
            resolve,
        )
    }

    fn total(index: &PersistentIndex) -> i64 {
        index
            .db
            .query_row("SELECT SUM(input) FROM usage_daily", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn source_and_identity_preserve_native_values() {
        let path = PathBuf::from("session-\u{4e2d}\u{6587}.jsonl");
        assert_eq!(decode_source(&encode_source(&path)).unwrap(), path);
        assert!(decode_source(b"invalid").is_err());
        assert_eq!(
            encode_identity((0x0102_0304_0506_0708, 0x1112_1314_1516_1718)),
            [8, 7, 6, 5, 4, 3, 2, 1, 24, 23, 22, 21, 20, 19, 18, 17]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_source_round_trips_unpaired_surrogate() {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let original = PathBuf::from(OsString::from_wide(&[b'C' as u16, 0xd800, b'x' as u16]));
        let decoded = decode_source(&encode_source(&original)).unwrap();
        assert_eq!(
            original.as_os_str().encode_wide().collect::<Vec<_>>(),
            decoded.as_os_str().encode_wide().collect::<Vec<_>>()
        );
        assert!(decode_source(&[b'W', 0]).is_err());
    }

    #[test]
    fn actual_jsonl_resumes_after_restart_and_failed_commit() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-test.jsonl");
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        std::fs::write(&path, format!("{header}\n{}\n", usage(10))).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let database = root.path().join("usage.sqlite");
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert!(scan(&mut index, &mut state, &path).unwrap());
        assert_eq!(total(&index), 10);
        assert!(state.events.is_empty());
        let first_offset = state.offset;
        drop(state);
        drop(index);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", usage(20)).unwrap();
        drop(file);
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert_eq!(state.offset, first_offset);
        let mut budget = index_budget::Budget::new(std::iter::empty());
        assert!(
            scan_source(
                &mut index,
                &mut state,
                &path,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &mut budget,
                |_, _| anyhow::bail!("synthetic commit failure"),
            )
            .is_err()
        );
        assert_eq!(state.offset, first_offset);
        assert_eq!(total(&index), 10);
        assert!(scan(&mut index, &mut state, &path).unwrap());
        assert_eq!(total(&index), 20);
        let offsets: Vec<i64> = index
            .db
            .prepare("SELECT position FROM usage_observations ORDER BY position")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            offsets,
            vec![format!("{header}\n").len() as i64, first_offset as i64]
        );
        assert_eq!(index.revision().unwrap(), 2);
        assert!(scan(&mut index, &mut state, &path).unwrap());
        assert_eq!(index.revision().unwrap(), 2);
    }

    #[test]
    fn replacement_rebuilds_only_the_source_and_keeps_cursor_atomic() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-test.jsonl");
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        std::fs::write(&path, format!("{header}\n{}\n{}\n", usage(10), usage(20))).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert!(scan(&mut index, &mut state, &path).unwrap());
        assert_eq!(total(&index), 20);
        std::fs::write(&path, format!("{header}\n{}\n", usage(15))).unwrap();
        assert!(scan(&mut index, &mut state, &path).unwrap());
        assert_eq!(total(&index), 15);
        assert_eq!(index.revision().unwrap(), 2);
        let observations: usize = index
            .db
            .query_row("SELECT COUNT(*) FROM usage_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(observations, 1);
    }

    #[test]
    fn source_rewritten_during_commit_cannot_advance_cursor() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-test.jsonl");
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        std::fs::write(&path, format!("{header}\n{}\n", usage(10))).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert!(scan(&mut index, &mut state, &path).unwrap());
        let previous_offset = state.offset;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", usage(20)).unwrap();
        drop(file);

        let mut budget = index_budget::Budget::new(std::iter::empty());
        assert!(
            scan_source(
                &mut index,
                &mut state,
                &path,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &mut budget,
                |tx, sessions| {
                    let events = resolve(tx, sessions)?;
                    std::fs::write(&path, b"short")?;
                    Ok(events)
                }
            )
            .is_err()
        );
        assert_eq!(state.offset, previous_offset);
        assert_eq!(total(&index), 10);
        assert_eq!(index.revision().unwrap(), 1);
    }

    #[test]
    fn compressed_source_resumes_without_replaying_committed_events() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-test.jsonl.zst");
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        let filler = format!(
            "{}\n",
            json!({"type":"response_item","payload":{"type":"message","content":"x".repeat(1024)}})
        );
        let content = format!(
            "{header}\n{}\n{}{}\n",
            usage(10),
            filler.repeat(4300),
            usage(20)
        );
        std::fs::write(
            &path,
            zstd::stream::encode_all(content.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let database = root.path().join("usage.sqlite");
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert!(!scan(&mut index, &mut state, &path).unwrap());
        assert_eq!(total(&index), 10);
        drop(state);
        drop(index);

        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        let mut done = false;
        for _ in 0..4 {
            if scan(&mut index, &mut state, &path).unwrap() {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(total(&index), 20);
        let observations: usize = index
            .db
            .query_row("SELECT COUNT(*) FROM usage_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(observations, 2);
    }

    #[test]
    fn tools_scope_and_usage_commit_together_and_failed_retry_replays_the_line() {
        let root = tempfile::tempdir().unwrap();
        let alias = "00000000-0000-4000-8000-000000000011";
        let parent = "00000000-0000-4000-8000-000000000012";
        let path = root.path().join(format!("rollout-{alias}.jsonl"));
        let header = json!({"type":"session_meta","payload":{
            "id":SESSION,"source":{"subagent":{"thread_spawn":{"parent_thread_id":parent}}}
        }});
        let timestamp = Local::now().to_rfc3339();
        let call = json!({"type":"response_item","timestamp":timestamp,"payload":{
            "type":"function_call","call_id":"call-1","name":"functions.exec_command",
            "arguments":"private command"
        }});
        std::fs::write(&path, format!("{header}\n{}\n{call}\n", usage(10))).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let database = root.path().join("usage.sqlite");
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert!(scan(&mut index, &mut state, &path).unwrap());
        assert_eq!(total(&index), 10);
        let parent_row: (String, String) = index
            .db
            .query_row(
                "SELECT session, parent FROM usage_scope_parents",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(parent_row, (SESSION.into(), parent.into()));
        let alias_row: (String, String) = index
            .db
            .query_row(
                "SELECT alias, session FROM usage_scope_aliases",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(alias_row, (alias.into(), SESSION.into()));
        let first_offset = state.offset;
        let output = json!({"type":"response_item","timestamp":timestamp,"payload":{
            "type":"function_call_output","call_id":"call-1","output":"private output"
        }});
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{output}").unwrap();
        drop(file);
        let mut budget = index_budget::Budget::new(std::iter::empty());
        assert!(
            scan_source(
                &mut index,
                &mut state,
                &path,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &mut budget,
                |_, _| anyhow::bail!("synthetic commit failure"),
            )
            .is_err()
        );
        assert_eq!(state.offset, first_offset);
        let count: i64 = index
            .db
            .query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        assert!(scan(&mut index, &mut state, &path).unwrap());
        let rows: Vec<(i64, i64)> = index
            .db
            .prepare("SELECT start, end FROM execution_observations ORDER BY start")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            String::from_utf8_lossy(&bytes[rows[0].0 as usize..rows[0].1 as usize])
                .contains("private command")
        );
        assert!(
            String::from_utf8_lossy(&bytes[rows[1].0 as usize..rows[1].1 as usize])
                .contains("private output")
        );
        let checkpoint: Vec<u8> = index
            .db
            .query_row("SELECT parser_state FROM usage_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(!String::from_utf8_lossy(&checkpoint).contains("private"));
    }

    #[test]
    fn incomplete_tool_line_is_replayed_after_reopen_without_persisting_its_body() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-test.jsonl");
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        let timestamp = Local::now().to_rfc3339();
        let call = json!({"type":"response_item","timestamp":timestamp,"payload":{
            "type":"function_call","call_id":"call-1","name":"functions.exec_command",
            "arguments":"private unfinished command"
        }})
        .to_string();
        let split = call.len() - 2;
        std::fs::write(&path, format!("{header}\n{}", &call[..split])).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let database = root.path().join("usage.sqlite");
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert!(!scan(&mut index, &mut state, &path).unwrap());
        let checkpoint = index.source_state(&encode_source(&path)).unwrap().unwrap();
        assert_eq!(checkpoint.offset, format!("{header}\n").len() as u64);
        assert!(!String::from_utf8_lossy(&checkpoint.parser_state).contains("private"));
        drop(state);
        drop(index);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", &call[split..]).unwrap();
        drop(file);
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert_eq!(state.offset, checkpoint.offset);
        assert!(state.needs_resume);
        assert!(scan(&mut index, &mut state, &path).unwrap());
        let count: i64 = index
            .db
            .query_row(
                "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'call-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn persisted_resolution_matches_existing_snapshot_for_copies_and_turn_collisions() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        let home = std::fs::canonicalize(home).unwrap();
        let settings = root.path().join("settings.json");
        let log = root.path().join("requests.jsonl");
        std::fs::write(
            &settings,
            br#"{"usageHistoryEnabled":true,"relayProfiles":[]}"#,
        )
        .unwrap();
        std::fs::write(&log, b"").unwrap();
        let base = Local::now().timestamp_millis() - 60_000;
        let at = |offset| {
            chrono::DateTime::from_timestamp_millis(base + offset)
                .unwrap()
                .to_rfc3339()
        };
        let head = json!({"type":"session_meta","payload":{"id":SESSION}});
        let context = |turn| {
            json!({
                "type":"turn_context","timestamp":at(0),
                "payload":{"model":"test-model","turn_id":turn}
            })
        };
        let usage_row = |offset, input, last| {
            let tokens = |value: u64| {
                json!({
                    "input_tokens":value,"output_tokens":value/10,
                    "cached_input_tokens":value/5,"reasoning_output_tokens":0
                })
            };
            json!({"type":"event_msg","timestamp":at(offset),"payload":{
                "type":"token_count","info":{
                    "total_token_usage":tokens(input),"last_token_usage":tokens(last)
                }
            }})
        };
        let first = usage_row(1_000, 100, 100);
        let second = usage_row(2_000, 200, 100);
        let mut correction = second.clone();
        correction["timestamp"] = json!(at(3_000));
        correction["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(80);
        let paths = [
            (
                "rollout-a.jsonl",
                vec![
                    head.clone(),
                    context("first"),
                    first.clone(),
                    second.clone(),
                    correction.clone(),
                ],
            ),
            (
                "rollout-b.jsonl",
                vec![head.clone(), context("first"), first, second, correction],
            ),
            (
                "rollout-c.jsonl",
                vec![
                    head.clone(),
                    context("different-turn"),
                    usage_row(4_000, 300, 300),
                ],
            ),
            (
                "rollout-d.jsonl",
                vec![head, context("another-turn"), usage_row(4_000, 300, 300)],
            ),
        ];
        let mut source_paths = Vec::new();
        for (name, rows) in paths {
            let path = home.join("sessions").join(name);
            let contents = rows
                .into_iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>();
            std::fs::write(&path, contents).unwrap();
            source_paths.push(std::fs::canonicalize(path).unwrap());
        }
        let mut original = super::super::super::Index {
            settings_path: Some(settings),
            storage_dir: Some(root.path().join("cache")),
            ..Default::default()
        };
        let snapshot = original.update_with_request_log(&home, &log);
        assert_eq!(snapshot.errors, 0);
        let expected: BTreeMap<_, _> = snapshot
            .events
            .iter()
            .map(|event| {
                (
                    event.id.to_vec(),
                    (
                        event.delta.input_tokens,
                        event
                            .delta
                            .cached_known
                            .then_some(event.delta.cached_input_tokens),
                        event.delta.output_tokens,
                        event.delta.reasoning_output_tokens,
                        event.count_uncertain,
                        event.count_incomplete,
                        event.model.clone(),
                        event.provider.clone(),
                    ),
                )
            })
            .collect();
        let mut persistent = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        for path in source_paths {
            let mut state = restore_source(&persistent, &path).unwrap();
            let mut budget = index_budget::Budget::new(std::iter::empty());
            assert!(
                scan_source(
                    &mut persistent,
                    &mut state,
                    &path,
                    Local::now().date_naive() - chrono::Duration::days(364),
                    &mut budget,
                    |tx, sessions| super::super::resolve::resolve(tx, sessions, &BTreeMap::new()),
                )
                .unwrap()
            );
        }
        let actual: BTreeMap<
            Vec<u8>,
            (
                u64,
                Option<u64>,
                u64,
                Option<u64>,
                bool,
                bool,
                String,
                String,
            ),
        > = persistent
            .db
            .prepare(
                "SELECT id, input, cached, output, reasoning, count_uncertain, count_incomplete,
                    model, provider FROM usage_events",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    (
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ),
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn more_than_one_tool_batch_does_not_drop_or_duplicate_calls() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-test.jsonl");
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        let timestamp = Local::now().to_rfc3339();
        let mut contents = format!("{header}\n");
        for id in 0..1_050 {
            let call = json!({"type":"response_item","timestamp":timestamp,"payload":{
                "type":"function_call","call_id":format!("call-{id}"),"name":"functions.exec_command"
            }});
            contents.push_str(&format!("{call}\n"));
        }
        std::fs::write(&path, contents).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert!(!scan(&mut index, &mut state, &path).unwrap());
        let count: i64 = index
            .db
            .query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1_024);
        assert!(scan(&mut index, &mut state, &path).unwrap());
        let count: i64 = index
            .db
            .query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1_050);
    }

    #[test]
    fn unclosed_count_gap_recomputes_existing_events_and_rollback_keeps_cursor() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-gap.jsonl");
        let at = Local::now().to_rfc3339();
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        let usage_row = |input: u64| {
            json!({"type":"event_msg","timestamp":at,"payload":{
                "type":"token_count","info":{
                    "total_token_usage":{"input_tokens":input,"output_tokens":input / 10},
                    "last_token_usage":{"input_tokens":100,"output_tokens":10}
                }
            }})
        };
        let mut invalid = usage_row(200);
        invalid["payload"]["info"]["total_token_usage"]
            .as_object_mut()
            .unwrap()
            .remove("input_tokens");
        std::fs::write(&path, format!("{header}\n{}\n", usage_row(100))).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let scan = |index: &mut PersistentIndex, state: &mut FileState| {
            scan_source(
                index,
                state,
                &path,
                cutoff,
                &mut index_budget::Budget::new(std::iter::empty()),
                |tx, sessions| super::super::resolve::resolve(tx, sessions, &BTreeMap::new()),
            )
        };
        let daily = |index: &PersistentIndex| -> (i64, i64) {
            index
                .db
                .query_row(
                    "SELECT input, uncertain_events FROM usage_daily WHERE session = ?1",
                    [SESSION],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        };
        assert!(scan(&mut index, &mut state).unwrap());
        assert_eq!(daily(&index), (100, 0));
        let offset = state.offset;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{invalid}").unwrap();
        drop(file);
        assert!(
            scan_source(
                &mut index,
                &mut state,
                &path,
                cutoff,
                &mut index_budget::Budget::new(std::iter::empty()),
                |_, _| anyhow::bail!("synthetic failure"),
            )
            .is_err()
        );
        assert_eq!(state.offset, offset);
        assert_eq!(daily(&index), (100, 0));
        assert!(scan(&mut index, &mut state).unwrap());
        assert_eq!(daily(&index), (0, 1));
        drop(state);
        drop(index);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{}", usage_row(300)).unwrap();
        drop(file);
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        assert!(scan(&mut index, &mut state).unwrap());
        assert_eq!(daily(&index), (100, 1));
    }

    #[test]
    fn gap_only_source_marks_related_inherited_session_but_not_other_sessions() {
        let root = tempfile::tempdir().unwrap();
        let parent = SESSION;
        let child = "00000000-0000-4000-8000-000000000011";
        let other = "00000000-0000-4000-8000-000000000012";
        let at = Local::now().to_rfc3339();
        let usage_row = json!({"type":"event_msg","timestamp":at,"payload":{
            "type":"token_count","info":{
                "total_token_usage":{"input_tokens":100,"output_tokens":10},
                "last_token_usage":{"input_tokens":100,"output_tokens":10}
            }
        }});
        let mut invalid = usage_row.clone();
        invalid["ordinal"] = json!(1);
        invalid["payload"]["info"]["total_token_usage"]
            .as_object_mut()
            .unwrap()
            .remove("input_tokens");
        let child_header = json!({"type":"session_meta","payload":{
            "id":child,"forked_from_id":parent,"subagent_history_start_ordinal":3
        }});
        let inherited_context = json!({"type":"turn_context","ordinal":1,"timestamp":at,
            "payload":{"model":"test-model"}});
        let files = [
            (
                "rollout-parent.jsonl",
                format!(
                    "{}\n{usage_row}\n",
                    json!({"type":"session_meta","payload":{"id":parent}})
                ),
            ),
            (
                "rollout-other.jsonl",
                format!(
                    "{}\n{usage_row}\n",
                    json!({"type":"session_meta","payload":{"id":other}})
                ),
            ),
            (
                "rollout-child.jsonl",
                format!("{child_header}\n{inherited_context}\n{invalid}\n"),
            ),
        ];
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        for (name, contents) in files {
            let path = root.path().join(name);
            std::fs::write(&path, contents).unwrap();
            let path = std::fs::canonicalize(path).unwrap();
            let mut state = restore_source(&index, &path).unwrap();
            assert!(
                scan_source(
                    &mut index,
                    &mut state,
                    &path,
                    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                    &mut index_budget::Budget::new(std::iter::empty()),
                    |tx, sessions| {
                        super::super::resolve::resolve(tx, sessions, &BTreeMap::new())
                    },
                )
                .unwrap()
            );
        }
        let rows: Vec<(String, i64, i64)> = index
            .db
            .prepare("SELECT session, input, uncertain_events FROM usage_daily ORDER BY session")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![(parent.into(), 0, 1), (other.into(), 100, 0)]
        );
    }

    #[test]
    fn incremental_results_match_full_resolution_after_retry_and_restart() {
        use super::super::append::Appender;
        use std::cell::Cell;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-test.jsonl");
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        let row = |at: &str, input: u64, cached: u64, include_last: bool| {
            let mut info = json!({
                "total_token_usage":{
                    "input_tokens":input,"cached_input_tokens":cached,"output_tokens":input / 10
                }
            });
            if include_last {
                info["last_token_usage"] = json!({
                    "input_tokens":10,"cached_input_tokens":1,"output_tokens":1
                });
            }
            json!({"type":"event_msg","timestamp":at,"payload":{"type":"token_count","info":info}})
        };
        std::fs::write(
            &path,
            format!(
                "{header}\n{}\n{}\n",
                row("2026-11-10T00:00:01Z", 10, 1, false),
                row("2026-11-10T00:00:02Z", 20, 2, true)
            ),
        )
        .unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let fast_database = root.path().join("fast.sqlite");
        let full_database = root.path().join("full.sqlite");
        let mut fast = PersistentIndex::open(&fast_database).unwrap();
        let mut full = PersistentIndex::open(&full_database).unwrap();
        let mut fast_state = restore_source(&fast, &path).unwrap();
        let mut full_state = restore_source(&full, &path).unwrap();
        let mut appender = Appender::default();
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let names = BTreeMap::new();
        let snapshot = |index: &PersistentIndex| {
            index
                .db
                .prepare(
                    "SELECT hex(id), session, day, timestamp_ms, model, provider, provider_source,
                            role, context_conflicts, count_uncertain, count_incomplete,
                            input, cached, output, reasoning
                     FROM usage_events ORDER BY timestamp_ms, id",
                )
                .unwrap()
                .query_map([], |row| {
                    Ok(json!([
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, bool>(9)?,
                        row.get::<_, bool>(10)?,
                        row.get::<_, i64>(11)?,
                        row.get::<_, Option<i64>>(12)?,
                        row.get::<_, i64>(13)?,
                        row.get::<_, Option<i64>>(14)?,
                    ]))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let daily_snapshot = |index: &PersistentIndex| {
            index
                .db
                .prepare(
                    "SELECT day, session, model, provider, provider_source, role,
                            context_conflicts, input, cached, cached_known_sum, output,
                            reasoning, records, uncertain_events, total_events
                     FROM usage_daily
                     ORDER BY day, session, model, provider, provider_source, role,
                              context_conflicts",
                )
                .unwrap()
                .query_map([], |row| {
                    Ok(json!([
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                        row.get::<_, Option<i64>>(11)?,
                        row.get::<_, i64>(12)?,
                        row.get::<_, i64>(13)?,
                        row.get::<_, i64>(14)?,
                    ]))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let scan_fast = |index: &mut PersistentIndex,
                         state: &mut FileState,
                         appender: &mut Appender,
                         fail: bool|
         -> anyhow::Result<bool> {
            let used_append = Cell::new(false);
            scan_source_resolved(
                index,
                state,
                &path,
                cutoff,
                &mut index_budget::Budget::new(std::iter::empty()),
                |tx, sessions, batch| {
                    let resolution = appender.resolve(tx, sessions, batch, &names)?;
                    used_append.set(matches!(resolution, Resolution::Append(_)));
                    if fail {
                        anyhow::bail!("synthetic failure after resolution");
                    }
                    Ok(resolution)
                },
            )?;
            Ok(used_append.get())
        };
        let scan_full = |index: &mut PersistentIndex, state: &mut FileState| {
            scan_source(
                index,
                state,
                &path,
                cutoff,
                &mut index_budget::Budget::new(std::iter::empty()),
                |tx, sessions| super::super::resolve::resolve(tx, sessions, &names),
            )
            .unwrap()
        };

        assert!(scan_fast(&mut fast, &mut fast_state, &mut appender, false).unwrap());
        assert!(scan_full(&mut full, &mut full_state));
        assert_eq!(snapshot(&fast), snapshot(&full));
        assert_eq!(daily_snapshot(&fast), daily_snapshot(&full));
        assert_eq!(snapshot(&fast)[0][10], true);

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", row("2026-11-10T00:00:03Z", 30, 3, true)).unwrap();
        drop(file);
        assert!(scan_fast(&mut fast, &mut fast_state, &mut appender, true).is_err());
        assert_eq!(snapshot(&fast), snapshot(&full));
        assert_eq!(daily_snapshot(&fast), daily_snapshot(&full));
        assert!(scan_fast(&mut fast, &mut fast_state, &mut appender, false).unwrap());
        assert!(scan_full(&mut full, &mut full_state));
        assert_eq!(snapshot(&fast), snapshot(&full));
        assert_eq!(daily_snapshot(&fast), daily_snapshot(&full));

        drop(fast_state);
        drop(fast);
        let mut fast = PersistentIndex::open(&fast_database).unwrap();
        let mut fast_state = restore_source(&fast, &path).unwrap();
        let mut appender = Appender::default();
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", row("2026-11-10T00:00:04Z", 40, 4, true)).unwrap();
        drop(file);
        assert!(scan_fast(&mut fast, &mut fast_state, &mut appender, false).unwrap());
        assert!(scan_full(&mut full, &mut full_state));
        assert_eq!(snapshot(&fast), snapshot(&full));
        assert_eq!(daily_snapshot(&fast), daily_snapshot(&full));

        for event in [
            row("2026-11-10T00:00:05Z", 40, 2, true),
            row("2026-11-10T00:00:04Z", 40, 4, true),
            row("2026-11-10T00:00:03.500Z", 35, 3, true),
        ] {
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(file, "{event}").unwrap();
            drop(file);
            assert!(!scan_fast(&mut fast, &mut fast_state, &mut appender, false).unwrap());
            assert!(scan_full(&mut full, &mut full_state));
            assert_eq!(snapshot(&fast), snapshot(&full));
            assert_eq!(daily_snapshot(&fast), daily_snapshot(&full));
        }
    }

    #[test]
    fn incremental_resolution_falls_back_for_another_source() {
        use super::super::append::Appender;

        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("rollout-first.jsonl");
        let second = root.path().join("rollout-second.jsonl");
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        let event = json!({"type":"event_msg","timestamp":"2026-11-10T00:00:01Z","payload":{
            "type":"token_count","info":{
                "total_token_usage":{"input_tokens":10,"output_tokens":1},
                "last_token_usage":{"input_tokens":10,"output_tokens":1}
            }
        }});
        for path in [&first, &second] {
            std::fs::write(path, format!("{header}\n{event}\n")).unwrap();
        }
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        let mut appender = Appender::default();
        for (path, expected_append) in [(&first, true), (&second, false)] {
            let path = std::fs::canonicalize(path).unwrap();
            let mut state = restore_source(&index, &path).unwrap();
            let mut appended = false;
            assert!(
                scan_source_resolved(
                    &mut index,
                    &mut state,
                    &path,
                    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                    &mut index_budget::Budget::new(std::iter::empty()),
                    |tx, sessions, batch| {
                        let result = appender.resolve(tx, sessions, batch, &BTreeMap::new())?;
                        appended = matches!(result, Resolution::Append(_));
                        Ok(result)
                    },
                )
                .unwrap()
            );
            assert_eq!(appended, expected_append);
        }
        let totals: (i64, i64) = index
            .db
            .query_row(
                "SELECT SUM(input), SUM(records) FROM usage_daily",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(totals, (10, 1));
    }

    #[test]
    fn first_source_batch_can_exceed_full_resolution_limit() {
        use super::super::append::Appender;
        use std::fmt::Write as _;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-large.jsonl");
        let mut contents =
            format!("{}\n", json!({"type":"session_meta","payload":{"id":SESSION}}));
        let start = chrono::DateTime::parse_from_rfc3339("2026-11-10T00:00:00Z").unwrap();
        for input in 1..=25_001 {
            let time = start + chrono::Duration::milliseconds(input - 1);
            let at = match input {
                1 => time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                3 => time
                    .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
                _ => time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            };
            let row = json!({"type":"event_msg","timestamp":at,"payload":{
                "type":"token_count","info":{"total_token_usage":{
                    "input_tokens":input,"output_tokens":0
                }}
            }});
            writeln!(contents, "{row}").unwrap();
        }
        assert!(contents.len() < 4 * 1024 * 1024);
        std::fs::write(&path, contents).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        index
            .db
            .execute_batch(
                "INSERT INTO usage_sources(source, identity, offset, parser_state, complete,
                     scope_session, scope_indexed, scope_valid, execution_indexed, execution_valid)
                 VALUES (X'61', X'01', 0, X'FF', 0, 'unrelated-session', 1, 1, 1, 1);
                 INSERT INTO usage_unindexed_checkpoints(source, error)
                 VALUES (X'61', 'invalid unrelated checkpoint');",
            )
            .unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        let mut appender = Appender::default();
        let mut appended = false;
        assert!(
            scan_source_resolved(
                &mut index,
                &mut state,
                &path,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &mut index_budget::Budget::new(std::iter::empty()),
                |tx, sessions, batch| {
                    let result = appender.resolve(tx, sessions, batch, &BTreeMap::new())?;
                    appended = matches!(result, Resolution::Append(_));
                    Ok(result)
                },
            )
            .unwrap()
        );
        assert!(appended);
        let totals: (i64, i64, i64) = index
            .db
            .query_row(
                "SELECT SUM(input), SUM(records), SUM(uncertain_events) FROM usage_daily",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(totals, (25_000, 25_000, 1));
        assert!(state.events.is_empty());

        drop(index);
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        let committed_offset = state.offset;
        let revision = index.revision().unwrap();
        let mut appender = Appender::default();
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        let next = json!({"type":"event_msg","timestamp":"2026-11-10T00:00:26Z","payload":{
            "type":"token_count","info":{"total_token_usage":{
                "input_tokens":25_002,"output_tokens":0
            }}
        }});
        writeln!(file, "{next}").unwrap();
        drop(file);
        assert!(
            scan_source_resolved(
                &mut index,
                &mut state,
                &path,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &mut index_budget::Budget::new(std::iter::empty()),
                |tx, sessions, batch| {
                    let result = appender.resolve(tx, sessions, batch, &BTreeMap::new())?;
                    assert!(matches!(result, Resolution::Append(_)));
                    Ok(result)
                },
            )
            .unwrap()
        );
        assert!(state.offset > committed_offset);
        assert_eq!(index.revision().unwrap(), revision + 1);
        let totals: (i64, i64) = index
            .db
            .query_row(
                "SELECT SUM(input), SUM(records) FROM usage_daily",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(totals, (25_001, 25_001));

        let row = |second: u32, input: u64, cached: u64| {
            json!({"type":"event_msg","timestamp":format!("2026-11-10T00:00:{second:02}Z"),
                "payload":{"type":"token_count","info":{
                    "total_token_usage":{
                        "input_tokens":input,"cached_input_tokens":cached,"output_tokens":0
                    },
                    "last_token_usage":{
                        "input_tokens":1,"cached_input_tokens":1,"output_tokens":0
                    }
                }}
            })
        };
        let scan_full = |index: &mut PersistentIndex,
                         state: &mut FileState,
                         appender: &mut Appender,
                         path: &Path| {
            assert!(
                scan_source_resolved(
                    index,
                    state,
                    path,
                    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                    &mut index_budget::Budget::new(std::iter::empty()),
                    |tx, sessions, batch| {
                        let result = appender.resolve(tx, sessions, batch, &BTreeMap::new())?;
                        assert!(matches!(result, Resolution::Full(_)));
                        Ok(result)
                    },
                )
                .unwrap()
            );
        };
        let totals = |index: &PersistentIndex| {
            index
                .db
                .query_row(
                    "SELECT SUM(input), SUM(records), SUM(uncertain_events) FROM usage_daily",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .unwrap()
        };

        // 同一身份的第二个副本 raw delta 为零，仍须保留为计数冲突证据。
        let duplicate = row(27, 25_003, 100);
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{duplicate}\n{duplicate}").unwrap();
        drop(file);
        scan_full(&mut index, &mut state, &mut appender, &path);
        assert_eq!(totals(&index), (25_001, 25_001, 2));

        let next = row(28, 25_004, 101);
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{next}").unwrap();
        drop(file);
        scan_full(&mut index, &mut state, &mut appender, &path);
        assert_eq!(totals(&index), (25_002, 25_002, 2));
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-10T00:00:28Z")
            .unwrap()
            .timestamp_millis();
        let cached: Option<i64> = index
            .db
            .query_row(
                "SELECT cached FROM usage_events WHERE timestamp_ms = ?1",
                [at],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cached, Some(1));

        let correction = row(29, 25_004, 99);
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{correction}").unwrap();
        drop(file);
        scan_full(&mut index, &mut state, &mut appender, &path);
        assert_eq!(totals(&index), (25_002, 25_002, 2));
        let cached: Option<i64> = index
            .db
            .query_row(
                "SELECT cached FROM usage_events WHERE timestamp_ms = ?1",
                [at],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cached, None);

        let copy = root.path().join("rollout-copy.jsonl");
        std::fs::write(
            &copy,
            format!(
                "{}\n{next}\n",
                json!({"type":"session_meta","payload":{"id":SESSION}})
            ),
        )
        .unwrap();
        let copy = std::fs::canonicalize(copy).unwrap();
        let mut copy_state = restore_source(&index, &copy).unwrap();
        scan_full(&mut index, &mut copy_state, &mut appender, &copy);
        assert_eq!(totals(&index), (25_002, 25_002, 2));

        drop(index);
        let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        let mut appender = Appender::default();
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", row(30, 25_005, 100)).unwrap();
        drop(file);
        scan_full(&mut index, &mut state, &mut appender, &path);
        assert_eq!(totals(&index), (25_003, 25_003, 2));
        assert_eq!(state.offset, std::fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn restarted_append_rejects_a_source_with_real_time_reversal() {
        use super::super::append::Appender;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-reversed.jsonl");
        let header = json!({"type":"session_meta","payload":{"id":SESSION}});
        let row = |at: &str, input| {
            json!({"type":"event_msg","timestamp":at,"payload":{
                "type":"token_count","info":{"total_token_usage":{
                    "input_tokens":input,"output_tokens":0
                }}
            }})
        };
        let first = row("2026-11-10T00:00:00Z", 10);
        let reversed = row("2026-11-10T08:00:00+09:00", 20);
        std::fs::write(&path, format!("{header}\n{first}\n{reversed}\n")).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let database = root.path().join("usage.sqlite");
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        let mut appender = Appender::default();
        let mut used_append = true;
        assert!(
            scan_source_resolved(
                &mut index,
                &mut state,
                &path,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &mut index_budget::Budget::new(std::iter::empty()),
                |tx, sessions, batch| {
                    let result = appender.resolve(tx, sessions, batch, &BTreeMap::new())?;
                    used_append = matches!(result, Resolution::Append(_));
                    Ok(result)
                },
            )
            .unwrap()
        );
        assert!(!used_append);
        drop(index);

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", row("2026-11-10T09:00:00+09:00", 30)).unwrap();
        drop(file);
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = restore_source(&index, &path).unwrap();
        let mut appender = Appender::default();
        used_append = true;
        assert!(
            scan_source_resolved(
                &mut index,
                &mut state,
                &path,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &mut index_budget::Budget::new(std::iter::empty()),
                |tx, sessions, batch| {
                    let result = appender.resolve(tx, sessions, batch, &BTreeMap::new())?;
                    used_append = matches!(result, Resolution::Append(_));
                    Ok(result)
                },
            )
            .unwrap()
        );
        assert!(!used_append);
    }

    #[test]
    fn related_checkpoint_rejects_append_without_advancing_committed_state() {
        use super::super::append::Appender;

        let root = tempfile::tempdir().unwrap();
        for relation in ["session", "parent"] {
            let path = root.path().join(format!("rollout-{relation}.jsonl"));
            let header = json!({"type":"session_meta","payload":{"id":SESSION}});
            std::fs::write(&path, format!("{header}\n{}\n{}\n", usage(1), usage(2)))
                .unwrap();
            let path = std::fs::canonicalize(path).unwrap();
            let mut index =
                PersistentIndex::open(&root.path().join(format!("{relation}.sqlite"))).unwrap();
            let mut state = restore_source(&index, &path).unwrap();
            let mut appender = Appender::default();
            let scan = |index: &mut PersistentIndex,
                        state: &mut FileState,
                        appender: &mut Appender| {
                scan_source_resolved(
                    index,
                    state,
                    &path,
                    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                    &mut index_budget::Budget::new(std::iter::empty()),
                    |tx, sessions, batch| {
                        appender.resolve(tx, sessions, batch, &BTreeMap::new())
                    },
                )
            };
            assert!(scan(&mut index, &mut state, &mut appender).unwrap());
            let committed = index.source_state(&encode_source(&path)).unwrap();
            let revision = index.revision().unwrap();
            let totals = |index: &PersistentIndex| {
                index
                    .db
                    .query_row(
                        "SELECT SUM(input), SUM(records) FROM usage_daily",
                        [],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .unwrap()
            };
            let before = totals(&index);
            index
                .db
                .execute(
                    "INSERT INTO usage_sources(source, identity, offset, parser_state, complete,
                         scope_session, scope_indexed, scope_valid, execution_indexed, execution_valid)
                     VALUES (X'61', X'01', 0, X'FF', 0, ?1, 1, 1, 1, 1)",
                    [if relation == "session" { SESSION } else { "child-session" }],
                )
                .unwrap();
            index
                .db
                .execute_batch(
                    "INSERT INTO usage_unindexed_checkpoints(source, error)
                     VALUES (X'61', 'invalid related checkpoint');",
                )
                .unwrap();
            if relation == "parent" {
                index
                    .db
                    .execute(
                        "INSERT INTO usage_scope_parents(source, session, parent)
                         VALUES (X'61', 'child-session', ?1)",
                        [SESSION],
                    )
                    .unwrap();
            }
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(file, "{}", usage(3)).unwrap();
            drop(file);
            let error = scan(&mut index, &mut state, &mut appender).unwrap_err();
            assert!(format!("{error:#}").contains("invalid related checkpoint"));
            assert_eq!(index.source_state(&encode_source(&path)).unwrap(), committed);
            assert_eq!(state.offset, committed.unwrap().offset);
            assert!(state.events.is_empty());
            assert_eq!(index.revision().unwrap(), revision);
            assert_eq!(totals(&index), before);
            let observations: i64 = index
                .db
                .query_row("SELECT COUNT(*) FROM usage_observations", [], |row| row.get(0))
                .unwrap();
            assert_eq!(observations, 2);
        }
    }
}
