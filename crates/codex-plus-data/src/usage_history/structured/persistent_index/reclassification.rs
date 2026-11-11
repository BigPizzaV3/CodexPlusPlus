//! 后台有界归类；每批重读原始上下文，绝不把上次补证当作本次原始身份。
use super::{
    BUDGET_BLOCKED, PersistentIndex, attribution, checkpoint, decode_source, populate_daily,
};
use crate::usage_history::structured::{
    ContextConflicts, RequestSessionAliases, Tokens, UsageEvent, provider_cache,
};
use anyhow::{Context, ensure};
use chrono::{DateTime, Local};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

const WINDOW_MS: i64 = 600_000;
const TARGET_MS: i64 = 60_000;
const RECENT_MS: i64 = 20 * 60_000;
const MAX_WINDOW_EVENTS: usize = 512;
const MAX_SOURCES: usize = 64;
const MAX_ALIASES: usize = 64;
const MAX_OBSERVATIONS: usize = 64;
const BATCH_TIME: Duration = Duration::from_millis(100);

pub(in crate::usage_history::structured) struct Inputs<'a> {
    pub(in crate::usage_history::structured) home: &'a Path,
    pub(in crate::usage_history::structured) request_log: &'a Path,
    pub(in crate::usage_history::structured) cache: &'a provider_cache::ProviderCache,
    pub(in crate::usage_history::structured) endpoints: &'a BTreeMap<String, Option<String>>,
    pub(in crate::usage_history::structured) endpoint_since: i64,
    pub(in crate::usage_history::structured) provider_names: &'a BTreeMap<String, String>,
    pub(in crate::usage_history::structured) now_ms: i64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::usage_history::structured) struct Report {
    pub(in crate::usage_history::structured) revision: i64,
    pub(in crate::usage_history::structured) checked_events: usize,
    pub(in crate::usage_history::structured) changed_events: usize,
    pub(in crate::usage_history::structured) deferred_windows: usize,
    pub(in crate::usage_history::structured) errors: Vec<String>,
}

#[derive(Default)]
pub(in crate::usage_history::structured) struct Worker {
    classifier: attribution::Classifier,
    recent_cursor: i64,
    historical_cursor: i64,
}

struct Target {
    rowid: i64,
    session: String,
    at: i64,
}

struct Stored {
    id: [u8; 32],
    session: String,
    day: String,
    at: i64,
    model: String,
    provider: String,
    source: Option<String>,
    conflicts: u8,
    uncertain: bool,
    incomplete: bool,
    delta: Tokens,
}

type SourceStamp = (u64, Option<SystemTime>, Option<(u64, u64)>);

