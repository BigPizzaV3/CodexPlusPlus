//! 封存已完成的单来源旧会话；旧日汇总冻结，最后一条累计观察继续充当追加基线。
use super::{Resolution, SourceBatch, checkpoint};
use crate::usage_history::structured::UsageEvent;
use anyhow::Context;
use chrono::{DateTime, Days, Local};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

const BATCH_ROWS: usize = 256;

#[derive(Clone)]
pub(super) struct Cursor {
    after_source: Vec<u8>,
    work: Option<Work>,
}

#[derive(Clone)]
enum Work {
    Validate(Validation),
    Prune(Seal),
}

#[derive(Clone)]
struct Validation {
    source: Vec<u8>,
    identity: Vec<u8>,
    offset: i64,
    parser_state: Vec<u8>,
    session: String,
    cutoff: String,
    last: Option<(i64, i64, UsageEvent)>,
}

#[derive(Clone)]
struct Seal {
    source: Vec<u8>,
    range: SealedRange,
    anchor_position: i64,
    review_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SealedRange {
    pub(super) session: String,
    pub(super) cutoff: String,
}

impl SealedRange {
    pub(super) fn contains(&self, session: &str, day: &str) -> bool {
        self.session == session && day < self.cutoff.as_str()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Review {
    pub(super) session: String,
    pub(super) reason: String,
}

pub(super) struct Batch {
    pub(super) next: Option<Cursor>,
    pub(super) scanned: usize,
    pub(super) deleted: usize,
    pub(super) sealed: usize,
    pub(super) changed: bool,
}

pub(super) fn initialize(tx: &Transaction<'_>) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_sealed_sessions (
            session TEXT PRIMARY KEY,
            source BLOB NOT NULL UNIQUE REFERENCES usage_sources(source),
            cutoff TEXT NOT NULL,
            anchor_position INTEGER NOT NULL,
            review_reason TEXT
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS usage_sources_scope_session
         ON usage_sources(scope_session, source);",
    )?;
    Ok(())
}

fn seal_for_session(tx: &Transaction<'_>, session: &str) -> anyhow::Result<Option<Seal>> {
    Ok(tx
        .query_row(
            "SELECT source, cutoff, anchor_position, review_reason
             FROM usage_sealed_sessions WHERE session = ?1",
            [session],
            |row| {
                Ok(Seal {
                    source: row.get(0)?,
                    range: SealedRange {
                        session: session.to_owned(),
                        cutoff: row.get(1)?,
                    },
                    anchor_position: row.get(2)?,
                    review_reason: row.get(3)?,
                })
            },
        )
        .optional()?)
}

fn eligible_source(tx: &Transaction<'_>, validation: &Validation) -> anyhow::Result<bool> {
    let state = checkpoint::restore(&validation.parser_state)?;
    if state.session != validation.session
        || state.identity_conflict
        || state.invalid_counts
        || state.unlocalized_invalid_counts
        || state.pending_count_gap
        || state.issues != 0
        || state.continuation
        || state.history_boundary.parent.is_some()
        || !state.inherited_contexts.is_empty()
    {
        return Ok(false);
    }
    let related: bool = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM usage_sources
            WHERE scope_session = ?1 AND source < ?2
            UNION ALL
            SELECT 1 FROM usage_sources
            WHERE scope_session = ?1 AND source > ?2
            UNION ALL
            SELECT 1 FROM usage_observations WHERE session = ?1 AND source < ?2
            UNION ALL
            SELECT 1 FROM usage_observations WHERE session = ?1 AND source > ?2
            UNION ALL
            SELECT 1 FROM usage_count_gaps WHERE source = ?2
            UNION ALL
            SELECT 1 FROM usage_unindexed_checkpoints WHERE source = ?2
            UNION ALL
            SELECT 1 FROM usage_scope_parents
            WHERE source = ?2 AND (session != ?1 OR parent != '')
            UNION ALL
            SELECT 1 FROM usage_scope_aliases
            WHERE source = ?2 AND (alias != ?1 OR session != ?1)
            UNION ALL
            SELECT 1 FROM usage_scope_aliases WHERE alias = ?1 AND session != ?1
         )",
        params![validation.session, validation.source],
        |row| row.get(0),
    )?;
    Ok(!related)
}

