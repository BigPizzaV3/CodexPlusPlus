//! 工具执行的时间索引；输入和输出正文始终留在原会话文件中。
use anyhow::{Context, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::{HashMap, HashSet};

pub(super) const DETAIL_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const PURGE_BATCH_ROWS: usize = 256;
const COMPACT_BATCH_ROWS: usize = 256;
const COMPACT_SCAN_ROWS: usize = 1024;
const COMPACT_LOOKUP_ROWS: usize = 1024;
const MAX_OUTPUT_SEGMENTS: usize = 64;
const MAX_RECEIPT_PREFIX_BYTES: usize = 4096;

#[path = "execution_metadata.rs"]
mod metadata;
pub(super) use metadata::{MetadataCursor, MetadataRepair, repair_metadata};

fn retention_start(now_ms: i64) -> anyhow::Result<i64> {
    let before_day = DateTime::from_timestamp_millis(now_ms)
        .context("compaction time is out of range")?
        .with_timezone(&Local)
        .date_naive()
        .checked_sub_days(chrono::Days::new(59))
        .context("compaction day is out of range")?;
    Ok(Local
        .from_local_datetime(&before_day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .context("compaction day has no local midnight")?
        .timestamp_millis())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ObservationKind {
    Call {
        name: String,
        namespace: String,
        kind: String,
        process_id: Option<String>,
    },
    Output {
        process_id: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ExecutionObservation {
    pub(super) session: String,
    pub(super) id: String,
    pub(super) timestamp_ms: i64,
    pub(super) start: u64,
    pub(super) end: u64,
    pub(super) kind: ObservationKind,
}

/// 解析当前扫描器已读取的完整行。继承的会话副本不作为新的工具调用。
pub(super) fn parse_line(
    line: &[u8],
    start: u64,
    end: u64,
    session: &str,
    inherited: bool,
) -> anyhow::Result<Option<ExecutionObservation>> {
    if inherited || session.is_empty() {
        return Ok(None);
    }
    #[derive(Deserialize)]
    struct Envelope<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
        timestamp: Option<&'a str>,
        #[serde(borrow)]
        payload: &'a RawValue,
    }
    #[derive(Deserialize)]
    struct Item<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
        call_id: Option<&'a str>,
        id: Option<&'a str>,
        name: Option<&'a str>,
        namespace: Option<&'a str>,
        #[serde(borrow)]
        arguments: Option<&'a RawValue>,
        #[serde(borrow)]
        input: Option<&'a RawValue>,
        #[serde(borrow)]
        output: Option<&'a RawValue>,
    }
    #[derive(Deserialize)]
    struct ItemKind<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
    }
    let envelope: Envelope<'_> = serde_json::from_slice(line)?;
    if envelope.kind != "response_item" {
        return Ok(None);
    }
    let item_kind: ItemKind<'_> = serde_json::from_str(envelope.payload.get())?;
    if !matches!(
        item_kind.kind,
        "function_call"
            | "custom_tool_call"
            | "local_shell_call"
            | "function_call_output"
            | "custom_tool_call_output"
            | "local_shell_call_output"
    ) {
        return Ok(None);
    }
    let item: Item<'_> = serde_json::from_str(envelope.payload.get())?;
    let id = item
        .call_id
        .or(item.id)
        .filter(|id| !id.is_empty() && id.len() <= 300)
        .context("tool event has no valid call identity")?;
    let timestamp = envelope.timestamp.context("tool event has no timestamp")?;
    let timestamp_ms = DateTime::parse_from_rfc3339(timestamp)
        .context("tool event timestamp is invalid")?
        .timestamp_millis();
    let kind = if matches!(item.kind, "function_call" | "custom_tool_call" | "local_shell_call") {
        let name = item.name.unwrap_or(if item.kind == "local_shell_call" { "local_shell" } else { "" });
        let name = &name[..name.char_indices().nth(300).map_or(name.len(), |(i, _)| i)];
        let namespace = item.namespace.unwrap_or("");
        let namespace =
            &namespace[..namespace.char_indices().nth(300).map_or(namespace.len(), |(i, _)| i)];
        let arguments = item.arguments.or(item.input).filter(|raw| {
            raw.get().len() <= super::tool_receipts::MAX_JSON_TEXT_BYTES
        });
        let decoded = arguments
            .filter(|raw| raw.get().starts_with('"'))
            .map(|raw| serde_json::from_str::<String>(raw.get()))
            .transpose()?;
        let args = decoded
            .as_deref()
            .or_else(|| arguments.map(|raw| raw.get()))
            .unwrap_or("");
        let receipt = super::tool_receipts::call(&serde_json::json!({
            "name":name, "namespace":namespace, "arguments":args
        }));
        let skill = receipt["arguments"]["skillNames"]
            .as_array()
            .is_some_and(|names| !names.is_empty());
        let is_poll = name.rsplit('.').next() == Some("write_stdin");
        let process_id = if is_poll {
            #[derive(Deserialize)]
            struct PollArguments {
                session_id: u64,
            }
            let args: PollArguments =
                serde_json::from_str(args).context("invalid write_stdin arguments")?;
            Some(args.session_id.to_string())
        } else {
            None
        };
        ObservationKind::Call {
            name: if skill {
                receipt["arguments"]["skillNames"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                name.to_owned()
            },
            namespace: namespace.to_owned(),
            kind: if skill {
                "skill"
            } else if name.starts_with("mcp__") || namespace.starts_with("mcp__") {
                "mcp"
            } else {
                "tool"
            }
            .to_owned(),
            process_id,
        }
    } else {
        let mut process_id = None;
        if let Some(raw) = item.output.filter(|raw| raw.get().starts_with('"')) {
            let encoded = raw.get();
            let bytes = encoded.as_bytes();
            // 截断时只解码完整换行以前的前缀，不能把被截断的进程号当作完整回执。
            let end = if bytes.len() <= MAX_RECEIPT_PREFIX_BYTES {
                Some(bytes.len() - 1)
            } else {
                let mut index = 1;
                let mut last_newline = None;
                while index < MAX_RECEIPT_PREFIX_BYTES {
                    if bytes[index] != b'\\' {
                        index += 1;
                        continue;
                    }
                    if index + 1 >= MAX_RECEIPT_PREFIX_BYTES {
                        break;
                    }
                    match bytes[index + 1] {
                        b'n' | b'r' => {
                            last_newline = Some(index + 2);
                            index += 2;
                        }
                        b'u' if index + 6 <= MAX_RECEIPT_PREFIX_BYTES => {
                            if bytes[index + 2..index + 6].eq_ignore_ascii_case(b"000a")
                                || bytes[index + 2..index + 6].eq_ignore_ascii_case(b"000d")
                            {
                                last_newline = Some(index + 6);
                            }
                            index += 6;
                        }
                        _ => index += 2,
                    }
                }
                last_newline
            };
            if let Some(end) = end {
                let mut prefix = String::with_capacity(end + 1);
                prefix.push_str(&encoded[..end]);
                prefix.push('"');
                let text: String = serde_json::from_str(&prefix)
                    .context("invalid tool output receipt prefix")?;
                process_id = super::exec_receipt(&serde_json::Value::String(text))
                    .and_then(|(running, id)| running.then_some(id))
                    .filter(|id| id.len() <= 20);
            }
        }
        ObservationKind::Output { process_id }
    };
    ensure!(start < end, "tool event has an empty location");
    Ok(Some(ExecutionObservation {
        session: session.to_owned(),
        id: id.to_owned(),
        timestamp_ms,
        start,
        end,
        kind,
    }))
}

pub(super) fn initialize(tx: &Transaction<'_>) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS execution_observations (
            source BLOB NOT NULL REFERENCES usage_sources(source),
            start INTEGER NOT NULL,
            end INTEGER,
            session TEXT NOT NULL,
            call_id TEXT NOT NULL,
            timestamp_ms INTEGER NOT NULL,
            is_output INTEGER NOT NULL,
            name TEXT,
            namespace TEXT,
            call_kind TEXT,
            process_id TEXT,
            PRIMARY KEY(source, start)
        ) WITHOUT ROWID;
        CREATE INDEX IF NOT EXISTS executions_by_time
            ON execution_observations(is_output, timestamp_ms, session, call_id);
        CREATE INDEX IF NOT EXISTS executions_by_call
            ON execution_observations(session, call_id, is_output, source, start);
        CREATE INDEX IF NOT EXISTS executions_detail_expiry
            ON execution_observations(timestamp_ms, session, call_id)
            WHERE is_output = 0 AND end IS NOT NULL;
        CREATE TABLE IF NOT EXISTS execution_daily (
            day TEXT NOT NULL,
            session TEXT NOT NULL,
            count INTEGER NOT NULL,
            partial INTEGER NOT NULL,
            skill INTEGER NOT NULL DEFAULT 0,
            mcp INTEGER NOT NULL DEFAULT 0,
            tool INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(day, session)
        ) WITHOUT ROWID;
        CREATE INDEX IF NOT EXISTS execution_daily_session ON execution_daily(session);
        CREATE TABLE IF NOT EXISTS execution_compacted_sources (
            source BLOB NOT NULL REFERENCES usage_sources(source),
            day TEXT NOT NULL,
            session TEXT NOT NULL,
            PRIMARY KEY(source, day, session)
        ) WITHOUT ROWID;",
    )?;
    Ok(())
}

/// 与来源游标和用量一起提交；未提交的批次不能改变索引。
pub(super) fn apply(
    tx: &Transaction<'_>,
    source: &[u8],
    replace: bool,
    observations: &[ExecutionObservation],
    now_ms: i64,
) -> anyhow::Result<()> {
    if replace {
        tx.execute(
            "UPDATE execution_daily SET partial = 1 WHERE (day, session) IN (
                SELECT day, session FROM execution_compacted_sources WHERE source = ?1
            )",
            [source],
        )?;
        tx.execute(
            "DELETE FROM execution_observations WHERE source = ?1",
            [source],
        )?;
    }
    let cutoff = now_ms.saturating_sub(DETAIL_RETENTION_MS);
    let retained_from = retention_start(now_ms)?;
    let mut sealed_sessions = HashMap::new();
    let mut marked_sessions = HashSet::new();
    let mut insert = tx.prepare_cached(
        "INSERT INTO execution_observations(source, start, end, session, call_id, timestamp_ms,
            is_output, name, namespace, call_kind, process_id)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for item in observations {
        ensure!(
            !item.session.is_empty() && !item.id.is_empty(),
            "tool identity is empty"
        );
        ensure!(item.start < item.end, "tool event has an empty location");
        let start = i64::try_from(item.start).context("tool start exceeds SQLite integer")?;
        let end = i64::try_from(item.end).context("tool end exceeds SQLite integer")?;
        let recent = item.timestamp_ms > cutoff;
        if !recent
            && matches!(&item.kind, ObservationKind::Call { name, .. }
            if name.rsplit('.').next() == Some("write_stdin"))
        {
            continue;
        }
        if !recent && matches!(&item.kind, ObservationKind::Call { .. }) {
            let day = DateTime::from_timestamp_millis(item.timestamp_ms)
                .context("tool event timestamp is out of range")?
                .with_timezone(&Local)
                .date_naive()
                .to_string();
            let compacted = if item.timestamp_ms < retained_from {
                if let Some(compacted) = sealed_sessions.get(&item.session) {
                    *compacted
                } else {
                    let compacted = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM execution_daily WHERE session = ?1)",
                        [&item.session],
                        |row| row.get::<_, bool>(0),
                    )?;
                    sealed_sessions.insert(item.session.clone(), compacted);
                    compacted
                }
            } else {
                tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM execution_daily WHERE day = ?1 AND session = ?2)",
                    params![day, item.session],
                    |row| row.get(0),
                )?
            };
            if compacted {
                if marked_sessions.insert(item.session.clone()) {
                    tx.execute(
                        "UPDATE execution_daily SET partial = 1 WHERE session = ?1",
                        [&item.session],
                    )?;
                }
                tx.execute(
                    "INSERT INTO execution_daily(day, session, count, partial) VALUES(?1, ?2, 0, 1)
                     ON CONFLICT(day, session) DO UPDATE SET partial = 1",
                    params![day, item.session],
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO execution_compacted_sources(source, day, session)
                     VALUES(?1, ?2, ?3)",
                    params![source, day, item.session],
                )?;
                continue;
            }
        }
        let (is_output, name, namespace, kind, process_id) = match &item.kind {
            ObservationKind::Call {
                name,
                namespace,
                kind,
                process_id,
            } => (
                false,
                Some(name.as_str()),
                Some(namespace.as_str()),
                Some(kind.as_str()),
                recent.then_some(process_id.as_deref()).flatten(),
            ),
            ObservationKind::Output { process_id } if recent => {
                let expired_call: bool = tx.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM execution_observations
                        WHERE source = ?1 AND session = ?2 AND call_id = ?3
                            AND is_output = 0 AND timestamp_ms <= ?4
                    ) AND NOT EXISTS(
                        SELECT 1 FROM execution_observations
                        WHERE source = ?1 AND session = ?2 AND call_id = ?3
                            AND is_output = 0 AND timestamp_ms > ?4
                    )",
                    params![source, item.session, item.id, cutoff],
                    |row| row.get(0),
                )?;
                if expired_call {
                    continue;
                }
                (true, None, None, None, process_id.as_deref())
            }
            ObservationKind::Output { .. } => continue,
        };
        let pending_outputs = if !recent && !is_output {
            tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM execution_observations
                    WHERE source = ?1 AND session = ?2 AND call_id = ?3 AND is_output = 1
                )",
                params![source, item.session, item.id],
                |row| row.get::<_, bool>(0),
            )?
        } else {
            false
        };
        insert.execute(params![
            source,
            start,
            if recent {
                Some(end)
            } else {
                // 旧调用只保留清理标记，不保留可读取详情的结束位置。
                pending_outputs.then_some(0)
            },
            item.session,
            item.id,
            item.timestamp_ms,
            is_output,
            name,
            namespace,
            kind,
            process_id,
        ])?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct CompactionCursor {
    timestamp_ms: i64,
    session: String,
    id: String,
}

