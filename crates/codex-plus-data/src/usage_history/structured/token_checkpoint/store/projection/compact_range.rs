use super::{
    ClassificationEvidence, Counter, DayTotals, EvidenceOverlay, GroupKey, Query, evidence_flags,
};
use crate::usage_history::structured::ContextConflicts;
use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::{Connection, params};
use serde::Deserialize;
use std::{borrow::Cow, collections::HashSet};

pub(super) enum Sessions<'a> {
    Omit,
    All,
    Members(&'a HashSet<String>),
}

const FIELDS: &str = "json_extract(data,
    '$.model', '$.provider', '$.role', '$.provider_source',
    '$.context_conflicts.model', '$.context_conflicts.provider',
    '$.context_conflicts.role', '$.context_conflicts.turn',
    '$.count_uncertain', '$.count_incomplete', '$.delta'
)";

#[derive(Deserialize)]
struct Row<'a> {
    #[serde(borrow)]
    model: Cow<'a, str>,
    #[serde(borrow)]
    provider: Cow<'a, str>,
    #[serde(borrow)]
    role: Cow<'a, str>,
    #[serde(borrow)]
    source: Option<Cow<'a, str>>,
    model_conflict: bool,
    provider_conflict: bool,
    role_conflict: bool,
    turn_conflict: bool,
    uncertain: bool,
    incomplete: bool,
    delta: Counter,
}

pub(in super::super) fn initialize(db: &Connection) -> Result<()> {
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'token_range_projection')",
        [],
        |row| row.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    // 在原账本事务内维护派生副本；按时间读取，不反复随机查询整条观察记录。
    db.execute_batch(&format!(
        "CREATE TABLE token_range_projection (
            timestamp_ms INTEGER NOT NULL,
            event_id BLOB NOT NULL,
            session_id INTEGER NOT NULL,
            day TEXT NOT NULL,
            payload TEXT NOT NULL,
            PRIMARY KEY(timestamp_ms,event_id)
         ) WITHOUT ROWID, STRICT;
         INSERT INTO token_range_projection
            SELECT timestamp_ms,event_id,session_id,day,{FIELDS}
            FROM (SELECT timestamp_ms,event_id,session_id,day,CAST(payload AS TEXT) AS data
                  FROM token_resolved NOT INDEXED)
            ORDER BY timestamp_ms,event_id;
         CREATE TRIGGER token_range_insert AFTER INSERT ON token_resolved BEGIN
            INSERT INTO token_range_projection
                SELECT NEW.timestamp_ms,NEW.event_id,NEW.session_id,NEW.day,{FIELDS}
                FROM (SELECT CAST(NEW.payload AS TEXT) AS data);
         END;
         CREATE TRIGGER token_range_update AFTER UPDATE ON token_resolved BEGIN
            UPDATE token_range_projection
                SET timestamp_ms=NEW.timestamp_ms, event_id=NEW.event_id, session_id=NEW.session_id,
                    day=NEW.day,payload=(SELECT {FIELDS} FROM (SELECT CAST(NEW.payload AS TEXT) AS data))
                WHERE timestamp_ms=OLD.timestamp_ms AND event_id=OLD.event_id;
         END;
         CREATE TRIGGER token_range_delete AFTER DELETE ON token_resolved BEGIN
            DELETE FROM token_range_projection
                WHERE timestamp_ms=OLD.timestamp_ms AND event_id=OLD.event_id;
         END;"
    ))?;
    Ok(())
}

