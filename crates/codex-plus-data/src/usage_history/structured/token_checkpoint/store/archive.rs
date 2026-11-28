use super::{
    super::{Checkpoint, Observation, SourceStamp, historical::EvidenceOverlay},
    decode_path,
};
use crate::usage_history::structured::{
    FileState, Index, Snapshot, UsageEvent, index_budget::Budget, terminal_receipts,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

const ARCHIVE_CACHE: &str = "usage-token-archive-v1.json";
const ARCHIVE_CACHE_DIR: &str = "usage-token-archives-v1";

#[derive(Clone, PartialEq, Eq)]
pub(in crate::usage_history::structured) struct ArchiveKey {
    pub(in crate::usage_history::structured) first: NaiveDate,
    pub(in crate::usage_history::structured) end: NaiveDate,
    pub(in crate::usage_history::structured) session: Option<String>,
}

impl ArchiveKey {
    pub(in crate::usage_history::structured) fn contains(&self, other: &Self) -> bool {
        self.session == other.session && self.first <= other.first && self.end >= other.end
    }

    fn scope(&self) -> Option<HashSet<String>> {
        self.session.as_ref().map(|id| [id.clone()].into())
    }

    fn committed_sources(&self, read: &Connection) -> Result<Vec<Source>> {
        let last = self.end.pred_opt().context("invalid archive date window")?;
        sources(read, (self.first, last), self.scope().as_ref())
    }

    pub(in crate::usage_history::structured::token_checkpoint) fn replay(
        &self,
        db: &Connection,
        home: &Path,
        provider_names: BTreeMap<String, String>,
    ) -> Result<Replay> {
        ensure!(
            (self.end - self.first).num_days() <= 16_384,
            "archive date window exceeds recovery budget"
        );
        let midnight = |day: NaiveDate| {
            Local
                .from_local_datetime(&day.and_time(chrono::NaiveTime::MIN))
                .earliest()
                .map(|time| time.timestamp_millis())
                .context("archive date has no local midnight")
        };
        let (start, end) = (midnight(self.first)?, midnight(self.end)?);
        Replay::new(db, home, (start, end), self.scope(), provider_names)
    }
}

pub(in crate::usage_history::structured) struct Recovered {
    revision: i64,
    sources: Vec<Source>,
    pub(in crate::usage_history::structured) events: Vec<UsageEvent>,
    pub(in crate::usage_history::structured::token_checkpoint) evidence:
        Option<Result<EvidenceOverlay, String>>,
}

impl Recovered {
    pub(in crate::usage_history::structured::token_checkpoint) fn save(
        &self,
        key: &ArchiveKey,
        storage: &Path,
    ) -> Result<()> {
        let cached = CachedArchive {
            first: key.first.to_string(),
            end: key.end.to_string(),
            session: key.session.clone(),
            sources: self.sources.clone(),
            events: self
                .events
                .iter()
                .map(|event| (event.id, event.session.clone(), Observation::capture(event)))
                .collect(),
        };
        let directory = storage.join(ARCHIVE_CACHE_DIR);
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!(
            "{}-{}-{}.json",
            key.first,
            key.end,
            key.session.as_deref().unwrap_or("all")
        ));
        codex_plus_core::settings::atomic_write_with(&path, |file| {
            let mut output = BufWriter::new(file);
            serde_json::to_writer(&mut output, &cached)?;
            output.flush()
        })
    }

    pub(in crate::usage_history::structured::token_checkpoint) fn load_for(
        db: &Connection,
        storage: &Path,
        requested: &ArchiveKey,
    ) -> Result<Option<(ArchiveKey, Self)>> {
        let suffix = format!("-{}.json", requested.session.as_deref().unwrap_or("all"));
        let mut paths = Vec::new();
        match std::fs::read_dir(storage.join(ARCHIVE_CACHE_DIR)) {
            Ok(directory) => {
                for entry in directory {
                    let entry = entry?;
                    let name = entry.file_name();
                    if let Some(dates) = name.to_str().and_then(|name| name.strip_suffix(&suffix))
                        && let Some((start, end)) = dates.split_at_checked(10)
                        && let (Ok(first), Some(Ok(end))) =
                            (start.parse::<NaiveDate>(), end.strip_prefix('-').map(str::parse::<NaiveDate>))
                        && first <= requested.first && end >= requested.end
                    {
                        paths.push(((end - first).num_days(), entry.path()));
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        // 只读覆盖本次查询的缓存，优先命中最小区间；旧版单文件仍可迁移。
        paths.sort_unstable();
        let legacy = storage.join(ARCHIVE_CACHE);
        for path in paths.into_iter().map(|(_, path)| path).chain([legacy]) {
            match Self::load_file(db, &path) {
                Ok(Some((key, answer))) if key.contains(requested) => return Ok(Some((key, answer))),
                Ok(_) => {}
                Err(error) => eprintln!("Usage archive cache restore failed: {error:#}"),
            }
        }
        Ok(None)
    }

    fn load_file(db: &Connection, path: &Path) -> Result<Option<(ArchiveKey, Self)>> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let cached: CachedArchive = serde_json::from_reader(BufReader::new(file))?;
        let key = ArchiveKey {
            first: cached.first.parse()?,
            end: cached.end.parse()?,
            session: cached.session,
        };
        let read = db.unchecked_transaction()?;
        let revision = read.query_row("SELECT revision FROM token_revision", [], |row| row.get(0))?;
        // 数据库重建后版本号可能相同，冷加载总是重新核对已提交游标。
        if cached.sources != key.committed_sources(&read)? {
            return Ok(None);
        }
        let mut recovered = Self {
            revision,
            sources: cached.sources,
            events: Vec::new(),
            evidence: None,
        };
        if !recovered.is_current(&read, revision, &key)? {
            return Ok(None);
        }
        read.commit()?;
        recovered.events = cached
            .events
            .into_iter()
            .map(|(id, session, observation)| observation.into_event(id, session))
            .collect::<Result<_>>()?;
        Ok(Some((key, recovered)))
    }

    pub(in crate::usage_history::structured::token_checkpoint) fn reuse(
        &self,
        db: &Connection,
        key: &ArchiveKey,
    ) -> Result<Option<Self>> {
        let read = db.unchecked_transaction()?;
        let revision = read.query_row("SELECT revision FROM token_revision", [], |row| row.get(0))?;
        if !self.is_current(&read, revision, key)? {
            return Ok(None);
        }
        read.commit()?;
        Ok(Some(Self {
            revision,
            sources: self.sources.clone(),
            events: self.events.clone(),
            evidence: None,
        }))
    }

    pub(super) fn is_current(
        &self,
        read: &Connection,
        revision: i64,
        key: &ArchiveKey,
    ) -> Result<bool> {
        if self.revision != revision && self.sources != key.committed_sources(read)? {
            return Ok(false);
        }
        for source in &self.sources {
            match FileState::source_stamp(&source.path) {
                Ok(stamp) if stamp == source.stamp => {}
                Ok(_) => return Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(true)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Source {
    path: PathBuf,
    revision: i64,
    offset: u64,
    stamp: SourceStamp,
    session: String,
}

#[derive(Serialize, Deserialize)]
struct CachedArchive {
    first: String,
    end: String,
    session: Option<String>,
    sources: Vec<Source>,
    events: Vec<([u8; 32], String, Observation)>,
}

pub(in crate::usage_history::structured::token_checkpoint) struct Replay {
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
        ORDER BY s.id",
    )?;
    let mut rows = statement.query(params![
        dates.0.num_days_from_ce(),
        dates.1.num_days_from_ce(),
        scope.map(serde_json::to_string).transpose()?,
    ])?;
    let mut sources = Vec::new();
    while let Some(row) = rows.next()? {
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
        ensure!(
            !sources.is_empty(),
            "no committed original sources for this archive range"
        );
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
    pub(in crate::usage_history::structured::token_checkpoint) fn step(&mut self) -> Result<bool> {
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
            complete
                || self.current.offset > offset
                || allowance != (budget.read_left, budget.source_left),
            "archive replay could not advance within its budget"
        );
        ensure!(
            !replaced
                && self.current.stamp == Some(source.stamp)
                && self.current.offset <= source.offset,
            "archive replay exceeded its committed source"
        );
        if !complete {
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

    pub(in crate::usage_history::structured::token_checkpoint) fn finish(
        mut self,
        db: &Connection,
    ) -> Result<Recovered> {
        ensure!(
            self.next_source == self.sources.len(),
            "archive replay is not complete"
        );
        let read = db.unchecked_transaction()?;
        let revision = read.query_row("SELECT revision FROM token_revision", [], |r| r.get(0))?;
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
        terminal_receipts::project(&self.index, &mut snapshot);
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
            // 未核准的增量仍参与覆盖检查，但不会计入已确认 Token。
            if event.delta.total_tokens > 0 || event.count_uncertain || event.count_incomplete {
                events.push(event);
            }
        }
        Ok(Recovered {
            revision,
            sources: self.sources,
            events,
            evidence: None,
        })
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
    fn archive_replays_settled_events_alongside_unresolved_counts() -> Result<()> {
        use crate::usage_history::Query;

        let root = tempfile::tempdir()?;
        let home = root.path().canonicalize()?;
        let archived = home.join("archived_sessions");
        fs::create_dir(&archived)?;
        let day = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        fs::write(archived.join("rollout-settled.jsonl"), session_log(SESSION, day, ""))?;
        let missing = json!({"type":"event_msg","timestamp":stamp(day, 12).to_rfc3339(),
            "payload":{"type":"token_count","info":{
                "total_token_usage":{"input_tokens":500,"cached_input_tokens":100,"output_tokens":50}
            }}});
        fs::write(archived.join("rollout-unresolved.jsonl"), format!("{}\n{missing}\n",
            json!({"type":"session_meta","timestamp":stamp(day, 12).to_rfc3339(),
                "payload":{"id":OTHER,"model_provider":"custom"}})))?;
        let mut store = Store::open(Connection::open_in_memory()?)?;
        let names = BTreeMap::new();
        let mut scan = Scan::default();
        scan.round(&mut store, &home, cutoff, &names, 1 << 20);
        let key = ArchiveKey { first: day, end: day.succ_opt().unwrap(), session: None };
        let mut replay = key.replay(&store.db, &home, names.clone())?;
        while !replay.step()? {}
        let recovered = replay.finish(&store.db)?;
        assert_eq!(recovered.events.iter().filter(|e| e.count_uncertain).count(), 1);
        let request = json!({"startAt":stamp(day, 0).timestamp_millis(),
            "endAt":stamp(day.succ_opt().unwrap(), 0).timestamp_millis(),"includeTrend":true});
        let q = Query::parse(&request, cutoff).unwrap();
        let answer = (key, Ok(recovered));
        let (published, _) = Store::read_response_with_archive(
            &mut store.db, &q, &request, None, &names, Some(&answer), None)?;
        assert_eq!(published["days"][0]["usage"]["totalTokens"], 220);
        assert_eq!(published["days"][0]["recordCount"], 2);
        assert_eq!(published["countStatus"]["uncertainEvents"], 1);
        assert_eq!(published["countStatus"]["complete"], false);
        Ok(())
    }

    #[test]
    fn archive_replay_uses_the_shared_budget_across_more_than_256_sources() -> Result<()> {
        use crate::usage_history::{Tokens, structured::index_budget::MAX_RETAINED_BYTES};
        use sha2::{Digest, Sha256};
        use std::fmt::Write;

        const SOURCE_COUNT: usize = 257;
        const EVENTS_PER_SOURCE: i64 = 200;
        let root = tempfile::tempdir()?;
        let home = root.path().canonicalize()?;
        let archived = home.join("archived_sessions");
        fs::create_dir(&archived)?;
        let day = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let at = stamp(day, 12);
        let model = "archive-model-".repeat(40);
        let mut body = format!(
            "{}\n",
            json!({"type":"turn_context","timestamp":at.to_rfc3339(),
            "payload":{"model":model,"turn_id":"archive-turn"}})
        );
        let delta = Tokens::new(10, 0, 1);
        let mut expected = Vec::new();
        for n in 1..=EVENTS_PER_SOURCE {
            let time = at + chrono::Duration::seconds(n);
            let total = Tokens::new(10 * n as u64, 0, n as u64);
            expected.push((
                time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                serde_json::to_vec(&json!([total, delta]))?,
                total,
            ));
            writeln!(
                body,
                "{}",
                json!({"type":"event_msg","timestamp":time.to_rfc3339(),"payload":{
                "type":"token_count","info":{
                    "last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":1},
                    "total_token_usage":{"input_tokens":10*n,"cached_input_tokens":0,"output_tokens":n}}}})
            )?;
        }
        for n in 0..SOURCE_COUNT {
            let id = format!("00000000-0000-4000-8000-{n:012x}");
            let header =
                json!({"type":"session_meta","timestamp":at.to_rfc3339(),"payload":{"id":id}});
            fs::write(
                archived.join(format!("rollout-{id}.jsonl")),
                format!("{header}\n{body}"),
            )?;
        }
        let mut store = Store::open(Connection::open_in_memory()?)?;
        let mut scan = Scan::default();
        let names = BTreeMap::new();
        let cutoff = day.succ_opt().unwrap();
        for _ in 0..512 {
            let report = scan.round(&mut store, &home, cutoff, &names, 1 << 20);
            assert!(report.errors.is_empty(), "{:?}", report.errors);
            if report.complete {
                break;
            }
        }
        let key = ArchiveKey {
            first: day,
            end: cutoff,
            session: None,
        };
        let mut replay = key.replay(&store.db, &home, names)?;
        assert_eq!(replay.sources.len(), SOURCE_COUNT);
        assert!(!replay.step()?);
        let used = replay.index.files.values().next().unwrap().retained_charge;
        replay
            .index
            .files
            .values_mut()
            .next()
            .unwrap()
            .retained_charge = MAX_RETAINED_BYTES;
        assert!(
            replay
                .step()
                .unwrap_err()
                .to_string()
                .contains("could not advance within its budget")
        );
        assert_eq!(replay.next_source, 1);
        replay
            .index
            .files
            .values_mut()
            .next()
            .unwrap()
            .retained_charge = used;
        while !replay.step()? {}
        let retained: usize = replay.index.files.values().map(|s| s.retained_charge).sum();
        assert!(
            retained > 24 << 20 && retained <= MAX_RETAINED_BYTES,
            "{retained}"
        );
        let recovered = replay.finish(&store.db)?;
        assert_eq!(recovered.events.len(), SOURCE_COUNT * expected.len());
        let mut events: BTreeMap<_, _> = recovered.events.iter().map(|e| (e.id, e)).collect();
        for n in 0..SOURCE_COUNT {
            let session = format!("00000000-0000-4000-8000-{n:012x}");
            for (time, counters, total) in &expected {
                let id: [u8; 32] = Sha256::new()
                    .chain_update(session.as_bytes())
                    .chain_update(time.as_bytes())
                    .chain_update(counters)
                    .finalize()
                    .into();
                let event = events.remove(&id).unwrap_or_else(|| {
                    panic!("missing archive event for session {session} at {time}")
                });
                assert_eq!(event.original_id, id);
                assert_eq!(event.session, session);
                assert_eq!(&event.timestamp, time);
                assert_eq!(&event.total, total);
                assert_eq!(event.delta, delta);
            }
        }
        assert!(events.is_empty());
        Ok(())
    }

    #[test]
    fn archived_evidence_reclassifies_one_event_without_replacing_counts_and_is_revocable() {
        use crate::usage_history::{
            Query, structured::token_checkpoint::historical::EvidenceVerifier,
        };
        use std::sync::Mutex;

        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let archive = home.join("archived_sessions");
        fs::create_dir(&archive).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        fs::write(
            archive.join("rollout-old.jsonl"),
            session_log(SESSION, day, ""),
        )
        .unwrap();
        let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        let mut scan = Scan::default();
        let names = BTreeMap::new();
        assert!(scan.round(&mut store, &home, day, &names, 1 << 20).complete);
        assert!(
            scan.round(&mut store, &home, cutoff, &names, 1 << 20)
                .complete
        );
        let request = json!({"startAt":stamp(day, 0).timestamp_millis(),
            "endAt":stamp(day.succ_opt().unwrap(), 0).timestamp_millis(),
            "includeTrend":true, "includeActivity":true});
        let q = Query::parse(&request, cutoff).unwrap();
        store
            .record_coverage(
                crate::usage_history::structured::token_checkpoint::store::Coverage {
                    pending_sources: 1, issues: 0, maintenance_blocked: false,
                },
                false,
            )
            .unwrap();
        let (first, schedule) = Store::read_response_with_archive(
            &mut store.db,
            &q,
            &request,
            Some(SESSION),
            &names,
            None,
            None,
        )
        .unwrap();
        assert_eq!(first["complete"], false);
        let key = schedule.expect("unrelated scan backlog must not block committed archive replay");
        // Daily preloads the containing month while the selected day is open.
        let key = ArchiveKey {
            first: key.first.pred_opt().unwrap(),
            ..key
        };
        let mut replay = key.replay(&store.db, &home, names.clone()).unwrap();
        assert!(replay.step().unwrap());
        let mut result = replay.finish(&store.db).unwrap();
        let event = &result.events[0];
        let mut record = json!({
            "eventId":event.id.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "session":event.session, "timestamp":event.timestamp, "role":event.role, "model":event.model,
            "input":100, "cached":20, "output":10, "provider":"old-relay", "providerName":"Old Relay",
            "source":"upstream-exact-bidirectional"
        });
        let settings = home.join("settings.json");
        let file = settings.with_file_name("usage-provider-evidence.json");
        let publish = |record: &serde_json::Value| {
            fs::write(&file, json!({"version":1,"records":[record]}).to_string()).unwrap()
        };
        publish(&record);
        let verifier = Mutex::new(EvidenceVerifier::default());
        result.evidence = Some(Ok(EvidenceOverlay::for_archive(
            &verifier,
            &settings,
            &result.events,
            false,
        )
        .unwrap()));
        let mut answer = (key, Ok(result));
        let read = |store: &mut Store, answer: &super::super::projection::ArchiveAnswer| {
            Store::read_response_with_archive(
                &mut store.db,
                &q,
                &request,
                Some(SESSION),
                &names,
                Some(answer),
                None,
            )
            .unwrap()
        };
        let (complete, next) = read(&mut store, &answer);
        assert!(next.is_none());
        assert_eq!(complete["archiveRecovery"]["status"], "partial");
        assert_eq!(complete["complete"], false);
        assert_eq!(complete["activityComplete"], false);
        assert_eq!(complete["revision"], first["revision"]);
        assert_eq!(complete["days"][0]["usage"], first["days"][0]["usage"]);
        let amount = |response: &serde_json::Value| {
            response["days"][0]["breakdowns"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["provider"] == "old-relay")
                .map(|row| row["usage"]["totalTokens"].as_u64().unwrap())
        };
        assert_eq!(amount(&complete), Some(110));
        assert_eq!(complete["scopedUsage"]["days"], complete["days"]);
        assert_eq!(
            complete["trend"]["buckets"][24]["breakdowns"][0]["provider"],
            "old-relay"
        );
        assert_eq!(complete["providerNames"]["old-relay"], "Old Relay");

        let saved = file.with_extension("removed");
        fs::rename(&file, &saved).unwrap();
        let (pending, reschedule) = read(&mut store, &answer);
        assert!(reschedule.as_ref() == Some(&answer.0));
        assert_eq!(pending["archiveRecovery"]["status"], "pending");
        assert_eq!(pending["classificationStatus"]["complete"], false);
        assert_eq!(pending["days"], complete["days"]);
        let recovered = answer.1.as_mut().unwrap();
        recovered.evidence = Some(Ok(EvidenceOverlay::for_archive(
            &verifier, &settings, &recovered.events, false,
        ).unwrap()));
        let (withdrawn, next) = read(&mut store, &answer);
        assert!(next.is_none());
        assert_eq!(first["days"][0]["fingerprintCoverage"], "unavailable");
        assert_eq!(withdrawn["days"][0]["fingerprintCoverage"], "events");
        assert_eq!(withdrawn["days"][0]["usage"], first["days"][0]["usage"]);
        assert_eq!(
            withdrawn["days"][0]["accountingFingerprint"],
            complete["days"][0]["accountingFingerprint"]
        );
        assert_eq!(
            withdrawn["trend"]["buckets"][24]["usage"]["totalTokens"],
            110
        );
        assert_eq!(amount(&withdrawn), None);
        assert!(withdrawn["providerNames"]["old-relay"].is_null());
        record["provider"] = json!("rewritten-relay");
        publish(&record);
        let (changed, reschedule) = read(&mut store, &answer);
        assert!(reschedule.is_some());
        assert_eq!(changed["days"], withdrawn["days"]);
        let recovered = answer
            .1
            .as_mut()
            .unwrap_or_else(|error: &mut String| panic!("{error}"));
        recovered.evidence = Some(Ok(EvidenceOverlay::for_archive(
            &verifier,
            &settings,
            &recovered.events,
            false,
        )
        .unwrap()));
        let (rejected, next) = read(&mut store, &answer);
        assert!(next.is_none());
        assert_eq!(
            rejected["days"], withdrawn["days"],
            "mutated event proof cannot claim a new supplier"
        );
        assert!(rejected["providerNames"]["rewritten-relay"].is_null());
    }

    #[test]
    fn recovered_trend_keeps_archived_day_totals_and_does_not_invent_classification() {
        use crate::usage_history::Query;

        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let archived = home.join("archived_sessions");
        fs::create_dir(&archived).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        fs::write(
            archived.join("rollout-old.jsonl"),
            session_log(SESSION, day, ""),
        )
        .unwrap();
        fs::write(
            archived.join("rollout-recent.jsonl"),
            session_log(OTHER, cutoff, ""),
        )
        .unwrap();
        let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        let mut scan = Scan::default();
        let names = BTreeMap::new();
        assert!(scan.round(&mut store, &home, day, &names, 1 << 20).complete);
        assert!(
            scan.round(&mut store, &home, cutoff, &names, 1 << 20)
                .complete
        );
        let original_groups: Vec<u8> = store
            .db
            .query_row(
                "SELECT groups FROM token_days WHERE day = ?1",
                [day.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        store
            .db
            .execute(
                "UPDATE token_days SET groups=CAST(json_set(CAST(groups AS TEXT),
                '$[0].provider','proven-relay', '$[0].totals.evidence','requestExact') AS BLOB)
             WHERE day=?1",
                [day.to_string()],
            )
            .unwrap();
        let request = json!({"startAt":stamp(day, 0).timestamp_millis(),
            "endAt":stamp(cutoff, 14).timestamp_millis(), "includeTrend":true});
        let query = Query::parse(&request, cutoff).unwrap();
        let (first, job) = Store::read_response_with_archive(
            &mut store.db,
            &query,
            &request,
            Some(OTHER),
            &names,
            None,
            None,
        )
        .unwrap();
        assert_eq!(first["archiveRecovery"]["status"], "pending");
        assert_eq!(first["days"][0]["usage"]["totalTokens"], 220);
        assert_eq!(first["days"][1]["usage"]["totalTokens"], 220);
        assert_eq!(first["days"][0]["fingerprintCoverage"], "unavailable");
        assert_eq!(first["days"][1]["fingerprintCoverage"], "events");
        let key = job.unwrap();
        let mut replay = key.replay(&store.db, &home, names.clone()).unwrap();
        assert!(replay.step().unwrap());
        let answer = (key, Ok(replay.finish(&store.db).unwrap()));
        let (recovered, _) = Store::read_response_with_archive(
            &mut store.db,
            &query,
            &request,
            Some(OTHER),
            &names,
            Some(&answer),
            None,
        )
        .unwrap();
        assert_eq!(recovered["archiveRecovery"]["status"], "complete");
        assert_eq!(
            recovered["days"], first["days"],
            "the replay never adds to committed summaries"
        );
        assert_eq!(
            recovered["days"][0]["breakdowns"][0]["provider"],
            "proven-relay"
        );
        assert_eq!(recovered["classificationStatus"]["complete"], false);
        let buckets = recovered["trend"]["buckets"].as_array().unwrap();
        assert_eq!(
            buckets
                .iter()
                .filter_map(|b| b["usage"]["totalTokens"].as_u64())
                .sum::<u64>(),
            440
        );
        let archived_group = &buckets
            .iter()
            .find(|b| b["recordCount"].as_u64().unwrap() > 0)
            .unwrap()["breakdowns"][0];
        assert_eq!(archived_group["provider"], "unknown");
        assert!(archived_group.get("classificationEvidence").is_none());
        assert_eq!(
            recovered["scopedUsage"]["days"].as_array().unwrap().len(),
            1
        );
        assert_eq!(recovered["scopedUsage"]["trend"]["precision"], "exact");

        store
            .db
            .execute(
                "UPDATE token_days SET groups=CAST(json_set(CAST(groups AS TEXT),
                '$[0].totals.records',3) AS BLOB) WHERE day=?1",
                [day.to_string()],
            )
            .unwrap();
        let (mismatch, _) = Store::read_response_with_archive(
            &mut store.db,
            &query,
            &request,
            Some(OTHER),
            &names,
            Some(&answer),
            None,
        )
        .unwrap();
        assert_eq!(mismatch["archiveRecovery"]["status"], "partial");
        assert_eq!(mismatch["countStatus"]["complete"], false);
        assert_eq!(mismatch["days"][0]["usage"]["totalTokens"], 220);
        assert!(
            mismatch["trend"].is_null(),
            "missing archive counts cannot become empty buckets"
        );
        assert_eq!(mismatch["scopedUsage"]["countStatus"]["complete"], true);
        assert_eq!(mismatch["scopedUsage"]["trend"]["precision"], "exact");
        assert_eq!(mismatch["days"][0]["fingerprintCoverage"], "unavailable");
        store
            .db
            .execute(
                "UPDATE token_days SET groups = ?1 WHERE day = ?2",
                rusqlite::params![original_groups, day.to_string()],
            )
            .unwrap();
        let (verified, _) = Store::read_response_with_archive(
            &mut store.db,
            &query,
            &request,
            Some(OTHER),
            &names,
            Some(&answer),
            None,
        )
        .unwrap();
        assert_eq!(verified["days"][0]["fingerprintCoverage"], "events");
        assert_eq!(verified["days"][0]["usage"], first["days"][0]["usage"]);
        assert_eq!(
            verified["days"][0]["accountingFingerprint"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(
            verified["days"][0]["accountingFingerprint"],
            verified["days"][0]["breakdowns"][0]["membershipFingerprint"]
        );
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
        let events = replay.finish(&store.db).unwrap().events;
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
        assert_eq!(
            replay.finish(&store.db).unwrap().events[0]
                .delta
                .total_tokens,
            110
        );
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
        let events = replay.finish(&store.db).unwrap().events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].session, SESSION);
        assert_eq!(
            events[0].delta.total_tokens, 110,
            "causal prefix intersects count resolution but not returned scope"
        );
    }
}