impl Worker {
    pub(in crate::usage_history::structured) fn tick(
        &mut self,
        index: &mut PersistentIndex,
        input: &Inputs<'_>,
    ) -> anyhow::Result<Report> {
        ensure!(!index.budget_blocked, "{BUDGET_BLOCKED}");
        let started = Instant::now();
        let mut report = Report {
            revision: index.revision()?,
            ..Report::default()
        };
        let mut visited = HashSet::new();
        // 近期与历史各一个游标；迟到请求不依赖 rollout 再次追加才会被发现。
        for recent in [true, false] {
            if started.elapsed() >= BATCH_TIME {
                report.deferred_windows += 1;
                break;
            }
            let cursor = if recent {
                &mut self.recent_cursor
            } else {
                &mut self.historical_cursor
            };
            let mut statement = index.db.prepare(
                "SELECT rowid, session, timestamp_ms FROM usage_events
                 WHERE rowid > ?1 AND (?2 = 0 OR timestamp_ms >= ?3)
                   AND NOT EXISTS (
                     SELECT 1 FROM usage_day_only d
                     WHERE d.day = usage_events.day AND d.session = usage_events.session)
                   AND NOT EXISTS (
                     SELECT 1 FROM usage_sealed_sessions s
                     WHERE s.session = usage_events.session AND usage_events.day < s.cutoff)
                 ORDER BY rowid LIMIT ?4",
            )?;
            let targets = statement
                .query_map(
                    params![
                        *cursor,
                        recent,
                        input.now_ms.saturating_sub(RECENT_MS),
                        MAX_WINDOW_EVENTS
                    ],
                    |row| {
                        Ok(Target {
                            rowid: row.get(0)?,
                            session: row.get(1)?,
                            at: row.get(2)?,
                        })
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            let Some(target) = targets.first() else {
                *cursor = 0;
                continue;
            };
            *cursor = target.rowid;
            if !visited.insert((target.session.clone(), target.at)) {
                continue;
            }
            match self.window(index, input, target, started) {
                Ok((checked, changed, revision, warning)) => {
                    report.checked_events += checked;
                    report.changed_events += changed;
                    report.revision = revision;
                    if let Some(warning) = warning {
                        report.deferred_windows += 1;
                        report.errors.push(warning);
                    }
                    let covered = targets
                        .iter()
                        .take_while(|next| {
                            next.session == target.session
                                && next.at >= target.at
                                && next.at < target.at.saturating_add(TARGET_MS)
                        })
                        .last()
                        .expect("target is in its own classification window")
                        .rowid;
                    if recent {
                        self.recent_cursor = covered;
                    } else {
                        self.historical_cursor = covered;
                    }
                }
                Err(error) => {
                    report.deferred_windows += 1;
                    report.errors.push(format!("{error:#}"));
                }
            }
        }
        Ok(report)
    }

    fn window(
        &mut self,
        index: &mut PersistentIndex,
        input: &Inputs<'_>,
        target: &Target,
        started: Instant,
    ) -> anyhow::Result<(usize, usize, i64, Option<String>)> {
        let tx = index.db.transaction()?;
        let mut aliases = BTreeMap::new();
        let mut members = BTreeSet::from([target.session.clone()]);
        loop {
            let before = members.len();
            let mut statement = tx.prepare_cached(
                "SELECT alias, session FROM usage_scope_aliases
                 WHERE alias = ?1 OR session = ?1 LIMIT ?2",
            )?;
            for member in members.clone() {
                for (alias, session) in input.cache.aliases_for(&member, MAX_ALIASES)? {
                    if let Some(previous) = aliases.insert(alias.clone(), session.clone()) {
                        ensure!(previous == session, "conflicting captured session alias");
                    }
                    members.insert(alias);
                    members.insert(session);
                }
                ensure!(
                    members.len() <= MAX_ALIASES,
                    "usage alias window exceeds batch limit"
                );
                let rows = statement.query_map(params![member, MAX_ALIASES + 1], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (alias, session) = row?;
                    if let Some(previous) = aliases.insert(alias.clone(), session.clone()) {
                        ensure!(previous == session, "conflicting usage session alias");
                    }
                    members.insert(alias);
                    members.insert(session);
                    ensure!(
                        members.len() <= MAX_ALIASES,
                        "usage alias window exceeds batch limit"
                    );
                }
            }
            ensure!(
                started.elapsed() < BATCH_TIME,
                "classification batch time exhausted"
            );
            if members.len() == before {
                break;
            }
        }
        let canonical = RequestSessionAliases::new(&aliases)
            .resolve(&target.session)
            .context("usage session alias is ambiguous")?;
        let mut sources = BTreeSet::new();
        let mut direct_sources = BTreeSet::new();
        let mut stored = Vec::new();
        let from = target.at.saturating_sub(WINDOW_MS);
        let target_until = target.at.saturating_add(TARGET_MS);
        let until = target_until.saturating_add(WINDOW_MS);
        for member in &members {
            let mut statement = tx.prepare_cached(
                "SELECT source, 1 FROM usage_sources WHERE scope_session = ?1
                 UNION SELECT source, 1 FROM usage_observations WHERE session = ?1
                 UNION SELECT source, 1 FROM usage_count_gaps WHERE session = ?1
                 UNION SELECT source, session = ?1 FROM usage_scope_parents
                   WHERE session = ?1 OR parent = ?1
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(params![member, MAX_SOURCES * 2 + 1], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, bool>(1)?))
            })?;
            for source in rows {
                let (source, direct) = source?;
                if direct {
                    direct_sources.insert(source.clone());
                }
                sources.insert(source);
                ensure!(
                    sources.len() <= MAX_SOURCES,
                    "usage source window exceeds batch limit"
                );
            }
            let mut statement = tx.prepare_cached(
                "SELECT rowid, id, session, day, timestamp_ms, model, provider, provider_source,
                    context_conflicts, count_uncertain, count_incomplete,
                    input, cached, output, reasoning
                 FROM usage_events
                 WHERE session = ?1 AND day >= ?2 AND day <= ?3
                   AND timestamp_ms >= ?4 AND timestamp_ms <= ?5
                 ORDER BY timestamp_ms, rowid LIMIT ?6",
            )?;
            let mut rows = statement.query(params![
                member,
                DateTime::from_timestamp_millis(from)
                    .context("invalid window start")?
                    .with_timezone(&Local)
                    .date_naive()
                    .to_string(),
                DateTime::from_timestamp_millis(until)
                    .context("invalid window end")?
                    .with_timezone(&Local)
                    .date_naive()
                    .to_string(),
                from,
                until,
                MAX_WINDOW_EVENTS + 1,
            ])?;
            while let Some(row) = rows.next()? {
                let id: Vec<u8> = row.get(1)?;
                let cached: Option<u64> = row.get(12)?;
                let mut delta = Tokens::new(row.get(11)?, cached.unwrap_or(0), row.get(13)?);
                delta.cached_known = cached.is_some();
                delta.reasoning_output_tokens = row.get(14)?;
                stored.push(Stored {
                    id: id
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("invalid usage identity"))?,
                    session: row.get(2)?,
                    day: row.get(3)?,
                    at: row.get(4)?,
                    model: row.get(5)?,
                    provider: row.get(6)?,
                    source: row.get(7)?,
                    conflicts: row.get(8)?,
                    uncertain: row.get(9)?,
                    incomplete: row.get(10)?,
                    delta,
                });
                ensure!(
                    stored.len() <= MAX_WINDOW_EVENTS,
                    "usage competition window exceeds batch limit"
                );
            }
        }
        let mut stamps = Vec::new();
        let mut inherited_uncertain = HashMap::new();
        for source in &sources {
            let (bytes, complete, valid): (Vec<u8>, bool, bool) = tx.query_row(
                "SELECT parser_state, complete, scope_valid FROM usage_sources WHERE source = ?1",
                [source],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            let state = checkpoint::restore(&bytes)?;
            // 普通子会话按自身身份匹配请求；没有复制父历史时不参与父会话的归类竞争。
            if !direct_sources.contains(source) && valid && state.history_boundary.parent.is_none() {
                continue;
            }
            let recovery: Option<String> = tx.query_row(
                "SELECT error FROM usage_unindexed_checkpoints WHERE source = ?1",
                [source], |row| row.get(0),
            ).optional()?;
            if let Some(error) = recovery {
                anyhow::bail!("usage source requires checkpoint recovery: {error}");
            }
            ensure!(
                complete && valid,
                "related usage source is not fully indexed"
            );
            let path = decode_source(source)?;
            let stamp = state.stamp.context("usage source has no committed stamp")?;
            ensure!(
                source_stamp(&path)? == stamp,
                "related usage source has unread changes"
            );
            stamps.push((path, stamp));
            if let Some(parent) = state.history_boundary.parent
                && members.contains(&parent)
            {
                let mut statement = tx.prepare_cached(
                    "SELECT timestamp_ms FROM usage_events
                     WHERE session = ?1 AND count_incomplete != 0
                       AND day >= ?2 AND day <= ?3
                       AND timestamp_ms >= ?4 AND timestamp_ms <= ?5 LIMIT ?6",
                )?;
                let rows = statement.query_map(
                    params![
                        state.session,
                        DateTime::from_timestamp_millis(from)
                            .unwrap()
                            .with_timezone(&Local)
                            .date_naive()
                            .to_string(),
                        DateTime::from_timestamp_millis(until)
                            .unwrap()
                            .with_timezone(&Local)
                            .date_naive()
                            .to_string(),
                        from,
                        until,
                        MAX_WINDOW_EVENTS + 1,
                    ],
                    |row| row.get::<_, i64>(0),
                )?;
                let (times, missing) = inherited_uncertain
                    .entry(parent)
                    .or_insert_with(|| (Vec::new(), false));
                for time in rows {
                    times.push(time?);
                    ensure!(
                        times.len() <= MAX_WINDOW_EVENTS,
                        "inherited competition window exceeds batch limit"
                    );
                }
                *missing |= state.unlocalized_invalid_counts;
            }
            ensure!(
                started.elapsed() < BATCH_TIME,
                "classification batch time exhausted"
            );
        }
        let mut events = Vec::with_capacity(stored.len());
        let mut collisions = HashSet::new();
        for saved in &stored {
            let mut statement = tx.prepare_cached(
                "SELECT payload FROM usage_observations
                 WHERE session = ?1 AND source = ?2 AND event_id = ?3 LIMIT ?4",
            )?;
            let mut restored: Option<UsageEvent> = None;
            let mut observations = 0;
            for source in &sources {
                let mut rows = statement.query(params![
                    saved.session,
                    source,
                    saved.id.as_slice(),
                    MAX_OBSERVATIONS - observations + 1,
                ])?;
                while let Some(row) = rows.next()? {
                    observations += 1;
                    ensure!(
                        observations <= MAX_OBSERVATIONS,
                        "usage duplicate window exceeds batch limit"
                    );
                    let payload: Vec<u8> = row.get(0)?;
                    let event =
                        checkpoint::restore_event(saved.id, saved.session.clone(), &payload)?;
                    ensure!(
                        event.original_id() == saved.id,
                        "usage observation identity mismatch"
                    );
                    match &mut restored {
                        Some(previous) if previous.inherited && !event.inherited => {
                            *previous = event
                        }
                        Some(previous) if previous.inherited == event.inherited => {
                            previous.total.cached_known &= event.total.cached_known;
                            if let (Some(last), Some(other)) = (&mut previous.last, event.last) {
                                last.cached_known &= other.cached_known;
                            }
                            previous.merge_duplicate_context(&event, input.provider_names);
                        }
                        None => restored = Some(event),
                        _ => {}
                    }
                }
            }
            let mut event = restored.context("usage original context is unavailable")?;
            event.delta = saved.delta;
            event.count_uncertain = saved.uncertain;
            event.count_incomplete = saved.incomplete;
            event.provider =
                super::super::usage_provider(&event.model, &event.provider, input.provider_names)
                    .to_owned();
            let collision: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM usage_collision_families WHERE id = ?1)",
                [event.original_id().as_slice()],
                |row| row.get(0),
            )?;
            if collision {
                collisions.insert(event.original_id());
            }
            events.push(event);
            ensure!(
                started.elapsed() < BATCH_TIME,
                "classification batch time exhausted"
            );
        }
        let mut names = input.provider_names.clone();
        let ready = HashSet::from([canonical]);
        let result = self.classifier.attribute(
            &mut events,
            &mut names,
            &attribution::Inputs {
                home: input.home,
                request_log: input.request_log,
                cache: input.cache,
                aliases: &aliases,
                collision_families: &collisions,
                inherited_uncertain: &inherited_uncertain,
                endpoints: input.endpoints,
                endpoint_since: input.endpoint_since,
                ready_sessions: &ready,
            },
        );
        let proxy_error = result.proxy_error.or_else(|| {
            result
                .proxy
                .as_ref()
                .filter(|value| value["capturedLimited"] == true)
                .map(|_| "request competition evidence exceeds batch limit".into())
        });
        let native_available = result
            .native
            .as_ref()
            .is_some_and(|value| value["unavailable"] != true && value["limited"] != true);
        if !native_available && let Some(error) = &proxy_error {
            anyhow::bail!("proxy classification deferred: {error}");
        }
        let warning = proxy_error.clone().or_else(|| {
            (!input.endpoints.is_empty() && !native_available)
                .then(|| "native classification evidence is unavailable or incomplete".into())
        });
        for (path, stamp) in stamps {
            ensure!(
                source_stamp(&path)? == stamp,
                "usage source changed during classification"
            );
        }
        let mut changed = 0;
        let mut affected = BTreeSet::new();
        for (saved, event) in stored.iter().zip(&events) {
            // 只提交中心目标；外围事件仅用于证明唯一性，其外围不一定完整。
            if saved.at < target.at || saved.at >= target_until || saved.session != target.session {
                continue;
            }
            if proxy_error.is_some()
                && event.provider_source.as_deref() != Some("native-http-endpoint")
            {
                continue;
            }
            if !native_available
                && saved.source.as_deref() == Some("native-http-endpoint")
                && super::super::unknown_provider(&event.provider)
            {
                continue;
            }
            let conflicts = conflict_bits(event.context_conflicts);
            if saved.model == event.model
                && saved.provider == event.provider
                && saved.source == event.provider_source
                && saved.conflicts == conflicts
            {
                continue;
            }
            let day_only: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM usage_day_only WHERE session = ?1 AND day = ?2)
                     OR EXISTS(SELECT 1 FROM usage_sealed_sessions WHERE session = ?1 AND ?2 < cutoff)",
                params![saved.session, saved.day], |row| row.get(0),
            )?;
            ensure!(!day_only, "usage day was compacted before classification");
            tx.execute(
                "UPDATE usage_events SET model = ?2, provider = ?3, provider_source = ?4,
                    context_conflicts = ?5 WHERE id = ?1",
                params![
                    saved.id.as_slice(),
                    event.model,
                    event.provider,
                    event.provider_source,
                    conflicts
                ],
            )?;
            affected.insert((saved.session.clone(), saved.day.clone()));
            changed += 1;
        }
        for (session, day) in affected {
            tx.execute(
                "DELETE FROM usage_daily WHERE session = ?1 AND day = ?2",
                params![session, day],
            )?;
            populate_daily(&tx, Some((&session, &day)))?;
        }
        if changed != 0 {
            tx.execute(
                "UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1",
                [],
            )?;
        }
        let revision = tx.query_row(
            "SELECT revision FROM usage_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok((stored.len(), changed, revision, warning))
    }
}