pub(super) fn visit(
    db: &Connection,
    start: i64,
    end: i64,
    query: &Query,
    overlay: Option<&EvidenceOverlay>,
    trend_interval: Option<i64>,
    sessions: Sessions<'_>,
    mut accumulate: impl FnMut(&str, NaiveDate, &GroupKey, DayTotals, Option<usize>),
) -> Result<()> {
    let mut statement = db.prepare_cached(
        "SELECT r.event_id,
                CASE WHEN ?6 THEN (SELECT session FROM token_sessions WHERE id=r.session_id) ELSE '' END,
                r.day,r.payload,r.timestamp_ms
         FROM token_range_projection r
         WHERE r.timestamp_ms >= ?1 AND r.timestamp_ms < ?2
         AND r.day >= (SELECT precise_from FROM token_revision)
         AND (?3 IS NULL OR r.day >= ?3) AND (?4 IS NULL OR r.day <= ?4)
         AND (?5 IS NULL OR r.session_id = (SELECT id FROM token_sessions WHERE session=?5))
         AND (?7 IS NULL OR r.session_id IN (
             SELECT id FROM token_sessions WHERE session IN (SELECT value FROM json_each(?7))
         ))
         ORDER BY r.timestamp_ms,r.event_id",
    )?;
    let from = query.from.map(|day| day.to_string());
    let to = query.to.map(|day| day.to_string());
    let include_session = !matches!(sessions, Sessions::Omit) || query.session_id.is_some();
    let members = match sessions {
        Sessions::Members(members) => Some(serde_json::to_string(members)?),
        _ => None,
    };
    let mut rows = statement.query(params![
        start,
        end,
        from,
        to,
        query.session_id,
        include_session,
        members
    ])?;
    let mut last_date: Option<(String, NaiveDate)> = None;
    // 连续同组的记录只分配一次键，不改变日期、趋势桶或证据边界。
    let mut batch: Option<(String, NaiveDate, GroupKey, DayTotals, Option<usize>)> = None;
    while let Some(row) = rows.next()? {
        let session = row.get_ref(1)?.as_str()?;
        let date = row.get_ref(2)?.as_str()?;
        if last_date.as_ref().is_none_or(|(text, _)| text != date) {
            last_date = Some((date.to_owned(), date.parse()?));
        }
        let day = last_date.as_ref().unwrap().1;
        let mut value: Row<'_> = serde_json::from_str(row.get_ref(3)?.as_str()?)?;
        if let Some(overlay) = overlay
            && let Some(overlay) = overlay
                .classification(row.get_ref(0)?.as_blob()?.try_into()?)
        {
            value.model = Cow::Borrowed(&overlay.model);
            value.provider = Cow::Borrowed(&overlay.provider);
            value.source = overlay.provider_source.as_deref().map(Cow::Borrowed);
            value.provider_conflict = overlay.provider_conflict;
            value.uncertain = overlay.count_uncertain;
        }
        let model = Some(value.model.as_ref()).filter(|name| !name.is_empty());
        if !query.accepts_fields(session, "", model, Some(day)) {
            continue;
        }
        let conflicts = ContextConflicts {
            model: value.model_conflict,
            provider: value.provider_conflict,
            role: value.role_conflict,
            turn: value.turn_conflict,
        };
        let evidence =
            ClassificationEvidence::from_source(value.source.as_deref(), conflicts.any());
        let totals =
            DayTotals::from_count(value.delta, value.uncertain || value.incomplete, evidence);
        let flags = super::conflict_flags(conflicts) | evidence_flags(evidence);
        let bucket = trend_interval
            .map(|interval| {
                row.get::<_, i64>(4)
                    .map(|at| ((at - start) / interval) as usize)
            })
            .transpose()?;
        if let Some((owner, date, key, group, previous_bucket)) = &mut batch
            && *previous_bucket == bucket
            && (
                *date,
                owner.as_str(),
                key.0.as_str(),
                key.1.as_str(),
                key.2.as_str(),
                key.3,
            ) == (
                day,
                session,
                value.model.as_ref(),
                value.provider.as_ref(),
                value.role.as_ref(),
                flags,
            )
        {
            group.merge(totals);
            continue;
        }
        if let Some((owner, date, key, group, bucket)) = batch.replace((
            session.to_owned(),
            day,
            (
                value.model.into_owned(),
                value.provider.into_owned(),
                value.role.into_owned(),
                flags,
            ),
            totals,
            bucket,
        )) {
            accumulate(&owner, date, &key, group, bucket);
        }
    }
    if let Some((owner, day, key, group, bucket)) = batch {
        accumulate(&owner, day, &key, group, bucket);
    }
    Ok(())
}