fn eligible_event(event: &UsageEvent) -> bool {
    !event.inherited
        && !event.count_uncertain
        && !event.count_incomplete
        && !event.context_conflicts.any()
}

pub(super) fn compact(
    tx: &Transaction<'_>,
    now_ms: i64,
    cursor: Option<&Cursor>,
) -> anyhow::Result<Batch> {
    let cutoff = DateTime::from_timestamp_millis(now_ms)
        .context("usage retention time is out of range")?
        .with_timezone(&Local)
        .date_naive()
        .checked_sub_days(Days::new(62))
        .context("usage retention day is out of range")?
        .to_string();
    let mut cursor = cursor.cloned().unwrap_or(Cursor {
        after_source: Vec::new(),
        work: None,
    });
    let mut result = Batch {
        next: None,
        scanned: 0,
        deleted: 0,
        sealed: 0,
        changed: false,
    };
    if cursor.work.is_none() {
        let source: Option<(Vec<u8>, Vec<u8>, i64, Vec<u8>, String, bool)> = tx
            .query_row(
                "SELECT source, identity, offset, parser_state, scope_session,
                        complete AND scope_indexed AND scope_valid
                 FROM usage_sources WHERE source > ?1 ORDER BY source LIMIT 1",
                [&cursor.after_source],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((source, identity, offset, parser_state, session, ready)) = source else {
            return Ok(result);
        };
        cursor.after_source.clone_from(&source);
        if let Some(seal) = seal_for_session(tx, &session)?.filter(|seal| seal.source == source) {
            cursor.work = Some(Work::Prune(seal));
        } else if ready && !session.is_empty() {
            let validation = Validation {
                source,
                identity,
                offset,
                parser_state,
                session,
                cutoff,
                last: None,
            };
            if eligible_source(tx, &validation)? {
                cursor.work = Some(Work::Validate(validation));
            }
        }
    }
    match cursor.work.take() {
        None => {}
        Some(Work::Validate(mut validation)) => {
            let unchanged: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM usage_sources
                 WHERE source = ?1 AND identity = ?2 AND offset = ?3 AND parser_state = ?4
                   AND complete = 1 AND scope_indexed = 1 AND scope_valid = 1)",
                params![
                    validation.source,
                    validation.identity,
                    validation.offset,
                    validation.parser_state
                ],
                |row| row.get(0),
            )?;
            if !unchanged {
                result.next = Some(cursor);
                return Ok(result);
            }
            let mut statement = tx.prepare_cached(
                "SELECT position, event_id, session, payload FROM usage_observations
                 WHERE source = ?1 AND position > ?2 ORDER BY position LIMIT ?3",
            )?;
            let mut rows = statement.query(params![
                validation.source,
                validation.last.as_ref().map_or(-1, |last| last.0),
                BATCH_ROWS as i64
            ])?;
            let mut eligible = true;
            let mut collision = tx.prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM usage_collision_families WHERE id = ?1)",
            )?;
            while let Some(row) = rows.next()? {
                result.scanned += 1;
                let position: i64 = row.get(0)?;
                let id: Vec<u8> = row.get(1)?;
                let id: [u8; 32] = id
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid stored usage identity"))?;
                let session: String = row.get(2)?;
                let payload: Vec<u8> = row.get(3)?;
                let event = checkpoint::restore_event(id, session, &payload)?;
                let at = DateTime::parse_from_rfc3339(&event.timestamp)
                    .context("invalid usage event timestamp")?
                    .timestamp_millis();
                let collided: bool = collision.query_row([id.as_slice()], |row| row.get(0))?;
                let follows = validation
                    .last
                    .as_ref()
                    .is_none_or(|(_, before_at, before)| {
                        at > *before_at
                            && event.total.is_monotonic_after(before.total)
                            && (event.total.input_tokens != before.total.input_tokens
                                || event.total.output_tokens != before.total.output_tokens
                                || event.total == before.total)
                    });
                if event.session != validation.session
                    || event.original_id() != id
                    || event.date.to_string() >= validation.cutoff
                    || !eligible_event(&event)
                    || !follows
                    || collided
                    || validation.last.is_none() && !event.has_file_baseline && event.last.is_none()
                {
                    eligible = false;
                    break;
                }
                validation.last = Some((position, at, event));
            }
            drop(rows);
            drop(statement);
            if eligible && result.scanned == BATCH_ROWS {
                cursor.work = Some(Work::Validate(validation));
            } else if eligible {
                if let Some((position, _, event)) = &validation.last {
                    let state = checkpoint::restore(&validation.parser_state)?;
                    if state.previous == Some(event.total) && eligible_source(tx, &validation)? {
                        tx.execute(
                            "INSERT INTO usage_sealed_sessions
                             (session, source, cutoff, anchor_position)
                             VALUES (?1, ?2, ?3, ?4)",
                            params![
                                validation.session,
                                validation.source,
                                validation.cutoff,
                                position
                            ],
                        )?;
                        cursor.work = Some(Work::Prune(Seal {
                            source: validation.source,
                            range: SealedRange {
                                session: validation.session,
                                cutoff: validation.cutoff,
                            },
                            anchor_position: *position,
                            review_reason: None,
                        }));
                        result.sealed = 1;
                        result.changed = true;
                    }
                }
            }
        }
        Some(Work::Prune(seal)) => {
            let positions: Vec<i64> = tx
                .prepare_cached(
                    "SELECT position FROM usage_observations
                     WHERE source = ?1 AND position < ?2 ORDER BY position LIMIT ?3",
                )?
                .query_map(
                    params![seal.source, seal.anchor_position, BATCH_ROWS as i64],
                    |row| row.get(0),
                )?
                .collect::<Result<_, _>>()?;
            for position in &positions {
                result.deleted += tx.execute(
                    "DELETE FROM usage_observations WHERE source = ?1 AND position = ?2",
                    params![seal.source, position],
                )?;
            }
            let remaining = BATCH_ROWS - result.deleted;
            if remaining > 0 {
                let events: Vec<(Vec<u8>, String)> = tx
                    .prepare_cached(
                        "SELECT id, day FROM usage_events
                         WHERE session = ?1 AND day < ?2 ORDER BY day LIMIT ?3",
                    )?
                    .query_map(
                        params![seal.range.session, seal.range.cutoff, remaining as i64],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )?
                    .collect::<Result<_, _>>()?;
                for (id, day) in events {
                    result.deleted += tx.execute("DELETE FROM usage_events WHERE id = ?1", [id])?;
                    result.changed |= tx.execute(
                        "INSERT OR IGNORE INTO usage_day_only(day, session) VALUES (?1, ?2)",
                        params![day, seal.range.session],
                    )? != 0;
                }
            }
            if result.deleted == BATCH_ROWS {
                cursor.work = Some(Work::Prune(seal));
            }
        }
    }
    result.next = Some(cursor);
    Ok(result)
}