fn conflict_bits(value: ContextConflicts) -> u8 {
    u8::from(value.model)
        | u8::from(value.provider) << 1
        | u8::from(value.role) << 2
        | u8::from(value.turn) << 3
}

fn source_stamp(path: &Path) -> anyhow::Result<SourceStamp> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(metadata.is_file(), "usage source is not a regular file");
    Ok((
        metadata.len(),
        metadata.modified().ok(),
        super::super::observed_file_identity(path, &metadata)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_history::structured::{
        FileState, RequestEvidence, RequestEvidenceUsage, index_budget,
    };
    use chrono::{Days, NaiveDate};
    use serde_json::json;
    use std::io::Write;
    use std::path::PathBuf;

    const SESSION: &str = "00000000-0000-4000-8000-000000000010";

    struct Fixture {
        root: tempfile::TempDir,
        source: PathBuf,
        log: PathBuf,
        at: i64,
        index: PersistentIndex,
    }

    impl Fixture {
        fn new(events: usize) -> Self {
            Self::with_context(events, "unknown", "custom")
        }

        fn with_context(events: usize, model: &str, provider: &str) -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("rollout.jsonl");
            let log = root.path().join("request.jsonl");
            let at = Local::now().timestamp_millis() - 1_000;
            let mut contents = format!(
                "{}\n{}\n",
                json!({"type":"session_meta","payload":{"id":SESSION}}),
                json!({"type":"turn_context",
                    "timestamp":DateTime::from_timestamp_millis(at - 1).unwrap().to_rfc3339(),
                    "payload":{"turn_id":"turn-a","model":model,"model_provider":provider}}),
            );
            for position in 0..events {
                contents.push_str(&format!("{}\n", json!({
                    "type":"event_msg",
                    "timestamp":DateTime::from_timestamp_millis(at + position as i64).unwrap().to_rfc3339(),
                    "payload":{"type":"token_count","info":{
                        "total_token_usage":{"input_tokens":100 * (position + 1),
                            "cached_input_tokens":20 * (position + 1),"output_tokens":10 * (position + 1)},
                        "last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10}
                    }}
                })));
            }
            fs::write(&source, contents).unwrap();
            let source = fs::canonicalize(source).unwrap();
            let mut index = PersistentIndex::open(&root.path().join("usage.sqlite")).unwrap();
            let mut state = FileState::default();
            let mut budget = index_budget::Budget::new(std::iter::empty());
            assert!(
                super::super::scan::scan_source(
                    &mut index,
                    &mut state,
                    &source,
                    NaiveDate::MIN,
                    &mut budget,
                    |tx, sessions| super::super::resolve::resolve(tx, sessions, &BTreeMap::new()),
                )
                .unwrap()
            );
            assert!(!state.unlocalized_invalid_counts);
            let mut cache = provider_cache::ProviderCache::default();
            cache
                .open(root.path(), Local::now().date_naive() - Days::new(61))
                .unwrap();
            fs::write(&log, "").unwrap();
            Self {
                root,
                source,
                log,
                at,
                index,
            }
        }

        fn request(&self, provider: &str) {
            let request = RequestEvidence {
                id: "request-a".into(),
                thread_id: SESSION.into(),
                model: "model-a".into(),
                upstream_model: "model-a".into(),
                relay_id: provider.into(),
                relay_name: "Renamed supplier".into(),
                success: true,
                state: "success".into(),
                status_code: 200,
                timestamp_ms: self.at - 10,
                started_at_ms: self.at - 100,
                attempts: Some(1),
                auto_retries: Some(0),
                final_response_evidence: None,
                usage: Some(RequestEvidenceUsage {
                    input_tokens: Some(100),
                    output_tokens: Some(10),
                    cached_tokens: Some(20),
                    cached_tokens_known: None,
                }),
                agent: String::new(),
            };
            fs::write(
                &self.log,
                format!("{}\n", serde_json::to_string(&request).unwrap()),
            )
            .unwrap();
        }

        fn add_source(&mut self, name: &str, contents: &str) {
            let path = self.root.path().join(name);
            fs::write(&path, contents).unwrap();
            let path = fs::canonicalize(path).unwrap();
            let mut state = super::super::scan::restore_source(&self.index, &path).unwrap();
            let mut budget = index_budget::Budget::new(std::iter::empty());
            assert!(
                super::super::scan::scan_source(
                    &mut self.index,
                    &mut state,
                    &path,
                    NaiveDate::MIN,
                    &mut budget,
                    |tx, sessions| super::super::resolve::resolve(tx, sessions, &BTreeMap::new()),
                )
                .unwrap()
            );
        }

        fn tick(&mut self, worker: &mut Worker) -> Report {
            let cache = provider_cache::ProviderCache::read_snapshot(
                self.root.path(),
                Local::now().date_naive() - Days::new(61),
            )
            .unwrap();
            worker
                .tick(
                    &mut self.index,
                    &Inputs {
                        home: self.root.path(),
                        request_log: &self.log,
                        cache: &cache,
                        endpoints: &BTreeMap::new(),
                        endpoint_since: 0,
                        provider_names: &BTreeMap::new(),
                        now_ms: self.at + 1_000,
                    },
                )
                .unwrap()
        }

        fn counts(&self) -> (i64, i64, i64, i64, i64) {
            self.index.db.query_row(
                "SELECT SUM(input), SUM(cached), SUM(output), SUM(records), SUM(total_events) FROM usage_daily",
                [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            ).unwrap()
        }

        fn provider(&self) -> String {
            self.index
                .db
                .query_row(
                    "SELECT provider FROM usage_events ORDER BY rowid LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        }
    }

    #[test]
    fn late_evidence_reclassifies_committed_counts_without_rollout_changes() {
        let mut fixture = Fixture::new(1);
        let counts = fixture.counts();
        let before = fixture.index.revision().unwrap();
        let mut worker = Worker::default();
        let pending = fixture.tick(&mut worker);
        assert_eq!(pending.changed_events, 0);
        assert_eq!(pending.deferred_windows, 1);
        fixture.request("supplier-a");
        let mut changed = 0;
        for _ in 0..3 {
            changed += fixture.tick(&mut worker).changed_events;
        }
        assert_eq!(changed, 1);
        assert_eq!(fixture.provider(), "supplier-a");
        assert_eq!(fixture.counts(), counts);
        assert_eq!(fixture.index.revision().unwrap(), before + 1);
        drop(worker);
        let restarted = fixture.tick(&mut Worker::default());
        assert_eq!(restarted.changed_events, 0);
        assert_eq!(fixture.counts(), counts);
    }

    #[test]
    fn known_requested_model_follows_unique_actual_request_and_revokes_on_competition() {
        let mut fixture = Fixture::with_context(1, "model-a", "supplier-a");
        let counts = fixture.counts();
        fixture.request("supplier-a");
        let mut request: RequestEvidence =
            serde_json::from_str(&fs::read_to_string(&fixture.log).unwrap()).unwrap();
        request.upstream_model = "model-b".into();
        fs::write(
            &fixture.log,
            format!("{}\n", serde_json::to_string(&request).unwrap()),
        )
        .unwrap();
        let mut worker = Worker::default();
        for _ in 0..3 {
            fixture.tick(&mut worker);
        }
        let model: String = fixture
            .index
            .db
            .query_row(
                "SELECT model FROM usage_events ORDER BY rowid LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(model, "model-b");
        assert_eq!(fixture.provider(), "supplier-a");
        assert_eq!(fixture.counts(), counts);

        let mut competitor = request.clone();
        competitor.id = "request-b".into();
        fs::write(
            &fixture.log,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&request).unwrap(),
                serde_json::to_string(&competitor).unwrap()
            ),
        )
        .unwrap();
        for _ in 0..3 {
            fixture.tick(&mut worker);
        }
        let model: String = fixture
            .index
            .db
            .query_row(
                "SELECT model FROM usage_events ORDER BY rowid LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(model, "model-a");
        assert_eq!(fixture.provider(), "supplier-a");
        assert_eq!(fixture.counts(), counts);
    }

    #[test]
    fn competing_usage_and_dense_windows_cannot_create_unique_attribution() {
        for count in [2, MAX_WINDOW_EVENTS + 1] {
            let mut fixture = Fixture::new(count);
            fixture.request("supplier-a");
            let before = fixture.counts();
            let report = fixture.tick(&mut Worker::default());
            assert_eq!(report.changed_events, 0);
            assert!(super::super::super::unknown_provider(&fixture.provider()));
            assert_eq!(fixture.counts(), before);
            if count > MAX_WINDOW_EVENTS {
                assert_eq!(report.deferred_windows, 1);
                assert!(report.errors[0].contains("competition window"));
            } else {
                assert!(report.errors.is_empty(), "{:?}", report.errors);
            }
        }
    }

    #[test]
    fn original_context_lookup_includes_duplicate_observations_from_each_source() {
        let mut fixture = Fixture::new(1);
        let before = fixture.counts();
        let mut lines: Vec<serde_json::Value> = fs::read_to_string(&fixture.source)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        lines[1]["payload"]["model"] = json!("model-b");
        lines[1]["payload"]["model_provider"] = json!("supplier-b");
        let contents = lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>();
        fixture.add_source("rollout-copy.jsonl", &contents);
        assert_eq!(fixture.counts(), before);
        assert_eq!(fixture.provider(), "supplier-b");
        fixture.request("supplier-a");
        let report = fixture.tick(&mut Worker::default());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(fixture.provider(), "supplier-b");
        assert_eq!(fixture.counts(), before);
    }

    #[test]
    fn duplicate_limit_is_shared_across_sources_without_using_a_truncated_match() {
        let mut fixture = Fixture::new(1);
        let contents = fs::read_to_string(&fixture.source).unwrap();
        let lines: Vec<_> = contents.lines().collect();
        for (name, count) in [("rollout.jsonl", 32), ("rollout-copy.jsonl", 33)] {
            let mut repeated = format!("{}\n{}\n", lines[0], lines[1]);
            for _ in 0..count {
                repeated.push_str(lines[2]);
                repeated.push('\n');
            }
            fixture.add_source(name, &repeated);
        }
        let observations: usize = fixture
            .index
            .db
            .query_row("SELECT COUNT(*) FROM usage_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(observations, MAX_OBSERVATIONS + 1);
        fixture.request("supplier-a");
        let before = fixture.index.revision().unwrap();
        let report = fixture.tick(&mut Worker::default());
        assert_eq!(report.changed_events, 0);
        assert_eq!(report.deferred_windows, 1);
        assert!(
            report.errors[0].contains("duplicate window"),
            "{:?}",
            report.errors
        );
        assert_eq!(fixture.index.revision().unwrap(), before);
        assert!(super::super::super::unknown_provider(&fixture.provider()));
    }

    #[test]
    fn unread_source_changes_defer_classification_and_keep_counts_readable() {
        let mut fixture = Fixture::new(1);
        fixture.request("supplier-a");
        let before = fixture.counts();
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&fixture.source)
            .unwrap();
        writeln!(
            file,
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"note\"}}}}"
        )
        .unwrap();
        let report = fixture.tick(&mut Worker::default());
        assert_eq!(report.changed_events, 0);
        assert_eq!(report.deferred_windows, 1);
        assert!(report.errors[0].contains("unread changes"));
        assert_eq!(fixture.counts(), before);
    }

    #[test]
    fn unfinished_unrelated_source_does_not_block_committed_window() {
        let mut fixture = Fixture::new(1);
        fixture.request("supplier-a");
        let before = fixture.counts();
        let other = "00000000-0000-4000-8000-000000000011";
        let path = fixture.root.path().join("rollout-unrelated.jsonl");
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                json!({"type":"session_meta","payload":{"id":other}}),
                json!({"type":"event_msg","payload":{"type":"note","text":"x".repeat(4096)}}),
            ),
        )
        .unwrap();
        let path = fs::canonicalize(path).unwrap();
        let mut state = FileState::default();
        let mut budget = index_budget::Budget::new(std::iter::empty());
        budget.read_left = 256;
        budget.source_left = 256;
        assert!(
            !super::super::scan::scan_source(
                &mut fixture.index,
                &mut state,
                &path,
                NaiveDate::MIN,
                &mut budget,
                |tx, sessions| super::super::resolve::resolve(tx, sessions, &BTreeMap::new()),
            )
            .unwrap()
        );
        let complete: bool = fixture
            .index
            .db
            .query_row(
                "SELECT complete FROM usage_sources WHERE scope_session = ?1",
                [other],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!complete);
        let report = fixture.tick(&mut Worker::default());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.changed_events, 1);
        assert_eq!(fixture.provider(), "supplier-a");
        assert_eq!(fixture.counts(), before);
    }

    #[test]
    fn unfinished_child_without_inherited_history_does_not_block_parent_classification() {
        for inherited in [false, true] {
            let mut fixture = Fixture::new(1);
            fixture.request("supplier-a");
            let before = fixture.counts();
            let child = "00000000-0000-4000-8000-000000000011";
            let mut header = json!({"type":"session_meta","payload":{
                "id":child,"source":{"subagent":{"thread_spawn":{"parent_thread_id":SESSION}}}
            }});
            if inherited {
                header["payload"]["forked_from_id"] = json!(SESSION);
                header["payload"]["subagent_history_start_ordinal"] = json!(5);
            }
            let path = fixture.root.path().join("rollout-child.jsonl");
            fs::write(&path, format!(
                "{header}\n{}\n",
                json!({"type":"event_msg","payload":{"type":"note","text":"x".repeat(4096)}}),
            )).unwrap();
            let path = fs::canonicalize(path).unwrap();
            let mut state = FileState::default();
            let mut budget = index_budget::Budget::new(std::iter::empty());
            budget.read_left = 1024;
            budget.source_left = 1024;
            assert!(!super::super::scan::scan_source(
                &mut fixture.index, &mut state, &path, NaiveDate::MIN, &mut budget,
                |tx, sessions| super::super::resolve::resolve(tx, sessions, &BTreeMap::new()),
            ).unwrap());
            if !inherited {
                fixture.index.db.execute(
                    "INSERT INTO usage_unindexed_checkpoints(source, error) VALUES (?1, 'child recovery')",
                    [super::super::scan::encode_source(&path)],
                ).unwrap();
            }
            let report = fixture.tick(&mut Worker::default());
            if inherited {
                assert_eq!(report.changed_events, 0);
                assert!(report.errors[0].contains("not fully indexed"), "{:?}", report.errors);
                assert!(super::super::super::unknown_provider(&fixture.provider()));
            } else {
                assert!(report.errors.is_empty(), "{:?}", report.errors);
                assert_eq!(report.changed_events, 1);
                assert_eq!(fixture.provider(), "supplier-a");
            }
            assert_eq!(fixture.counts(), before);
        }
    }

    #[test]
    fn direct_checkpoint_recovery_still_blocks_classification() {
        let mut fixture = Fixture::new(1);
        fixture.request("supplier-a");
        let before = fixture.counts();
        fixture.index.db.execute(
            "INSERT INTO usage_unindexed_checkpoints(source, error) VALUES (?1, 'direct recovery')",
            [super::super::scan::encode_source(&fixture.source)],
        ).unwrap();
        let report = fixture.tick(&mut Worker::default());
        assert_eq!(report.changed_events, 0);
        assert!(report.errors[0].contains("direct recovery"));
        assert_eq!(fixture.counts(), before);
    }

    #[test]
    fn classification_write_failure_rolls_back_event_daily_and_revision() {
        let mut fixture = Fixture::new(1);
        fixture.request("supplier-a");
        let before = fixture.counts();
        let revision = fixture.index.revision().unwrap();
        fixture
            .index
            .db
            .execute_batch(
                "CREATE TRIGGER reject_classification BEFORE UPDATE OF provider ON usage_events
             BEGIN SELECT RAISE(ABORT, 'synthetic classification write failure'); END;",
            )
            .unwrap();
        let report = fixture.tick(&mut Worker::default());
        assert_eq!(report.changed_events, 0);
        assert_eq!(report.deferred_windows, 1);
        assert!(report.errors[0].contains("synthetic classification write failure"));
        assert!(super::super::super::unknown_provider(&fixture.provider()));
        assert_eq!(fixture.counts(), before);
        assert_eq!(fixture.index.revision().unwrap(), revision);
        fixture
            .index
            .db
            .execute_batch("DROP TRIGGER reject_classification;")
            .unwrap();
        assert_eq!(fixture.tick(&mut Worker::default()).changed_events, 1);
        assert_eq!(fixture.counts(), before);
    }

    #[test]
    fn compacted_day_is_never_rebuilt_from_remaining_detail_rows() {
        let mut fixture = Fixture::new(1);
        fixture.request("supplier-a");
        fixture
            .index
            .db
            .execute(
                "INSERT INTO usage_day_only SELECT DISTINCT day, session FROM usage_events",
                [],
            )
            .unwrap();
        fixture
            .index
            .db
            .execute("UPDATE usage_daily SET input = input + 999", [])
            .unwrap();
        let before = fixture.counts();
        let report = fixture.tick(&mut Worker::default());
        assert_eq!(report.changed_events, 0);
        assert_eq!(report.checked_events, 0);
        assert_eq!(fixture.counts(), before);
    }

    #[test]
    fn sealed_session_is_skipped_before_its_detail_deletion_finishes() {
        let mut fixture = Fixture::new(1);
        fixture.request("supplier-a");
        fixture
            .index
            .db
            .execute(
                "INSERT INTO usage_sealed_sessions(session, source, cutoff, anchor_position)
             SELECT scope_session, source, ?1, 0 FROM usage_sources",
                [(Local::now().date_naive() + Days::new(1)).to_string()],
            )
            .unwrap();
        let before = fixture.counts();
        let report = fixture.tick(&mut Worker::default());
        assert_eq!(report.changed_events, 0);
        assert_eq!(report.checked_events, 0);
        assert_eq!(fixture.counts(), before);
    }
}