pub(super) struct CompactionBatch {
    pub(super) deleted: usize,
    pub(super) next: Option<CompactionCursor>,
    pub(super) reduced: bool,
    pub(super) summarized: bool,
}

/// 按身份压缩旧调用；容量不足时允许将未完成来源记为部分汇总。
pub(super) fn compact_batch(
    tx: &Transaction<'_>,
    now_ms: i64,
    cursor: Option<&CompactionCursor>,
    allow_incomplete: bool,
) -> anyhow::Result<CompactionBatch> {
    let before = retention_start(now_ms)?;
    let candidates = {
        let mut statement = tx.prepare(
            "SELECT e.timestamp_ms, e.session, e.call_id,
                    e.end IS NULL AND (s.complete = 1 OR ?6)
                        AND s.execution_indexed = 1 AND s.scope_valid = 1
             FROM execution_observations e
             JOIN usage_sources s ON s.source = e.source
             WHERE e.is_output = 0 AND e.timestamp_ms < ?1
               AND (e.timestamp_ms, e.session, e.call_id) > (?2, ?3, ?4)
             ORDER BY e.timestamp_ms, e.session, e.call_id LIMIT ?5",
        )?;
        statement
            .query_map(
                params![
                    before,
                    cursor.map_or(i64::MIN, |cursor| cursor.timestamp_ms),
                    cursor.map_or("", |cursor| cursor.session.as_str()),
                    cursor.map_or("", |cursor| cursor.id.as_str()),
                    COMPACT_SCAN_ROWS as i64,
                    allow_incomplete,
                ],
                |row| {
                    Ok((
                        CompactionCursor {
                            timestamp_ms: row.get(0)?,
                            session: row.get(1)?,
                            id: row.get(2)?,
                        },
                        row.get::<_, bool>(3)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut deleted = 0;
    let mut inspected = 0;
    let mut processed = 0;
    let mut next = cursor.cloned();
    let mut seen = HashSet::new();
    let mut reduced = false;
    let mut summarized = false;
    for (candidate, ready) in &candidates {
        let session = &candidate.session;
        let id = &candidate.id;
        if !seen.insert((session, id)) || !ready {
            next = Some(candidate.clone());
            processed += 1;
            continue;
        }
        if inspected + COMPACT_BATCH_ROWS + 1 > COMPACT_LOOKUP_ROWS {
            break;
        }
        let observations = {
            let mut statement = tx.prepare_cached(
                "SELECT e.source, e.start, e.timestamp_ms,
                        e.timestamp_ms < ?3 AND e.end IS NULL AND (s.complete = 1 OR ?5)
                            AND s.execution_indexed = 1 AND s.scope_valid = 1,
                        e.name, e.namespace, e.call_kind, s.execution_valid AND s.complete
                 FROM execution_observations e
                 JOIN usage_sources s ON s.source = e.source
                 WHERE e.session = ?1 AND e.call_id = ?2 AND e.is_output = 0 LIMIT ?4",
            )?;
            statement
                .query_map(
                    params![session, id, before, (COMPACT_BATCH_ROWS + 1) as i64, allow_incomplete],
                    |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, bool>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, bool>(7)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?
        };
        inspected += observations.len();
        if observations.len() > COMPACT_BATCH_ROWS {
            let mut representatives = HashSet::new();
            for (source, start, at, ready, name, namespace, kind, _) in observations {
                if ready && !representatives.insert((source.clone(), at, name, namespace, kind)) {
                    deleted += tx.execute(
                        "DELETE FROM execution_observations WHERE source = ?1 AND start = ?2",
                        params![source, start],
                    )?;
                    reduced = true;
                }
                if deleted == COMPACT_BATCH_ROWS {
                    break;
                }
            }
            next = Some(candidate.clone());
            processed += 1;
            if deleted == COMPACT_BATCH_ROWS {
                break;
            }
            continue;
        }
        if observations.iter().any(|item| !item.3) {
            next = Some(candidate.clone());
            processed += 1;
            continue;
        }
        if deleted + observations.len() > COMPACT_BATCH_ROWS {
            break;
        }
        let first_ms = observations.iter().map(|item| item.2).min().unwrap();
        let last_ms = observations.iter().map(|item| item.2).max().unwrap();
        // 输出缺口不阻止已确认调用到期压缩，但必须保留覆盖不完整的标记。
        let partial = first_ms != last_ms || observations.iter().any(|item| !item.7);
        let first = &observations[0];
        let classification = (first_ms == last_ms
            && observations.iter().all(|item| (item.4.as_ref(), item.5.as_ref(), item.6.as_ref())
                == (first.4.as_ref(), first.5.as_ref(), first.6.as_ref())))
            .then_some(first.6.as_deref())
            .flatten();
        let day = DateTime::from_timestamp_millis(first_ms)
            .context("tool event timestamp is out of range")?
            .with_timezone(&Local)
            .date_naive()
            .to_string();
        // 先释放页，再写汇总；失败时整个事务回滚。
        deleted += tx.execute(
            "DELETE FROM execution_observations
             WHERE session = ?1 AND call_id = ?2 AND is_output = 0",
            params![session, id],
        )?;
        summarized = true;
        tx.execute(
            "INSERT INTO execution_daily(day, session, count, partial, skill, mcp, tool)
             VALUES(?1, ?2, 1, ?3, ?4, ?5, ?6)
             ON CONFLICT(day, session) DO UPDATE SET
                 count = execution_daily.count + 1,
                 partial = MAX(execution_daily.partial, excluded.partial),
                 skill = execution_daily.skill + excluded.skill,
                 mcp = execution_daily.mcp + excluded.mcp,
                 tool = execution_daily.tool + excluded.tool",
            params![day, session, partial, classification == Some("skill"),
                classification == Some("mcp"), classification == Some("tool")],
        )?;
        for (source, _, at, _, _, _, _, _) in observations {
            tx.execute(
                "INSERT OR IGNORE INTO execution_compacted_sources(source, day, session)
                 VALUES(?1, ?2, ?3)",
                params![source, day, session],
            )?;
            let observed_day = DateTime::from_timestamp_millis(at)
                .context("tool event timestamp is out of range")?
                .with_timezone(&Local)
                .date_naive()
                .to_string();
            if observed_day != day {
                tx.execute(
                    "INSERT INTO execution_daily(day, session, count, partial) VALUES(?1, ?2, 0, 1)
                     ON CONFLICT(day, session) DO UPDATE SET partial = 1",
                    params![observed_day, session],
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO execution_compacted_sources(source, day, session)
                     VALUES(?1, ?2, ?3)",
                    params![source, observed_day, session],
                )?;
            }
        }
        next = Some(candidate.clone());
        processed += 1;
    }
    if processed == candidates.len() && candidates.len() < COMPACT_SCAN_ROWS {
        next = None;
    }
    Ok(CompactionBatch {
        deleted,
        next,
        reduced,
        summarized,
    })
}

#[cfg(test)]
fn compact_completed(tx: &Transaction<'_>, now_ms: i64) -> anyhow::Result<usize> {
    Ok(compact_batch(tx, now_ms, None, false)?.deleted)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DailyExecutionSummary {
    pub(super) day: String,
    pub(super) count: u64,
    pub(super) partial: bool,
    pub(super) skill: u64,
    pub(super) mcp: u64,
    pub(super) tool: u64,
    pub(super) classification_complete: bool,
}

/// 返回本地日粒度；调用方需对未覆盖整日的查询明确提示精度。
pub(super) fn daily_summary(
    db: &Connection,
    start_day: NaiveDate,
    end_day: NaiveDate,
    sessions: Option<&[String]>,
) -> anyhow::Result<Vec<DailyExecutionSummary>> {
    ensure!(start_day < end_day, "invalid daily execution range");
    if sessions.is_some_and(|sessions| sessions.is_empty()) {
        return Ok(Vec::new());
    }
    let mut arguments = vec![
        rusqlite::types::Value::Text(start_day.to_string()),
        rusqlite::types::Value::Text(end_day.to_string()),
    ];
    let filter = if let Some(sessions) = sessions {
        let first = arguments.len() + 1;
        arguments.extend(sessions.iter().cloned().map(rusqlite::types::Value::Text));
        let parameters = (first..first + sessions.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(" AND session IN ({parameters})")
    } else {
        String::new()
    };
    let mut statement = db.prepare(&format!(
        "SELECT day, SUM(count), MAX(partial), SUM(skill), SUM(mcp), SUM(tool) FROM execution_daily
         WHERE day >= ?1 AND day < ?2{filter} GROUP BY day ORDER BY day"
    ))?;
    let rows = statement.query_map(params_from_iter(arguments.iter()), |row| {
        Ok(DailyExecutionSummary {
            day: row.get(0)?,
            count: row.get(1)?,
            partial: row.get(2)?,
            skill: row.get(3)?,
            mcp: row.get(4)?,
            tool: row.get(5)?,
            classification_complete: row.get::<_, u64>(1)?
                == row.get::<_, u64>(3)? + row.get::<_, u64>(4)? + row.get::<_, u64>(5)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 清理已过期正文定位，保留名称、类别和逐次时间用于统计。
pub(super) fn purge_details(tx: &Transaction<'_>, now_ms: i64) -> anyhow::Result<bool> {
    let cutoff = now_ms.saturating_sub(DETAIL_RETENTION_MS);
    let mut calls_changed = false;
    let mut changed = tx.execute(
        "DELETE FROM execution_observations WHERE (source, start) IN (
             SELECT source, start FROM execution_observations
             WHERE is_output = 1 AND timestamp_ms <= ?1 LIMIT ?2
         )",
        params![cutoff, PURGE_BATCH_ROWS as i64],
    )?;
    while changed < PURGE_BATCH_ROWS {
        let call: Option<(Vec<u8>, i64, String, String, Option<String>)> = tx
            .query_row(
                "SELECT source, start, session, call_id, name FROM execution_observations
                 WHERE is_output = 0 AND end IS NOT NULL AND timestamp_ms <= ?1
                 ORDER BY timestamp_ms, session, call_id LIMIT 1",
                [cutoff],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((source, start, session, id, name)) = call else {
            break;
        };
        changed += tx.execute(
            "DELETE FROM execution_observations WHERE (source, start) IN (
                 SELECT source, start FROM execution_observations
                 WHERE source = ?1 AND session = ?2 AND call_id = ?3 AND is_output = 1
                     AND NOT EXISTS (
                         SELECT 1 FROM execution_observations
                         WHERE source = ?1 AND session = ?2 AND call_id = ?3
                             AND is_output = 0 AND timestamp_ms > ?4
                     )
                 LIMIT ?5
             )",
            params![source, session, id, cutoff, (PURGE_BATCH_ROWS - changed) as i64],
        )?;
        if changed == PURGE_BATCH_ROWS {
            break;
        }
        if name
            .as_deref()
            .is_some_and(|name| name.rsplit('.').next() == Some("write_stdin"))
        {
            changed += tx.execute(
                "DELETE FROM execution_observations WHERE source = ?1 AND start = ?2",
                params![source, start],
            )?;
        } else {
            changed += tx.execute(
                "UPDATE execution_observations SET end = NULL, process_id = NULL
                 WHERE source = ?1 AND start = ?2",
                params![source, start],
            )?;
        }
        calls_changed = true;
    }
    Ok(calls_changed)
}

#[derive(Clone, Debug)]
pub(super) struct DetailReclaimCursor {
    timestamp_ms: i64,
    session: String,
    id: String,
    source: Vec<u8>,
    start: i64,
}

pub(super) struct DetailReclaimBatch {
    pub(super) next: Option<DetailReclaimCursor>,
    pub(super) changed: bool,
}

/// 空间不足时只回收可在现有读取预算内恢复的普通工具详情定位。
pub(super) fn reclaim_oldest_details(
    tx: &Transaction<'_>,
    now_ms: i64,
    cursor: Option<&DetailReclaimCursor>,
) -> anyhow::Result<DetailReclaimBatch> {
    let cutoff = now_ms.saturating_sub(DETAIL_RETENTION_MS);
    let candidates = {
        let mut statement = tx.prepare(
            "SELECT timestamp_ms, session, call_id, source, start FROM execution_observations
             WHERE is_output = 0 AND timestamp_ms <= ?1
               AND (timestamp_ms, session, call_id, source, start) > (?2, ?3, ?4, ?5, ?6)
             ORDER BY timestamp_ms, session, call_id, source, start LIMIT ?7",
        )?;
        let rows = statement.query_map(
            params![
                cutoff,
                cursor.map_or(i64::MIN, |value| value.timestamp_ms),
                cursor.map_or("", |value| value.session.as_str()),
                cursor.map_or("", |value| value.id.as_str()),
                cursor.map_or(&[][..], |value| value.source.as_slice()),
                cursor.map_or(i64::MIN, |value| value.start),
                PURGE_BATCH_ROWS as i64,
            ],
            |row| Ok(DetailReclaimCursor {
                timestamp_ms: row.get(0)?,
                session: row.get(1)?,
                id: row.get(2)?,
                source: row.get(3)?,
                start: row.get(4)?,
            }),
        )?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut removed = 0;
    let mut processed = 0;
    let mut next = cursor.cloned();
    for candidate in &candidates {
        let calls: Vec<(Vec<u8>, i64, Option<i64>, Option<String>, Option<String>)> = {
            let mut statement = tx.prepare(
                "SELECT source, start, end, name, call_kind FROM execution_observations
                 WHERE session = ?1 AND call_id = ?2 AND is_output = 0 LIMIT 2",
            )?;
            let rows = statement.query_map(params![candidate.session, candidate.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )?;
            rows.collect::<Result<_, _>>()?
        };
        let [(source, start, Some(_), Some(name), Some(kind))] = calls.as_slice() else {
            next = Some(candidate.clone());
            processed += 1;
            continue;
        };
        if kind != "tool" || name == "write_stdin" || name.ends_with(".write_stdin") {
            next = Some(candidate.clone());
            processed += 1;
            continue;
        }
        let (count, last_end, invalid): (i64, Option<i64>, bool) = tx.query_row(
            "SELECT COUNT(*), MAX(end), COALESCE(MAX(
                end IS NULL OR end <= start OR start <= ?4 OR end - ?4 > ?5
                OR timestamp_ms > ?6 OR process_id IS NOT NULL), 0)
             FROM (SELECT start, end, timestamp_ms, process_id
                   FROM execution_observations INDEXED BY executions_by_call
                   WHERE session = ?1 AND call_id = ?2 AND source = ?3 AND is_output = 1
                   LIMIT ?7)",
            params![candidate.session, candidate.id, source, start,
                super::execution_detail::MAX_RECOVERY_ROWS as i64, cutoff,
                super::execution_detail::MAX_RECOVERY_OUTPUTS as i64 + 1],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let compressed_over_budget = last_end.is_some_and(|end| {
            end > super::execution_detail::MAX_DECOMPRESSED_BYTES as i64
        }) && super::persistent_index::decode_source(&source)?
            .extension()
            .is_some_and(|extension| extension == "zst");
        if invalid || count == 0 || count > super::execution_detail::MAX_RECOVERY_OUTPUTS as i64
            || compressed_over_budget
        {
            next = Some(candidate.clone());
            processed += 1;
            continue;
        }
        if removed + count as usize > PURGE_BATCH_ROWS {
            break;
        }
        tx.execute(
            "UPDATE execution_observations SET end = NULL, process_id = NULL
             WHERE source = ?1 AND start = ?2",
            params![source, start],
        )?;
        removed += tx.execute(
            "DELETE FROM execution_observations
             WHERE source = ?1 AND session = ?2 AND call_id = ?3 AND is_output = 1",
            params![source, candidate.session, candidate.id],
        )?;
        next = Some(candidate.clone());
        processed += 1;
    }
    Ok(DetailReclaimBatch {
        next: (processed < candidates.len() || candidates.len() == PURGE_BATCH_ROWS)
            .then_some(next)
            .flatten(),
        changed: removed != 0,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Cursor {
    timestamp_ms: i64,
    session: String,
    id: String,
}

pub(super) fn decode_cursor(value: &str) -> anyhow::Result<Cursor> {
    ensure!(value.len() <= 4096, "execution cursor is too long");
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .context("invalid execution cursor")?;
    let cursor: Cursor = serde_json::from_slice(&decoded).context("invalid execution cursor")?;
    ensure!(
        !cursor.session.is_empty() && !cursor.id.is_empty(),
        "invalid execution cursor"
    );
    Ok(cursor)
}

fn encode_cursor(cursor: &Cursor) -> anyhow::Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor)?))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExecutionItem {
    pub(super) session_id: String,
    pub(super) id: String,
    pub(super) timestamp_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) kind: Option<String>,
}

pub(super) struct ExecutionQuery<'a> {
    pub(super) start_ms: i64,
    pub(super) end_ms: i64,
    pub(super) sessions: Option<&'a [String]>,
    pub(super) limit: usize,
    pub(super) cursor: Option<&'a str>,
    pub(super) revision: Option<i64>,
    pub(super) search: &'a str,
    pub(super) kind: Option<&'a str>,
    pub(super) now_ms: i64,
}

pub(super) struct ExecutionPage {
    pub(super) revision: i64,
    pub(super) items: Vec<ExecutionItem>,
    pub(super) next_cursor: Option<String>,
    pub(super) total: u64,
    pub(super) matched: u64,
    pub(super) conflicts: u64,
    pub(super) recent_skill: u64,
    pub(super) recent_mcp: u64,
    pub(super) recent_tool: u64,
    pub(super) skill: u64,
    pub(super) mcp: u64,
    pub(super) tool: u64,
    pub(super) classified: u64,
    pub(super) search_applied: bool,
}

pub(super) enum PageLookup {
    Ready(ExecutionPage),
    Stale,
}

/// 只返回有限名称组和固定 64 桶，调用方不必下载全部时间记录做聚合。
pub(super) fn activity_summary(
    db: &Connection,
    request: &ExecutionQuery<'_>,
) -> anyhow::Result<serde_json::Value> {
    let span = request.end_ms.checked_sub(request.start_ms)
        .context("execution activity range is too large")?;
    let bucket_width = span / 64 + i64::from(span % 64 != 0);
    let mut args = vec![
        rusqlite::types::Value::Integer(request.start_ms),
        rusqlite::types::Value::Integer(request.end_ms),
        rusqlite::types::Value::Integer(request.now_ms),
        rusqlite::types::Value::Text(request.kind.unwrap_or("").to_owned()),
        rusqlite::types::Value::Integer(bucket_width),
        rusqlite::types::Value::Integer(request.now_ms.saturating_sub(DETAIL_RETENTION_MS)),
    ];
    let filter = if let Some(sessions) = request.sessions {
        let first = args.len() + 1;
        args.extend(sessions.iter().cloned().map(rusqlite::types::Value::Text));
        let slots = (first..first + sessions.len()).map(|i| format!("?{i}"))
            .collect::<Vec<_>>().join(",");
        format!(" AND e.session IN ({slots})")
    } else {
        String::new()
    };
    let mut statement = db.prepare(&format!(
        "WITH candidates AS (
            SELECT DISTINCT e.session, e.call_id FROM execution_observations e
            WHERE e.is_output = 0 AND e.timestamp_ms >= ?1 AND e.timestamp_ms < ?2{filter}
        ), calls AS (
            SELECT e.session, e.call_id, MIN(e.timestamp_ms) AS at,
                MIN(e.name) AS name, MIN(e.namespace) AS namespace, MIN(e.call_kind) AS kind
            FROM candidates c JOIN execution_observations e
                ON e.session = c.session AND e.call_id = c.call_id AND e.is_output = 0
            GROUP BY e.session, e.call_id
            HAVING MIN(e.timestamp_ms) = MAX(e.timestamp_ms)
                AND MIN(e.timestamp_ms) >= ?1 AND MAX(e.timestamp_ms) < ?2
                AND MAX(e.timestamp_ms) <= ?3 AND COUNT(e.name) = COUNT(*)
                AND MIN(e.name) = MAX(e.name) AND MIN(e.namespace) = MAX(e.namespace)
                AND MIN(e.call_kind) = MAX(e.call_kind)
                AND MIN(e.call_kind) IN ('skill', 'mcp', 'tool')
                AND (?4 = '' OR MIN(e.call_kind) = ?4)
                AND MIN(e.name) != 'write_stdin' AND MIN(e.name) NOT LIKE '%.write_stdin'
        ), visible_calls AS (
            SELECT at, kind,
                CASE WHEN kind = 'tool' AND at <= ?6 THEN '' ELSE name END AS name,
                CASE WHEN kind = 'tool' AND at <= ?6 THEN '' ELSE namespace END AS namespace
            FROM calls
        ), groups AS (
            SELECT kind, name, namespace, COUNT(*) AS count FROM visible_calls
            GROUP BY kind, name, namespace
        ), top_groups AS (
            SELECT * FROM groups ORDER BY count DESC, kind, name, namespace LIMIT 100
        ), buckets AS (
            SELECT c.kind, c.name, c.namespace, (c.at - ?1) / ?5 AS bucket,
                COUNT(*) AS count, MIN(c.at) AS first_at
            FROM visible_calls c JOIN top_groups g
                ON g.kind = c.kind AND g.name = c.name AND g.namespace = c.namespace
            GROUP BY c.kind, c.name, c.namespace, bucket
        )
        SELECT g.kind, g.name, g.namespace, g.count, b.bucket, b.count, b.first_at,
            (SELECT COUNT(*) FROM groups) FROM top_groups g JOIN buckets b
            ON b.kind = g.kind AND b.name = g.name AND b.namespace = g.namespace
        ORDER BY g.count DESC, g.kind, g.name, g.namespace, b.bucket"
    ))?;
    let mut rows = statement.query(params_from_iter(args.iter()))?;
    let mut groups: Vec<serde_json::Value> = Vec::new();
    let mut previous = None;
    let mut total_groups = 0u64;
    while let Some(row) = rows.next()? {
        let key = (row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?);
        if previous.as_ref() != Some(&key) {
            groups.push(serde_json::json!({
                "kind":key.0,"name":key.1,"namespace":key.2,
                "count":row.get::<_, u64>(3)?,"timeline":[]
            }));
            previous = Some(key);
        }
        groups.last_mut().unwrap()["timeline"].as_array_mut().unwrap().push(
            serde_json::json!([row.get::<_, u64>(4)?, row.get::<_, u64>(5)?, row.get::<_, i64>(6)?])
        );
        total_groups = row.get(7)?;
    }
    Ok(serde_json::json!({
        "groups":groups,"groupCount":total_groups,"startAt":request.start_ms,
        "bucketWidthMs":bucket_width,"bucketCount":64
    }))
}

fn revision(db: &Connection) -> anyhow::Result<i64> {
    Ok(db.query_row(
        "SELECT revision FROM usage_meta WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?)
}

/// 调用身份去重后按时间、会话和调用 ID 稳定倒序分页。
pub(super) fn query(db: &Connection, request: &ExecutionQuery<'_>) -> anyhow::Result<PageLookup> {
    ensure!(request.start_ms < request.end_ms, "invalid execution range");
    ensure!(
        (1..=200).contains(&request.limit),
        "invalid execution page size"
    );
    ensure!(request.search.len() <= 512, "execution search is too long");
    ensure!(
        request
            .kind
            .is_none_or(|kind| matches!(kind, "skill" | "mcp" | "tool")),
        "invalid execution kind"
    );
    let current_revision = revision(db)?;
    if request
        .revision
        .is_some_and(|value| value != current_revision)
    {
        return Ok(PageLookup::Stale);
    }
    let cursor = request.cursor.map(decode_cursor).transpose()?;
    ensure!(
        cursor.is_none() || request.revision.is_some(),
        "execution cursor requires revision"
    );
    if request.sessions.is_some_and(|sessions| sessions.is_empty()) {
        return Ok(PageLookup::Ready(ExecutionPage {
            revision: current_revision,
            items: Vec::new(),
            next_cursor: None,
            total: 0,
            matched: 0,
            conflicts: 0,
            recent_skill: 0,
            recent_mcp: 0,
            recent_tool: 0,
            skill: 0,
            mcp: 0,
            tool: 0,
            classified: 0,
            search_applied: !request.search.is_empty(),
        }));
    }
    let mut arguments = vec![
        rusqlite::types::Value::Integer(request.start_ms),
        rusqlite::types::Value::Integer(request.end_ms),
        rusqlite::types::Value::Integer(request.now_ms.saturating_sub(DETAIL_RETENTION_MS)),
        rusqlite::types::Value::Text(request.search.to_owned()),
        rusqlite::types::Value::Text(request.kind.unwrap_or("").to_owned()),
        rusqlite::types::Value::Integer(request.now_ms),
    ];
    let session_filter = if let Some(sessions) = request.sessions {
        let first = arguments.len() + 1;
        arguments.extend(sessions.iter().cloned().map(rusqlite::types::Value::Text));
        let parameters = (first..first + sessions.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(" AND e.session IN ({parameters})")
    } else {
        String::new()
    };
    let cursor_filter = if let Some(cursor) = cursor {
        let first = arguments.len() + 1;
        arguments.extend([
            rusqlite::types::Value::Integer(cursor.timestamp_ms),
            rusqlite::types::Value::Text(cursor.session),
            rusqlite::types::Value::Text(cursor.id),
        ]);
        format!(
            " WHERE (timestamp_ms, session, call_id) < (?{first}, ?{}, ?{})",
            first + 1,
            first + 2
        )
    } else {
        String::new()
    };
    arguments.push(rusqlite::types::Value::Integer(i64::try_from(
        request.limit + 1,
    )?));
    let limit_parameter = arguments.len();
    let sql = format!(
        "WITH candidates AS (
            SELECT DISTINCT e.session, e.call_id
            FROM execution_observations e
            WHERE e.is_output = 0 AND
                (e.name IS NULL OR (e.name != 'write_stdin' AND e.name NOT LIKE '%.write_stdin'))
                AND e.timestamp_ms >= ?1 AND e.timestamp_ms < ?2{session_filter}
        ), grouped AS (
            SELECT e.session, e.call_id,
                MIN(CASE WHEN e.timestamp_ms >= ?1 AND e.timestamp_ms < ?2
                    THEN e.timestamp_ms END) AS timestamp_ms,
                MIN(e.name) AS name, MIN(e.namespace) AS namespace, MIN(e.call_kind) AS call_kind,
                CASE WHEN MIN(e.timestamp_ms) != MAX(e.timestamp_ms)
                    OR COUNT(e.name) != COUNT(*) AND COUNT(e.name) != 0
                    OR MIN(e.name) != MAX(e.name)
                    OR MIN(e.namespace) != MAX(e.namespace)
                    OR MIN(e.call_kind) != MAX(e.call_kind)
                THEN 1 ELSE 0 END AS conflicting
            FROM candidates c JOIN execution_observations e
                ON e.session = c.session AND e.call_id = c.call_id AND e.is_output = 0
            GROUP BY e.session, e.call_id
        ), selected AS (
            SELECT * FROM grouped WHERE (?4 = '' OR (timestamp_ms <= ?6 AND conflicting = 0 AND
                    (instr(lower(COALESCE(name, '')), lower(?4)) > 0
                     OR instr(lower(COALESCE(namespace, '')), lower(?4)) > 0)))
                AND (?5 = '' OR (conflicting = 0 AND timestamp_ms <= ?6 AND call_kind = ?5))
                AND (?4 = '' AND ?5 != 'tool' OR call_kind != 'tool' OR timestamp_ms > ?3)
        ), summary AS (
            SELECT COUNT(*) AS total, COALESCE(SUM(conflicting), 0) AS conflicts,
                COALESCE(SUM(CASE WHEN timestamp_ms > ?3 AND timestamp_ms <= ?6 AND conflicting = 0 AND call_kind = 'skill' THEN 1 ELSE 0 END), 0) AS recent_skill,
                COALESCE(SUM(CASE WHEN timestamp_ms > ?3 AND timestamp_ms <= ?6 AND conflicting = 0 AND call_kind = 'mcp' THEN 1 ELSE 0 END), 0) AS recent_mcp,
                COALESCE(SUM(CASE WHEN timestamp_ms > ?3 AND timestamp_ms <= ?6 AND conflicting = 0 AND call_kind = 'tool' THEN 1 ELSE 0 END), 0) AS recent_tool,
                COALESCE(SUM(conflicting = 0 AND timestamp_ms <= ?6 AND call_kind = 'skill'), 0) AS skill,
                COALESCE(SUM(conflicting = 0 AND timestamp_ms <= ?6 AND call_kind = 'mcp'), 0) AS mcp,
                COALESCE(SUM(conflicting = 0 AND timestamp_ms <= ?6 AND call_kind = 'tool'), 0) AS tool
            FROM grouped
        ), result_page AS (
            SELECT * FROM selected{cursor_filter}
            ORDER BY timestamp_ms DESC, session DESC, call_id DESC LIMIT ?{limit_parameter}
        )
        SELECT result_page.session, result_page.call_id, result_page.timestamp_ms, result_page.name,
            result_page.namespace, result_page.call_kind, result_page.conflicting,
            summary.total, summary.conflicts, summary.recent_skill, summary.recent_mcp,
            summary.recent_tool, (SELECT COUNT(*) FROM selected) AS matched,
            summary.skill, summary.mcp, summary.tool
        FROM summary LEFT JOIN result_page ON 1
        ORDER BY result_page.timestamp_ms DESC, result_page.session DESC, result_page.call_id DESC"
    );
    let mut statement = db.prepare(&sql)?;
    let mut rows = statement.query(params_from_iter(arguments.iter()))?;
    let mut items = Vec::new();
    let mut total = 0;
    let mut matched = 0;
    let mut conflicts = 0;
    let mut recent_skill = 0;
    let mut recent_mcp = 0;
    let mut recent_tool = 0;
    let mut skill = 0;
    let mut mcp = 0;
    let mut tool = 0;
    while let Some(row) = rows.next()? {
        total = row.get(7)?;
        conflicts = row.get(8)?;
        recent_skill = row.get(9)?;
        recent_mcp = row.get(10)?;
        recent_tool = row.get(11)?;
        matched = row.get(12)?;
        skill = row.get(13)?;
        mcp = row.get(14)?;
        tool = row.get(15)?;
        let Some(session_id) = row.get::<_, Option<String>>(0)? else {
            continue;
        };
        let timestamp_ms: i64 = row.get(2)?;
        let conflicting: bool = row.get(6)?;
        let visible = timestamp_ms <= request.now_ms && !conflicting;
        items.push(ExecutionItem {
            session_id,
            id: row.get(1)?,
            timestamp_ms,
            name: visible
                .then(|| row.get::<_, Option<String>>(3))
                .transpose()?
                .flatten(),
            namespace: visible
                .then(|| row.get::<_, Option<String>>(4))
                .transpose()?
                .flatten(),
            kind: visible
                .then(|| row.get::<_, Option<String>>(5))
                .transpose()?
                .flatten(),
        });
    }
    let next_cursor = if items.len() > request.limit {
        items.pop();
        let last = items.last().expect("nonempty execution page");
        Some(encode_cursor(&Cursor {
            timestamp_ms: last.timestamp_ms,
            session: last.session_id.clone(),
            id: last.id.clone(),
        })?)
    } else {
        None
    };
    Ok(PageLookup::Ready(ExecutionPage {
        revision: current_revision,
        items,
        next_cursor,
        total,
        matched,
        conflicts,
        recent_skill,
        recent_mcp,
        recent_tool,
        skill,
        mcp,
        tool,
        classified: skill + mcp + tool,
        search_applied: !request.search.is_empty(),
    }))
}

#[derive(Debug)]
pub(super) struct ExecutionLocator {
    pub(super) source: Vec<u8>,
    pub(super) identity: Vec<u8>,
    pub(super) start: u64,
    pub(super) end: u64,
    pub(super) timestamp_ms: i64,
    pub(super) call_id: String,
}

pub(super) struct ExecutionLocators {
    pub(super) call: ExecutionLocator,
    pub(super) outputs: Vec<ExecutionLocator>,
    pub(super) outputs_truncated: bool,
}

pub(super) enum DetailLookup {
    Ready(ExecutionLocators),
    Stale,
    NotFound,
}

/// 只返回同一来源的定位，避免将不同副本的输出拼到同一调用。
pub(super) fn detail(
    db: &Connection,
    session: &str,
    id: &str,
    expected_revision: i64,
    now_ms: i64,
) -> anyhow::Result<DetailLookup> {
    if revision(db)? != expected_revision {
        return Ok(DetailLookup::Stale);
    }
    let cutoff = now_ms.saturating_sub(DETAIL_RETENTION_MS);
    let aggregate: Option<(
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = db
        .query_row(
            "SELECT MIN(timestamp_ms), MAX(timestamp_ms), MIN(name), MAX(name),
                MIN(namespace), MAX(namespace), MIN(call_kind), MAX(call_kind)
             FROM execution_observations
             WHERE session = ?1 AND call_id = ?2 AND is_output = 0
             HAVING COUNT(*) > 0",
            params![session, id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()?;
    let Some((first, last, min_name, max_name, min_namespace, max_namespace, min_kind, max_kind)) =
        aggregate
    else {
        return Ok(DetailLookup::NotFound);
    };
    if first != last
        || first <= cutoff
        || first > now_ms
        || min_name != max_name
        || min_namespace != max_namespace
        || min_kind != max_kind
    {
        return Ok(DetailLookup::NotFound);
    }
    let call = db
        .query_row(
            "SELECT e.source, s.identity, e.start, COALESCE(e.end, 0), e.timestamp_ms, e.call_id
         FROM execution_observations e JOIN usage_sources s ON s.source = e.source
         WHERE e.session = ?1 AND e.call_id = ?2 AND e.is_output = 0
         ORDER BY (e.end IS NOT NULL) DESC, (SELECT COUNT(*) FROM execution_observations o
                   WHERE o.source = e.source AND o.session = e.session AND o.call_id = e.call_id
                         AND o.is_output = 1 AND o.end IS NOT NULL AND o.start > e.start
                         AND o.timestamp_ms > ?3 AND o.timestamp_ms <= ?4) DESC,
             e.source, e.start LIMIT 1",
            params![session, id, cutoff, now_ms],
            read_locator,
        )
        .optional()?;
    let Some(call) = call else {
        return Ok(DetailLookup::NotFound);
    };
    let mut statement = db.prepare(
        "WITH root_process AS (
             SELECT DISTINCT process_id FROM execution_observations
             WHERE source = ?1 AND session = ?2 AND call_id = ?3 AND is_output = 1
                 AND process_id IS NOT NULL AND start > ?5 AND timestamp_ms > ?4
                 AND timestamp_ms <= ?7
         ), unique_process AS (
             SELECT process_id FROM root_process
             WHERE (SELECT COUNT(*) FROM root_process) = 1
                 AND (SELECT COUNT(DISTINCT origin.call_id)
                      FROM execution_observations origin
                      JOIN execution_observations origin_call
                          ON origin_call.source = origin.source
                          AND origin_call.session = origin.session
                          AND origin_call.call_id = origin.call_id
                          AND origin_call.is_output = 0
                      WHERE origin.session = ?2 AND origin.is_output = 1
                          AND origin.process_id = root_process.process_id
                          AND origin.timestamp_ms > ?4 AND origin.timestamp_ms <= ?7
                          AND origin_call.timestamp_ms > ?4 AND origin_call.timestamp_ms <= ?7
                          AND origin_call.name != 'write_stdin'
                          AND origin_call.name NOT LIKE '%.write_stdin') = 1
         ), polls AS (
             SELECT poll.call_id, poll.start
             FROM execution_observations poll
             JOIN unique_process process ON poll.process_id = process.process_id
             WHERE poll.source = ?1 AND poll.session = ?2
                 AND poll.is_output = 0
                 AND poll.timestamp_ms > ?4 AND poll.timestamp_ms <= ?7
                 AND (poll.name = 'write_stdin' OR poll.name LIKE '%.write_stdin')
                 AND poll.start > ?5
         )
         SELECT e.source, s.identity, e.start, e.end, e.timestamp_ms, e.call_id
         FROM execution_observations e JOIN usage_sources s ON s.source = e.source
         WHERE e.source = ?1 AND e.session = ?2 AND e.is_output = 1
             AND e.end IS NOT NULL AND e.timestamp_ms > ?4 AND e.timestamp_ms <= ?7
             AND e.start > ?5
             AND (e.call_id = ?3 OR EXISTS (
                 SELECT 1 FROM polls WHERE polls.call_id = e.call_id AND polls.start < e.start
             ))
         ORDER BY e.start LIMIT ?6",
    )?;
    let rows = statement.query_map(
        params![
            call.source,
            session,
            id,
            cutoff,
            i64::try_from(call.start)?,
            i64::try_from(MAX_OUTPUT_SEGMENTS + 1)?,
            now_ms,
        ],
        read_locator,
    )?;
    let mut outputs = rows.collect::<Result<Vec<_>, _>>()?;
    let ambiguous_process: bool = db.query_row(
        "WITH root_process AS (
             SELECT DISTINCT process_id FROM execution_observations
             WHERE source = ?1 AND session = ?2 AND call_id = ?3 AND is_output = 1
                 AND process_id IS NOT NULL AND start > ?4 AND timestamp_ms > ?5
                 AND timestamp_ms <= ?6
         )
         SELECT (SELECT COUNT(*) FROM root_process) > 1 OR EXISTS (
             SELECT 1 FROM root_process process
             WHERE (SELECT COUNT(DISTINCT origin.call_id)
                    FROM execution_observations origin
                    JOIN execution_observations origin_call
                        ON origin_call.source = origin.source
                        AND origin_call.session = origin.session
                        AND origin_call.call_id = origin.call_id
                        AND origin_call.is_output = 0
                    WHERE origin.session = ?2 AND origin.is_output = 1
                        AND origin.process_id = process.process_id
                        AND origin.timestamp_ms > ?5 AND origin.timestamp_ms <= ?6
                        AND origin_call.timestamp_ms > ?5 AND origin_call.timestamp_ms <= ?6
                        AND origin_call.name != 'write_stdin'
                        AND origin_call.name NOT LIKE '%.write_stdin') > 1
                 OR EXISTS (
                     SELECT 1 FROM execution_observations foreign_poll
                     WHERE foreign_poll.session = ?2 AND foreign_poll.is_output = 0
                         AND foreign_poll.process_id = process.process_id
                         AND foreign_poll.timestamp_ms > ?5 AND foreign_poll.timestamp_ms <= ?6
                         AND (foreign_poll.name = 'write_stdin'
                              OR foreign_poll.name LIKE '%.write_stdin')
                         AND NOT EXISTS (
                             SELECT 1 FROM execution_observations local_poll
                             WHERE local_poll.source = ?1 AND local_poll.session = ?2
                                 AND local_poll.call_id = foreign_poll.call_id
                                 AND local_poll.is_output = 0
                                 AND local_poll.timestamp_ms > ?5 AND local_poll.timestamp_ms <= ?6
                         )
                 )
         )",
        params![call.source, session, id, i64::try_from(call.start)?, cutoff, now_ms],
        |row| row.get(0),
    )?;
    let source_incomplete: bool = db.query_row(
        "SELECT execution_indexed = 0 OR execution_valid = 0
         FROM usage_sources WHERE source = ?1",
        [&call.source],
        |row| row.get(0),
    )?;
    let outputs_truncated =
        outputs.len() > MAX_OUTPUT_SEGMENTS || ambiguous_process || source_incomplete;
    outputs.truncate(MAX_OUTPUT_SEGMENTS);
    Ok(DetailLookup::Ready(ExecutionLocators {
        call,
        outputs,
        outputs_truncated,
    }))
}

fn read_locator(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExecutionLocator> {
    Ok(ExecutionLocator {
        source: row.get(0)?,
        identity: row.get(1)?,
        start: row.get(2)?,
        end: row.get(3)?,
        timestamp_ms: row.get(4)?,
        call_id: row.get(5)?,
    })
}

#[cfg(test)]
#[path = "execution_index_tests.rs"]
mod tests;