/// 在修改来源或插入观察前检查；有 review 时由调用方回滚该批次并单独保存标记。
pub(super) fn guard_batch(
    tx: &Transaction<'_>,
    batch: &SourceBatch,
) -> anyhow::Result<Vec<Review>> {
    let mut sessions = BTreeSet::from([batch.scope_session.clone()]);
    sessions.extend(batch.observations.iter().map(|row| row.session.clone()));
    sessions.extend(batch.count_gap_sessions.iter().flatten().cloned());
    for (session, parent) in &batch.scope_parents {
        sessions.insert(session.clone());
        sessions.extend(parent.iter().cloned());
    }
    for (alias, session) in &batch.scope_aliases {
        sessions.insert(alias.clone());
        sessions.insert(session.clone());
    }
    let source_session: Option<String> = tx
        .query_row(
            "SELECT session FROM usage_sealed_sessions WHERE source = ?1",
            [&batch.source],
            |row| row.get(0),
        )
        .optional()?;
    sessions.extend(source_session);
    let mut reviews = Vec::new();
    for session in sessions {
        let Some(seal) = seal_for_session(tx, &session)? else {
            continue;
        };
        let reason = if let Some(reason) = seal.review_reason {
            Some(reason)
        } else if seal.source != batch.source {
            Some("sealed usage received another source".into())
        } else if batch.replace {
            Some("sealed usage source was replaced".into())
        } else {
            let identity: Vec<u8> = tx.query_row(
                "SELECT identity FROM usage_sources WHERE source = ?1",
                [&batch.source],
                |row| row.get(0),
            )?;
            if identity != batch.identity {
                Some("sealed usage source identity changed".into())
            } else if !batch.scope_valid
                || batch.scope_session != session
                || batch.count_gap_sessions.is_some()
                || batch
                    .scope_parents
                    .iter()
                    .any(|(owner, parent)| owner != &session || parent.is_some())
                || batch
                    .scope_aliases
                    .iter()
                    .any(|(alias, target)| alias != &session || target != &session)
            {
                Some("sealed usage received ambiguous session context".into())
            } else {
                let mut reason = None;
                for observation in &batch.observations {
                    let event = checkpoint::restore_event(
                        observation.event_id,
                        observation.session.clone(),
                        &observation.payload,
                    )?;
                    if event.session != session
                        || !eligible_event(&event)
                        || seal.range.contains(&event.session, &event.date.to_string())
                    {
                        reason =
                            Some("sealed usage received late or ambiguous observations".into());
                        break;
                    }
                }
                reason
            }
        };
        if let Some(reason) = reason {
            reviews.push(Review { session, reason });
        }
    }
    Ok(reviews)
}

