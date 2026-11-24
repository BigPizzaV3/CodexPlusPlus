use super::{
    super::{Checkpoint, SourceStamp},
    decode_path,
};
use crate::usage_history::structured::{
    FileState, Index, Snapshot, UsageEvent, index_budget::Budget,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const MAX_SOURCES: usize = 256;
const REPLAY_WORK_BYTES: usize = 24 * 1024 * 1024;

#[derive(PartialEq, Eq)]
struct Source {
    path: PathBuf,
    revision: i64,
    offset: u64,
    stamp: SourceStamp,
    session: String,
}

pub(super) struct Replay {
    bounds: (i64, i64),
    scope: Option<HashSet<String>>,
    sources: Vec<Source>,
    next_source: usize,
    current: FileState,
    index: Index,
}

fn sources(
    db: &Connection,
    dates: (NaiveDate, NaiveDate),
    scope: Option<&HashSet<String>>,
) -> Result<Vec<Source>> {
    let mut statement = db.prepare_cached(
        "WITH RECURSIVE component(id, link) AS (
            SELECT s.id, NULL FROM token_sources s
            WHERE (json_extract(CAST(s.checkpoint AS TEXT), '$.token_day_span') IS NULL
                OR (json_extract(CAST(s.checkpoint AS TEXT), '$.token_day_span.first') <= ?2
                AND json_extract(CAST(s.checkpoint AS TEXT), '$.token_day_span.last') >= ?1))
            AND (?3 IS NULL OR EXISTS (
                SELECT 1 FROM token_source_links l JOIN json_each(?3) j ON l.link = 's:' || j.value
                WHERE l.source_id = s.id))
            UNION SELECT NULL, own.link FROM component
                JOIN token_source_links own ON own.source_id = component.id
            UNION SELECT neighbor.source_id, NULL FROM component
                JOIN token_source_links neighbor ON neighbor.link = component.link
        )
        SELECT s.source, s.revision, s.offset, s.checkpoint
        FROM component JOIN token_sources s ON s.id = component.id
        ORDER BY s.id LIMIT ?4",
    )?;
    let mut rows = statement.query(params![
        dates.0.num_days_from_ce(),
        dates.1.num_days_from_ce(),
        scope.map(serde_json::to_string).transpose()?,
        MAX_SOURCES + 1,
    ])?;
    let mut sources = Vec::new();
    while let Some(row) = rows.next()? {
        ensure!(
            sources.len() < MAX_SOURCES,
            "archive recovery exceeds source budget"
        );
        let checkpoint: Checkpoint = serde_json::from_slice(row.get_ref(3)?.as_blob()?)?;
        ensure!(
            !checkpoint.resume_pending
                && !checkpoint.skipping
                && checkpoint.issues == 0
                && !checkpoint.invalid_counts
                && !checkpoint.identity_conflict
                && !checkpoint.session.is_empty(),
            "archive source checkpoint is not complete"
        );
        sources.push(Source {
            path: decode_path(row.get(0)?)?,
            revision: row.get(1)?,
            offset: row.get(2)?,
            stamp: checkpoint
                .stamp
                .context("archive source has no file identity")?,
            session: checkpoint.session,
        });
    }
    Ok(sources)
}

fn dates((start, end): (i64, i64)) -> Result<(NaiveDate, NaiveDate)> {
    ensure!(start < end, "invalid archive range");
    let day = |at| {
        Local
            .timestamp_millis_opt(at)
            .single()
            .map(|time| time.date_naive())
            .context("archive date outside supported range")
    };
    Ok((day(start)?, day(end - 1)?))
}

impl Replay {
    pub(super) fn new(
        db: &Connection,
        home: &Path,
        bounds: (i64, i64),
        scope: Option<HashSet<String>>,
        provider_names: BTreeMap<String, String>,
    ) -> Result<Self> {
        let home = home.canonicalize()?;
        let sessions = home.join("sessions");
        let archived = home.join("archived_sessions");
        let sources = sources(db, dates(bounds)?, scope.as_ref())?;
        for source in &sources {
            ensure!(
                (source.path.starts_with(&sessions) || source.path.starts_with(&archived))
                    && source.path.canonicalize()? == source.path,
                "archive source path is outside the current home's history"
            );
        }
        Ok(Self {
            bounds,
            scope,
            sources,
            next_source: 0,
            current: FileState::default(),
            index: Index {
                provider_names,
                ..Index::default()
            },
        })
    }

