//! 回收旧的已提交事件行，保留来源观察与每日汇总供后续重新归并。
use anyhow::Context;
use chrono::{DateTime, Days, Local};
use rusqlite::{Row, Transaction, params};
use std::collections::BTreeSet;

const BATCH_ROWS: usize = 256;

#[derive(Clone)]
pub(super) struct Cursor {
    day: String,
    timestamp_ms: i64,
    session: String,
    rowid: i64,
}

pub(super) struct Batch {
    pub(super) next: Option<Cursor>,
    pub(super) deleted: usize,
    pub(super) precision_changed: bool,
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
    let read = |row: &Row<'_>| {
        Ok(Cursor {
            day: row.get(0)?,
            timestamp_ms: row.get(1)?,
            session: row.get(2)?,
            rowid: row.get(3)?,
        })
    };
    let mut candidates = Vec::with_capacity(BATCH_ROWS);
    if let Some(cursor) = cursor.filter(|cursor| cursor.day < cutoff) {
        let mut statement = tx.prepare_cached(
            "SELECT day, timestamp_ms, session, rowid FROM usage_events
             WHERE day = ?1 AND timestamp_ms = ?2 AND session = ?3 AND rowid > ?4
             ORDER BY rowid LIMIT ?5",
        )?;
        for candidate in statement.query_map(
            params![
                cursor.day,
                cursor.timestamp_ms,
                cursor.session,
                cursor.rowid,
                BATCH_ROWS as i64,
            ],
            read,
        )? {
            candidates.push(candidate?);
        }
    }
    if candidates.len() < BATCH_ROWS {
        let first = Cursor {
            day: String::new(),
            timestamp_ms: i64::MIN,
            session: String::new(),
            rowid: i64::MIN,
        };
        let cursor = cursor.unwrap_or(&first);
        let mut statement = tx.prepare_cached(
            "SELECT day, timestamp_ms, session, rowid FROM usage_events
             WHERE day < ?1 AND (day, timestamp_ms, session) > (?2, ?3, ?4)
             ORDER BY day, timestamp_ms, session, rowid LIMIT ?5",
        )?;
        for candidate in statement.query_map(
            params![
                cutoff,
                cursor.day,
                cursor.timestamp_ms,
                cursor.session,
                (BATCH_ROWS - candidates.len()) as i64,
            ],
            read,
        )? {
            candidates.push(candidate?);
        }
    }
    let mut days = BTreeSet::new();
    for candidate in &candidates {
        tx.execute(
            "DELETE FROM usage_events WHERE rowid = ?1",
            [candidate.rowid],
        )?;
        days.insert((&candidate.day, &candidate.session));
    }
    let mut precision_changed = false;
    for (day, session) in days {
        precision_changed |= tx.execute(
            "INSERT OR IGNORE INTO usage_day_only(day, session) VALUES (?1, ?2)",
            params![day, session],
        )? != 0;
    }
    Ok(Batch {
        next: candidates.last().cloned(),
        deleted: candidates.len(),
        precision_changed,
    })
}