pub(super) fn mark_review(tx: &Transaction<'_>, reviews: &[Review]) -> anyhow::Result<bool> {
    let mut changed = false;
    for review in reviews {
        changed |= tx.execute(
            "UPDATE usage_sealed_sessions SET review_reason = ?2
             WHERE session = ?1 AND review_reason IS NULL",
            params![review.session, review.reason],
        )? != 0;
    }
    Ok(changed)
}

pub(super) fn ranges(
    tx: &Transaction<'_>,
    sessions: &BTreeSet<String>,
) -> anyhow::Result<Vec<SealedRange>> {
    let mut ranges = Vec::new();
    for session in sessions {
        if let Some(seal) = seal_for_session(tx, session)? {
            ranges.push(seal.range);
        }
    }
    Ok(ranges)
}

/// 旧基线仅供计数归并，绝不再次写入冻结汇总。新增用量不能退回不完整的全量重算。
pub(super) fn filter_resolution(
    ranges: &[SealedRange],
    batch: &SourceBatch,
    resolution: &mut Resolution,
) -> Vec<Review> {
    let mut reviews = BTreeMap::new();
    let append = matches!(resolution, Resolution::Append(_));
    if !append {
        for range in ranges {
            if batch
                .observations
                .iter()
                .any(|row| row.session == range.session)
            {
                reviews.insert(
                    range.session.clone(),
                    "sealed usage requires full resolution".to_owned(),
                );
            }
        }
    }
    let events = match resolution {
        Resolution::Full(events) | Resolution::Append(events) => events,
    };
    events.retain(|event| {
        let frozen = ranges
            .iter()
            .any(|range| range.contains(&event.session, &event.day));
        if frozen && append {
            reviews.insert(
                event.session.clone(),
                "sealed usage attempted to append into a frozen day".to_owned(),
            );
        }
        !frozen
    });
    reviews
        .into_iter()
        .map(|(session, reason)| Review { session, reason })
        .collect()
}

#[cfg(test)]
mod tests;