    /// Processes one source at a time using the same budgeted parser as ingestion.
    pub(super) fn step(&mut self) -> Result<bool> {
        let Some(source) = self.sources.get(self.next_source) else {
            return Ok(true);
        };
        ensure!(
            FileState::source_stamp(&source.path)? == source.stamp,
            "archive source changed before replay completed"
        );
        let mut budget = Budget::new(self.index.files.values());
        budget.activity_limit = 0;
        let allowance = (budget.read_left, budget.source_left);
        let offset = self.current.offset;
        let (complete, replaced) =
            self.current
                .read_source_batch(&source.path, NaiveDate::MIN, &mut budget)?;
        ensure!(
            !replaced
                && self.current.stamp == Some(source.stamp)
                && self.current.offset <= source.offset,
            "archive replay exceeded its committed source"
        );
        ensure!(
            self.current.retained_charge
                + self
                    .index
                    .files
                    .values()
                    .map(|state| state.retained_charge)
                    .sum::<usize>()
                <= REPLAY_WORK_BYTES,
            "archive replay exceeds reconciliation memory budget"
        );
        if !complete {
            ensure!(
                self.current.offset > offset || allowance != (budget.read_left, budget.source_left),
                "archive replay could not advance within its budget"
            );
            return Ok(false);
        }
        ensure!(
            self.current.offset == source.offset
                && self.current.session == source.session
                && !self.current.invalid_counts
                && !self.current.identity_conflict
                && self.current.issues == 0,
            "archive replay differs from the committed source checkpoint"
        );
        self.index
            .files
            .insert(source.path.clone(), std::mem::take(&mut self.current));
        self.next_source += 1;
        Ok(self.next_source == self.sources.len())
    }

