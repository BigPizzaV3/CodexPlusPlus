//! 单来源有序追加的有界归并；其他情况交给完整归并。
use super::{Resolution, ResolvedEvent, SourceBatch, checkpoint, resolve};
use crate::usage_history::structured::{UsageEvent, usage_provider};
use anyhow::{Context, ensure};
use chrono::DateTime;
use rusqlite::Transaction;
use std::collections::{BTreeMap, BTreeSet};

const MAX_CACHED_SESSIONS: usize = 64;

struct Cursor {
    source: Vec<u8>,
    last: UsageEvent,
}

#[derive(Default)]
pub(super) struct Appender {
    revision: Option<i64>,
    sessions: BTreeMap<String, Cursor>,
}

impl Appender {
    pub(super) fn resolve(
        &mut self,
        tx: &Transaction<'_>,
        sessions: &BTreeSet<String>,
        batch: &SourceBatch,
        provider_names: &BTreeMap<String, String>,
    ) -> anyhow::Result<Resolution> {
        let revision: i64 = tx.query_row(
            "SELECT revision FROM usage_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        if self.revision != Some(revision) {
            self.sessions.clear();
            self.revision = Some(revision);
        }
        let next_revision = revision.checked_add(1).context("usage revision overflow")?;
        if let Some(session) = sessions.first().filter(|_| sessions.len() == 1) {
            if let Some((events, cursor)) = self.try_append(tx, session, batch, provider_names)? {
                if self.sessions.len() >= MAX_CACHED_SESSIONS && !self.sessions.contains_key(session)
                {
                    self.sessions.clear();
                }
                self.sessions.insert(session.clone(), cursor);
                // 若后续校验或提交失败，数据库修订不会递增，下次会清空此缓存。
                self.revision = Some(next_revision);
                return Ok(Resolution::Append(events));
            }
        }
        for session in sessions {
            self.sessions.remove(session);
        }
        let events = resolve::resolve(tx, sessions, provider_names)?;
        self.revision = Some(next_revision);
        Ok(Resolution::Full(events))
    }

    fn try_append(
        &self,
        tx: &Transaction<'_>,
        session: &str,
        batch: &SourceBatch,
        provider_names: &BTreeMap<String, String>,
    ) -> anyhow::Result<Option<(Vec<ResolvedEvent>, Cursor)>> {
        if batch.replace
            || batch.observations.is_empty()
            || batch.count_gap_sessions.is_some()
            || !batch.scope_valid
            || batch.scope_session != session
            || batch.observations.iter().any(|observation| {
                observation.session != session || observation.position < batch.expected_offset
            })
        {
            return Ok(None);
        }
        resolve::check_session_checkpoints(tx, session)?;
        let active_gap: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM usage_count_gaps WHERE session = ?1)",
            [session],
            |row| row.get(0),
        )?;
        if active_gap {
            return Ok(None);
        }
        let mut last = match self.sessions.get(session) {
            Some(cursor) if cursor.source == batch.source => Some(cursor.last.clone()),
            Some(_) => return Ok(None),
            None => {
                let collision: bool = tx.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM usage_observations o
                        JOIN usage_collision_families f ON f.id = o.event_id
                        WHERE o.session = ?1
                     )",
                    [session],
                    |row| row.get(0),
                )?;
                if collision {
                    return Ok(None);
                }
                let mut statement = tx.prepare_cached(
                    "SELECT source, position, event_id, payload FROM usage_observations
                     WHERE session = ?1 ORDER BY source, position",
                )?;
                let mut rows = statement.query([session])?;
                let mut previous: Option<UsageEvent> = None;
                let mut previous_timestamp = None;
                while let Some(row) = rows.next()? {
                    let source: Vec<u8> = row.get(0)?;
                    if source != batch.source {
                        return Ok(None);
                    }
                    let position: i64 = row.get(1)?;
                    if u64::try_from(position)? >= batch.expected_offset {
                        continue;
                    }
                    let id: Vec<u8> = row.get(2)?;
                    let id: [u8; 32] = id
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("invalid stored usage identity"))?;
                    let payload: Vec<u8> = row.get(3)?;
                    let event = checkpoint::restore_event(id, session.to_owned(), &payload)?;
                    ensure!(
                        event.original_id() == id,
                        "usage observation identity mismatch"
                    );
                    let timestamp = DateTime::parse_from_rfc3339(&event.timestamp)
                        .context("invalid usage event timestamp")?;
                    if !eligible(&event)
                        || previous_timestamp.is_some_and(|before| timestamp <= before)
                    {
                        return Ok(None);
                    }
                    previous_timestamp = Some(timestamp);
                    previous = Some(event);
                }
                if previous.is_none() {
                    let existing_events: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM usage_events WHERE session = ?1)",
                        [session],
                        |row| row.get(0),
                    )?;
                    if existing_events {
                        return Ok(None);
                    }
                }
                previous
            }
        };
        let mut events = Vec::with_capacity(batch.observations.len());
        let mut previous_position = None;
        for observation in &batch.observations {
            if previous_position.is_some_and(|position| position >= observation.position) {
                return Ok(None);
            }
            previous_position = Some(observation.position);
            let mut event = checkpoint::restore_event(
                observation.event_id,
                session.to_owned(),
                &observation.payload,
            )?;
            ensure!(
                event.original_id() == observation.event_id,
                "usage observation identity mismatch"
            );
            let timestamp = DateTime::parse_from_rfc3339(&event.timestamp)
                .context("invalid usage event timestamp")?;
            if !eligible(&event) {
                return Ok(None);
            }
            if let Some(before) = &last {
                let before_timestamp = DateTime::parse_from_rfc3339(&before.timestamp)
                    .context("invalid previous usage event timestamp")?;
                if timestamp <= before_timestamp || corrects_previous_component(before, &event) {
                    return Ok(None);
                }
            }
            let previous = last.as_ref().map(|before| before.total);
            event.count_incomplete |=
                previous.is_none() && !event.has_file_baseline && event.last.is_none();
            event.count_uncertain |= event.last.is_none()
                && previous.is_some_and(|before| event.total.requires_last_after(before));
            event.delta = event.session_delta(previous);
            event.provider =
                usage_provider(&event.model, &event.provider, provider_names).to_owned();
            if event.delta.total_tokens > 0
                || event.count_uncertain
                || event.count_incomplete
                || event.context_conflicts.any()
            {
                let timestamp_ms = timestamp.timestamp_millis();
                events.push(ResolvedEvent {
                    id: event.id,
                    session: event.session.clone(),
                    day: event.date.to_string(),
                    timestamp_ms,
                    model: event.model.clone(),
                    provider: event.provider.clone(),
                    provider_source: event.provider_source.clone(),
                    role: event.role.clone(),
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
            last = Some(event);
        }
        Ok(Some((
            events,
            Cursor {
                source: batch.source.clone(),
                last: last.context("incremental batch has no events")?,
            },
        )))
    }
}

fn eligible(event: &UsageEvent) -> bool {
    !event.inherited
        && !event.context_conflicts.any()
        && !event.provider.trim().eq_ignore_ascii_case("openai")
}

fn corrects_previous_component(before: &UsageEvent, after: &UsageEvent) -> bool {
    before.total.input_tokens == after.total.input_tokens
        && before.total.output_tokens == after.total.output_tokens
        && (before.total.cached_known
            && (!after.total.cached_known
                || before.total.cached_input_tokens != after.total.cached_input_tokens)
            || before.total.reasoning_output_tokens.is_some()
                && before.total.reasoning_output_tokens != after.total.reasoning_output_tokens)
}