    pub(super) fn finish(mut self, db: &Connection) -> Result<Vec<UsageEvent>> {
        ensure!(
            self.next_source == self.sources.len(),
            "archive replay is not complete"
        );
        let read = db.unchecked_transaction()?;
        ensure!(
            self.sources == sources(&read, dates(self.bounds)?, self.scope.as_ref())?,
            "archive source set or cursor changed during replay"
        );
        for source in &self.sources {
            ensure!(
                FileState::source_stamp(&source.path)? == source.stamp
                    && source.path.canonicalize()? == source.path,
                "archive source changed before result publication"
            );
        }
        {
            let mut collision =
                read.prepare_cached("SELECT 1 FROM token_collisions WHERE id = ?1")?;
            for state in self.index.files.values() {
                for event in &state.events {
                    if collision.exists([event.original_id.as_slice()])? {
                        self.index.collision_families.insert(event.original_id);
                    }
                }
            }
        }
        read.commit()?;
        self.index
            .collision_families
            .extend(self.index.observed_collisions().into_keys());
        self.index.apply_collision_ids();
        let mut snapshot = Snapshot::default();
        self.index.select_observations(
            NaiveDate::MIN,
            &mut snapshot,
            &mut BTreeMap::new(),
            usize::MAX,
        );
        self.index.resolve_counts(&mut snapshot);
        ensure!(
            snapshot.issues == 0,
            "archive replay has inconsistent Token order"
        );
        let mut events = Vec::new();
        for event in snapshot.events {
            let at = DateTime::parse_from_rfc3339(&event.timestamp)?.timestamp_millis();
            if at < self.bounds.0
                || at >= self.bounds.1
                || self
                    .scope
                    .as_ref()
                    .is_some_and(|scope| !scope.contains(&event.session))
            {
                continue;
            }
            ensure!(
                !event.count_uncertain && !event.count_incomplete,
                "archive replay cannot resolve a Token count"
            );
            if event.delta.total_tokens > 0 {
                events.push(event);
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_history::structured::token_checkpoint::{scan::Scan, store::Store};
    use serde_json::json;
    use std::fs;

    const SESSION: &str = "00000000-0000-4000-8000-000000000131";
    const OTHER: &str = "00000000-0000-4000-8000-000000000132";

    fn stamp(day: NaiveDate, hour: u32) -> chrono::DateTime<Local> {
        Local
            .from_local_datetime(&day.and_hms_opt(hour, 0, 0).unwrap())
            .single()
            .unwrap()
    }

    fn session_log(session: &str, day: NaiveDate, extra: &str) -> String {
        let first = stamp(day, 12).to_rfc3339();
        let second = stamp(day, 13).to_rfc3339();
        let header = json!({"type":"session_meta","timestamp":first,
            "payload":{"id":session,"model_provider":"custom"}});
        let context = json!({"type":"turn_context","timestamp":first,
            "payload":{"model":"gpt-fixture","turn_id":"fixture"}});
        let tokens = |scale: u64| {
            json!({
                "input_tokens":100 * scale,"cached_input_tokens":20 * scale,"output_tokens":10 * scale
            })
        };
        let count = |time: &str, scale| {
            json!({"type":"event_msg","timestamp":time,
            "payload":{"type":"token_count","info":{
                "total_token_usage":tokens(scale),"last_token_usage":tokens(1)
            }}})
        };
        format!(
            "{header}\n{context}\n{}\n{extra}{}\n",
            count(&first, 1),
            count(&second, 2)
        )
    }

    #[test]
    fn cold_archive_replays_compressed_and_plain_copies_without_touching_the_index() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let archived = home.join("archived_sessions");
        fs::create_dir(&archived).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let original = archived.join("rollout-original.jsonl");
        let compressed = archived.join("rollout-copy.jsonl.zst");
        let unrelated = archived.join("rollout-newer.jsonl");
        let body = json!({"type":"response_item","payload":{"type":"message","output":"a".repeat(256 * 1024)}});
        let old = session_log(SESSION, day, &format!("{body}\n").repeat(17));
        fs::write(&original, &old).unwrap();
        fs::write(
            &compressed,
            zstd::stream::encode_all(old.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        fs::write(&unrelated, session_log(OTHER, cutoff, "")).unwrap();
        let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        let mut scan = Scan::default();
        let mut complete = false;
        for _ in 0..10 {
            let report = scan.round(&mut store, &home, cutoff, &BTreeMap::new(), 1 << 20);
            assert!(report.errors.is_empty(), "{:?}", report.errors);
            if report.complete {
                complete = true;
                break;
            }
        }
        assert!(complete, "the initial scan also needs multiple rounds");
        let revision: i64 = store
            .db
            .query_row("SELECT revision FROM token_revision", [], |r| r.get(0))
            .unwrap();
        let old_days: i64 = store
            .db
            .query_row(
                "SELECT count(*) FROM token_days WHERE day = ?1",
                [day.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_days, 0, "first scan never ingested archived detail");

        let bounds = (
            stamp(day, 12).timestamp_millis(),
            stamp(day, 13).timestamp_millis(),
        );
        let mut replay = Replay::new(&store.db, &home, bounds, None, BTreeMap::new()).unwrap();
        assert_eq!(replay.sources.len(), 2);
        assert!(replay.sources.iter().all(|source| source.path != unrelated));
        let mut steps = 1;
        while !replay.step().unwrap() {
            steps += 1;
            assert!(steps < 12, "bounded parse should finish this fixture");
        }
        assert!(steps > 2, "at least one source should span multiple rounds");
        let events = replay.finish(&store.db).unwrap();
        assert_eq!(
            events.len(),
            1,
            "archived replay deduplicates copies and excludes the end timestamp"
        );
        assert_eq!(events[0].delta.total_tokens, 110);
        assert_eq!(events[0].delta.cached_input_tokens, 20);
        assert!(events[0].delta.cached_known);
        assert_eq!(
            store
                .db
                .query_row("SELECT revision FROM token_revision", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            revision
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT count(*) FROM token_days WHERE day = ?1",
                    [day.to_string()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );

        let mut replay = Replay::new(&store.db, &home, bounds, None, BTreeMap::new()).unwrap();
        while !replay.step().unwrap() {}
        // The already-read file must still be the one that supplied the committed cursor.
        fs::write(&original, format!("{old}\n")).unwrap();
        assert!(
            replay
                .finish(&store.db)
                .err()
                .unwrap()
                .to_string()
                .contains("changed")
        );
    }

    #[test]
    fn old_checkpoints_do_not_acquire_false_date_coverage_on_append() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let sessions = home.join("sessions");
        fs::create_dir(&sessions).unwrap();
        let source = sessions.join("rollout-legacy.jsonl");
        let today = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let before = today.pred_opt().unwrap();
        let old = session_log(SESSION, before, "");
        fs::write(&source, &old).unwrap();
        let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        let mut scan = Scan::default();
        assert!(
            scan.round(&mut store, &home, today, &BTreeMap::new(), 1 << 20)
                .complete
        );
        store.db.execute(
            "UPDATE token_sources SET checkpoint = CAST(json_remove(CAST(checkpoint AS TEXT), '$.token_day_span') AS BLOB)",
            [],
        ).unwrap();
        let new_line = json!({"type":"event_msg","timestamp":stamp(today, 12).to_rfc3339(),
        "payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":300,"output_tokens":30,"cached_input_tokens":60},
            "last_token_usage":{"input_tokens":100,"output_tokens":10,"cached_input_tokens":20}
        }}});
        fs::write(&source, format!("{old}{new_line}\n")).unwrap();
        assert!(
            scan.round(&mut store, &home, today, &BTreeMap::new(), 1 << 20)
                .complete
        );
        let (_, checkpoint) = store.checkpoint(&source).unwrap().unwrap();
        assert!(checkpoint.token_day_span.is_none());
        let bounds = (
            stamp(before, 12).timestamp_millis(),
            stamp(before, 13).timestamp_millis(),
        );
        let mut replay = Replay::new(&store.db, &home, bounds, None, BTreeMap::new()).unwrap();
        assert_eq!(replay.sources.len(), 1);
        assert!(replay.step().unwrap());
        assert_eq!(replay.finish(&store.db).unwrap()[0].delta.total_tokens, 110);
    }

    #[test]
    fn scoped_archive_replays_a_referenced_prefix_outside_the_requested_day() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let sessions = home.join("sessions");
        fs::create_dir(&sessions).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let previous = stamp(day.pred_opt().unwrap(), 12).to_rfc3339();
        let current = stamp(day, 12).to_rfc3339();
        let physical = "00000000-0000-4000-8000-000000000133";
        let parent = sessions.join(format!("rollout-base-{physical}.jsonl"));
        let child = sessions.join("rollout-continuation.jsonl");
        let head = |session, ordinal, time| {
            json!({
                "type":"session_meta","ordinal":ordinal,"timestamp":time,
                "payload":{"id":session,"history_mode":"paginated","model_provider":"custom"}
            })
        };
        let count = |ordinal, time, n: u64| {
            json!({
                "type":"event_msg","ordinal":ordinal,"timestamp":time,"payload":{"type":"token_count",
                "info":{"total_token_usage":{"input_tokens":n*100,"cached_input_tokens":n*20,"output_tokens":n*10}}}
            })
        };
        let base = format!(
            "{}\n{}\n",
            head(OTHER, 0, &previous),
            count(1, &previous, 1)
        );
        fs::write(&parent, &base).unwrap();
        let mut continued = head(SESSION, 2, &current);
        continued["payload"]["history_base"] = json!({
            "thread_id":physical,"end_ordinal_exclusive":2,"end_byte_offset":base.len()
        });
        fs::write(&child, format!("{continued}\n{}\n", count(3, &current, 2))).unwrap();
        let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        assert!(
            Scan::default()
                .round(&mut store, &home, cutoff, &BTreeMap::new(), 1 << 20)
                .complete
        );
        let bounds = (
            stamp(day, 12).timestamp_millis(),
            stamp(day, 13).timestamp_millis(),
        );
        let mut replay = Replay::new(
            &store.db,
            &home,
            bounds,
            Some([SESSION.to_owned()].into()),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(replay.sources.len(), 2);
        assert!(!replay.step().unwrap());
        assert!(replay.step().unwrap());
        let events = replay.finish(&store.db).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].session, SESSION);
        assert_eq!(
            events[0].delta.total_tokens, 110,
            "causal prefix intersects count resolution but not returned scope"
        );
    }
}
