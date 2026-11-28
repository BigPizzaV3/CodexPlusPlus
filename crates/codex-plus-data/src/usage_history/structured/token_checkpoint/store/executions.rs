use super::super::SourceStamp;
use super::{Store, decode_path};
use crate::usage_history::{
    normalize_session_id,
    structured::{
        FileState, exec_receipt,
        execution_observations::{DETAIL_MS, LIST_MS},
        log_file,
    },
};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

pub(super) const COMPACTABLE_METADATA: &str =
    "name IS NOT NULL AND name != 'write_stdin' AND substr(name,-12) != '.write_stdin'
     AND substr(name, 1, 5) != 'mcp__' AND substr(COALESCE(namespace,''), 1, 5) != 'mcp__'";

pub(in super::super) struct CompactionCursor(i64, i64, String);

fn local_midnight(day: NaiveDate) -> Result<i64> {
    Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .map(|at| at.timestamp_millis())
        .context("本地日期没有对应的零点")
}

impl Store {
    pub(in super::super) fn evict_execution_days(
        &mut self,
        cutoff: NaiveDate,
        batch: u32,
    ) -> Result<u32> {
        let tx = self.db.transaction()?;
        let (removed, through) = {
            let mut statement = tx.prepare_cached(
                "DELETE FROM execution_daily WHERE (day, session_id) IN (
                    SELECT day, session_id FROM execution_daily WHERE day < ?1
                    ORDER BY day, session_id LIMIT ?2
                 ) RETURNING day",
            )?;
            let mut rows = statement.query(params![cutoff.to_string(), batch])?;
            let mut removed = 0;
            let mut through = String::new();
            while let Some(row) = rows.next()? {
                through = through.max(row.get(0)?);
                removed += 1;
            }
            (removed, through)
        };
        if removed != 0 {
            tx.execute(
                "INSERT INTO execution_evicted VALUES(1,?1) ON CONFLICT(singleton)
                 DO UPDATE SET through_day=max(through_day,excluded.through_day)",
                [through],
            )?;
            tx.execute(
                "UPDATE token_revision SET revision=revision+1, updated_at=max(updated_at+1,?1)",
                [Local::now().timestamp_millis()],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }

    pub(in super::super) fn compact_execution_metadata(
        &mut self,
        now_ms: i64,
        batch: u32,
    ) -> Result<()> {
        // 技能名称单独存于 token_activity，MCP 名称和轮询识别仍需保留。
        self.db
            .prepare_cached(&format!(
                "UPDATE execution_observations SET name=NULL, namespace=NULL
             WHERE (source_id, position) IN (
                 SELECT source_id, position FROM execution_observations
                 WHERE is_output = 0 AND {COMPACTABLE_METADATA} AND timestamp_ms <= ?1
                 ORDER BY timestamp_ms LIMIT ?2
             )"
            ))?
            .execute(params![now_ms - LIST_MS, batch])?;
        Ok(())
    }

    pub(in super::super) fn compact_executions(
        &mut self,
        cutoff: NaiveDate,
        cursor: &mut Option<CompactionCursor>,
        batch: u32,
    ) -> Result<usize> {
        let boundary = local_midnight(cutoff)?;
        let tx = self.db.transaction()?;
        let candidates = {
            let mut statement = tx.prepare_cached(
                "SELECT timestamp_ms, session_id, call_id FROM execution_observations
                 WHERE is_output = 0 AND timestamp_ms < ?1
                   AND (timestamp_ms, session_id, call_id) > (?2, ?3, ?4)
                 ORDER BY timestamp_ms, session_id, call_id LIMIT ?5",
            )?;
            let (at, session, key) = cursor
                .as_ref()
                .map_or((i64::MIN, 0, ""), |c| (c.0, c.1, c.2.as_str()));
            statement
                .query_map(params![boundary, at, session, key, batch], |row| {
                    Ok(CompactionCursor(row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut removed = 0;
        let mut looked_up = 0;
        let mut processed = 0;
        let mut seen = HashSet::new();
        for candidate in &candidates {
            let CompactionCursor(_, session, id) = candidate;
            if !seen.insert((session, id)) {
                processed += 1;
                continue;
            }
            if looked_up >= batch as usize * 2 || removed >= batch as usize {
                break;
            }
            let (count, first, last, consistent, closed, partial, skill, mcp, poll) = tx.query_row(
                &format!("WITH rows AS (
                    SELECT e.timestamp_ms,
                        CASE WHEN {COMPACTABLE_METADATA} THEN NULL ELSE e.name END AS name,
                        CASE WHEN {COMPACTABLE_METADATA} THEN NULL ELSE e.namespace END AS namespace,
                        COALESCE(CAST(s.checkpoint AS TEXT) ->> '$.resume_pending', 1) = 0 AS closed,
                        s.activity_truncated AS partial
                    FROM execution_observations e JOIN token_sources s ON s.id = e.source_id
                    WHERE e.session_id = ?1 AND e.call_id = ?2 AND e.is_output = 0 LIMIT ?3
                 ) SELECT count(*), min(timestamp_ms), max(timestamp_ms),
                    min(COALESCE(name,'')) = max(COALESCE(name,''))
                      AND min(COALESCE(namespace,'')) = max(COALESCE(namespace,'')),
                    min(closed), max(partial),
                    COALESCE((
                        SELECT json_array_length(CAST(a.payload AS TEXT), '$.call.arguments.skillNames') > 0
                        FROM token_activity a WHERE a.session_id = ?1 AND a.call_id = ?2
                        ORDER BY a.source_id DESC LIMIT 1
                    ), 0),
                    COALESCE(substr(min(name),1,5) = 'mcp__' OR substr(min(namespace),1,5) = 'mcp__', 0),
                    COALESCE(min(name) = 'write_stdin' OR substr(min(name),-12) = '.write_stdin', 0) FROM rows"),
                params![session, id, batch + 1],
                |row| Ok((row.get::<_, usize>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?,
                    row.get::<_, bool>(3)?, row.get::<_, bool>(4)?, row.get::<_, bool>(5)?,
                    row.get::<_, bool>(6)?, row.get::<_, bool>(7)?, row.get::<_, bool>(8)?)),
            )?;
            looked_up += count;
            if count > batch as usize || !closed || last >= boundary {
                processed += 1;
                continue;
            }
            if removed + count > batch as usize {
                break;
            }
            let day = DateTime::from_timestamp_millis(first)
                .context("工具时间无效")?
                .with_timezone(&Local)
                .date_naive()
                .to_string();
            // 先释放详情页，再写入日计数。任何写失败都保留原始索引行。
            removed += tx.execute(
                "DELETE FROM execution_observations
                 WHERE session_id = ?1 AND call_id = ?2 AND is_output = 0",
                params![session, id],
            )?;
            let exact = consistent && first == last;
            if !poll || !exact {
                tx.execute(
                    "INSERT INTO execution_daily VALUES(?1,?2,1,?3,?4,?5,?6)
                     ON CONFLICT(day,session_id) DO UPDATE SET count=count+1,
                     skill=skill+excluded.skill, mcp=mcp+excluded.mcp, tool=tool+excluded.tool,
                     partial=max(partial,excluded.partial)",
                    params![
                        day,
                        session,
                        exact && skill,
                        exact && !skill && mcp,
                        exact && !skill && !mcp,
                        partial || !exact
                    ],
                )?;
            }
            processed += 1;
        }
        if removed != 0 {
            tx.execute(
                "UPDATE token_revision SET revision=revision+1, updated_at=max(updated_at+1,?1)",
                [Local::now().timestamp_millis()],
            )?;
        }
        tx.commit()?;
        // 扫描未完成来源及异常大的身份时仍可前进；到达末尾后允许下一轮重试。
        *cursor = if processed == candidates.len() && candidates.len() < batch as usize {
            None
        } else {
            candidates.into_iter().nth(processed.saturating_sub(1))
        };
        Ok(removed)
    }
}

#[derive(Deserialize, Serialize)]
struct Cursor(i64, String, String);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::usage_history::structured) struct ExecutionQuery {
    start_at: i64,
    end_at: i64,
    scope_session_id: Option<String>,
    #[serde(default = "default_limit")]
    limit: u16,
    revision: Option<i64>,
    cursor: Option<String>,
    #[serde(default)]
    search: String,
    kind: Option<String>,
    #[serde(default)]
    include_activity_summary: bool,
}

fn default_limit() -> u16 {
    50
}

impl ExecutionQuery {
    pub(in crate::usage_history::structured) fn parse(request: &Value) -> Result<Self> {
        let mut query = Self::deserialize(request)?;
        ensure!(
            query.start_at < query.end_at
                && DateTime::from_timestamp_millis(query.start_at).is_some()
                && DateTime::from_timestamp_millis(query.end_at - 1).is_some(),
            "工具时间范围无效"
        );
        ensure!((1..=200).contains(&query.limit), "工具分页大小无效");
        ensure!(
            query
                .kind
                .as_deref()
                .is_none_or(|kind| matches!(kind, "tool" | "skill" | "mcp")),
            "工具类别无效"
        );
        ensure!(query.revision.is_none_or(|rev| rev >= 0), "工具版本无效");
        if let Some(scope) = &mut query.scope_session_id {
            *scope = normalize_session_id(scope).context("会话范围 ID 无效")?;
        }
        if query.cursor.is_some() {
            ensure!(query.revision.is_some(), "工具游标需要版本");
            query.cursor()?;
        }
        Ok(query)
    }

    fn cursor(&self) -> Result<Option<Cursor>> {
        let Some(cursor) = &self.cursor else {
            return Ok(None);
        };
        let cursor: Cursor = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(cursor)?)?;
        ensure!(
            normalize_session_id(&cursor.1).is_some() && !cursor.2.is_empty(),
            "工具游标无效"
        );
        Ok(Some(cursor))
    }
}

// 先按时间索引找到调用身份，再检查该身份的所有副本。跨范围的冲突不能被过滤掉。
static CALLS: LazyLock<String> = LazyLock::new(|| {
    format!("
WITH candidates AS (
    SELECT DISTINCT e.session_id, e.call_id FROM execution_observations e
    WHERE e.is_output = 0 AND e.timestamp_ms >= ?1 AND e.timestamp_ms < ?2
      AND (?3 IS NULL OR e.session_id IN (
          SELECT s.id FROM json_each(?3) j JOIN token_sessions s ON s.session = j.value
      ))
), copies AS (
    SELECT e.session_id, e.call_id, e.timestamp_ms,
        CASE WHEN e.timestamp_ms <= ?4 AND {COMPACTABLE_METADATA} THEN '' ELSE COALESCE(e.name,'') END AS name,
        CASE WHEN e.timestamp_ms <= ?4 AND {COMPACTABLE_METADATA} THEN '' ELSE COALESCE(e.namespace,'') END AS namespace
    FROM candidates c JOIN execution_observations e
      ON e.session_id = c.session_id AND e.call_id = c.call_id AND e.is_output = 0
), identities AS (
    SELECT session_id, call_id, MIN(timestamp_ms) AS at, MIN(name) AS name, MIN(namespace) AS namespace,
        MIN(timestamp_ms) != MAX(timestamp_ms) OR MIN(name) != MAX(name) OR MIN(namespace) != MAX(namespace) AS conflict
    FROM copies GROUP BY session_id, call_id
), annotated AS (
    SELECT i.*, s.session, (
        SELECT json_extract(CAST(a.payload AS TEXT), '$.call.arguments.skillNames')
        FROM token_activity a WHERE a.session_id = i.session_id AND a.call_id = i.call_id
        ORDER BY a.source_id DESC LIMIT 1
    ) AS skills
    FROM identities i JOIN token_sessions s ON s.id = i.session_id
    WHERE i.conflict OR (i.name != 'write_stdin' AND substr(i.name,-12) != '.write_stdin')
), calls AS (
    SELECT *, CASE
        WHEN json_array_length(skills) > 0 THEN 'skill'
        WHEN substr(name, 1, 5) = 'mcp__' OR substr(namespace, 1, 5) = 'mcp__' THEN 'mcp'
        ELSE 'tool'
    END AS kind
    FROM annotated
), visible AS (
    SELECT at, session, call_id, kind, skills,
        COALESCE(NULLIF(name,''),json_extract(skills,'$[0]'),'') AS name, namespace
    FROM calls WHERE NOT conflict
), filtered AS (
    SELECT * FROM visible WHERE (?5 IS NULL OR kind = ?5)
      AND (kind != 'tool' OR at > ?4)
      AND (instr(lower(name), lower(?6)) > 0 OR instr(lower(namespace), lower(?6)) > 0
           OR instr(lower(COALESCE(skills,'')), lower(?6)) > 0)
 )")
});

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Daily {
    day: String,
    count: u64,
    skill: u64,
    mcp: u64,
    tool: u64,
    partial: bool,
    classification_complete: bool,
}

impl Store {
    pub(in crate::usage_history::structured) fn read_execution_page(
        db: &mut Connection,
        query: &ExecutionQuery,
        now_ms: i64,
        processing: bool,
        storage_error: Option<&str>,
    ) -> Result<Value> {
        let tx = db.transaction()?;
        let (revision, precise_from): (i64, String) = tx.query_row(
            "SELECT revision, precise_from FROM token_revision",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if query.revision.is_some_and(|rev| rev != revision) {
            return Ok(json!({"status":"stale","complete":false}));
        }
        let scope = if let Some(requested) = &query.scope_session_id {
            let tree = Self::scope(&tx, &BTreeMap::new(), true)?;
            let (_, members) = tree.scope_members(requested);
            Some(serde_json::to_string(&members)?)
        } else {
            None
        };
        let cutoff = (!precise_from.is_empty())
            .then(|| local_midnight(NaiveDate::parse_from_str(&precise_from, "%Y-%m-%d")?))
            .transpose()?;
        let range_missing = cutoff.is_none_or(|cutoff| query.start_at < cutoff);
        let capture_since: Option<i64> = tx.query_row(
            "SELECT started_at FROM execution_coverage_start WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        let capture_missing = capture_since.is_some_and(|since| query.start_at < since);
        let start_day = DateTime::from_timestamp_millis(query.start_at)
            .unwrap()
            .with_timezone(&Local)
            .date_naive();
        let storage_pruned: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM execution_evicted WHERE through_day >= ?1)",
            [start_day.to_string()],
            |row| row.get(0),
        )?;
        let from = query.start_at.max(cutoff.unwrap_or(query.end_at));
        let until = query.end_at.min(now_ms.saturating_add(1));
        let calls = CALLS.as_str();
        let detail_from = now_ms - DETAIL_MS;
        let list_from = now_ms - LIST_MS;
        let aggregate = tx.query_row(
            &format!(
                "{calls}
                SELECT json_object('total', count(*),
                    'conflicts', (SELECT count(*) FROM calls WHERE conflict),
                    'matched', (SELECT count(*) FROM filtered),
                    'skill', count(*) FILTER (WHERE kind = 'skill'),
                    'mcp', count(*) FILTER (WHERE kind = 'mcp'),
                    'tool', count(*) FILTER (WHERE kind = 'tool'),
                    'recentSkill', count(*) FILTER (WHERE kind = 'skill' AND at > ?7),
                    'recentMcp', count(*) FILTER (WHERE kind = 'mcp' AND at > ?7),
                    'recentTool', count(*) FILTER (WHERE kind = 'tool' AND at > ?7)) FROM visible"
            ),
            params![
                from,
                until,
                scope,
                list_from,
                query.kind,
                query.search,
                detail_from
            ],
            |row| row.get::<_, String>(0),
        )?;
        let mut summary: Value = serde_json::from_str(&aggregate)?;
        let mut daily = Vec::new();
        if let Some(cutoff) = cutoff
            && query.start_at < cutoff
        {
            let old_end = cutoff.min(query.end_at);
            let day_at = |at| {
                DateTime::from_timestamp_millis(at)
                    .unwrap()
                    .with_timezone(&Local)
                    .date_naive()
            };
            let mut statement = tx.prepare_cached(
                "SELECT d.day,sum(d.count),sum(d.skill),sum(d.mcp),sum(d.tool),max(d.partial)
                 FROM execution_daily d WHERE d.day >= ?1 AND d.day <= ?2
                 AND (?3 IS NULL OR d.session_id IN (
                    SELECT s.id FROM json_each(?3) j JOIN token_sessions s ON s.session = j.value
                 )) GROUP BY d.day ORDER BY d.day DESC",
            )?;
            let mut rows = statement.query(params![
                day_at(query.start_at).to_string(),
                day_at(old_end - 1).to_string(),
                scope,
            ])?;
            while let Some(row) = rows.next()? {
                let day: String = row.get(0)?;
                let date = NaiveDate::parse_from_str(&day, "%Y-%m-%d")?;
                let count: u64 = row.get(1)?;
                let (skill, mcp, tool): (u64, u64, u64) = (row.get(2)?, row.get(3)?, row.get(4)?);
                let partial: bool = row.get(5)?;
                if query.start_at <= local_midnight(date)?
                    && old_end >= local_midnight(date.succ_opt().context("工具汇总日期超出范围")?)?
                {
                    for (field, amount) in [
                        ("total", count),
                        ("skill", skill),
                        ("mcp", mcp),
                        ("tool", tool),
                    ] {
                        summary[field] = json!(summary[field].as_u64().unwrap() + amount);
                    }
                }
                daily.push(Daily {
                    day,
                    count,
                    skill,
                    mcp,
                    tool,
                    partial,
                    classification_complete: !partial && count == skill + mcp + tool,
                });
            }
        }
        let indexed: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM token_coverage
                WHERE revision = ?1 AND pending_sources = 0 AND issues = 0 AND NOT maintenance_blocked
             ) AND NOT EXISTS(SELECT 1 FROM token_sources WHERE activity_truncated)",
            [revision],
            |row| row.get(0),
        )?;
        let complete = indexed
            && !processing
            && storage_error.is_none()
            && !range_missing
            && !capture_missing
            && !storage_pruned
            && summary["conflicts"] == 0;
        summary["classificationComplete"] = json!(complete);
        if !complete && summary["total"] == 0 {
            summary["total"] = Value::Null;
        }
        let cursor = query.cursor()?;
        let mut items = Vec::new();
        let mut next = None;
        {
            let mut statement = tx.prepare(&format!(
                "{calls}
                SELECT at, session, call_id, kind, name, namespace
                FROM filtered WHERE (?7 IS NULL OR (at, session, call_id) < (?7, ?8, ?9))
                ORDER BY at DESC, session DESC, call_id DESC LIMIT ?10"
            ))?;
            let mut rows = statement.query(params![
                from,
                until,
                scope,
                list_from,
                query.kind,
                query.search,
                cursor.as_ref().map(|cursor| cursor.0),
                cursor.as_ref().map(|cursor| &cursor.1),
                cursor.as_ref().map(|cursor| &cursor.2),
                u32::from(query.limit) + 1,
            ])?;
            let mut last = None;
            while let Some(row) = rows.next()? {
                if items.len() == usize::from(query.limit) {
                    next = Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&last.unwrap())?));
                    break;
                }
                let key = Cursor(row.get(0)?, row.get(1)?, row.get(2)?);
                // The cursor is kept separately so a short last page never advertises another page.
                items.push(json!({
                    "timestampMs":key.0,"sessionId":key.1,"id":key.2,
                    "kind":row.get::<_, String>(3)?,
                    "name":row.get::<_, String>(4)?,"namespace":row.get::<_, String>(5)?
                }));
                last = Some(key);
            }
        }
        let activity = if query.include_activity_summary && query.search.is_empty() {
            Some(activity_summary(
                &tx,
                query,
                from,
                until,
                scope.as_deref(),
                list_from,
                complete,
            )?)
        } else {
            None
        };
        tx.commit()?;
        Ok(json!({
            "status":if complete {"ok"} else {"partial"}, "complete":complete,
            "revision":revision,"items":items,"nextCursor":next,"summary":summary,
            "dailySummaries":daily, "activitySummary":activity, "searchApplied":!query.search.is_empty(),
            "coverage":{"complete":complete,"precision":if !daily.is_empty() || storage_pruned {"day"} else {"exact"},"reason":if storage_pruned {
                "storage-pruned"
            } else if range_missing {
                "outside-retention"
            } else if capture_missing {
                "before-tool-capture"
            } else if storage_error.is_some() {
                "storage-limited"
            } else if complete { "covered" } else { "discovery-unconfirmed" }},
            "scan":{"processing":processing,"storageError":storage_error}
        }))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::usage_history::structured) struct DetailQuery {
    session_id: String,
    id: String,
    revision: i64,
}

impl DetailQuery {
    pub(in crate::usage_history::structured) fn parse(request: &Value) -> Result<Self> {
        let mut query = Self::deserialize(request)?;
        query.session_id = normalize_session_id(&query.session_id).context("会话 ID 无效")?;
        ensure!(!query.id.is_empty(), "工具调用 ID 无效");
        ensure!(query.revision >= 0, "工具版本无效");
        Ok(query)
    }
}

struct Location {
    start: u64,
    end: u64,
    at: i64,
}

fn recent(at: i64, now: i64) -> bool {
    at <= now && at > now - DETAIL_MS
}

impl Store {
    pub(in crate::usage_history::structured) fn read_execution_detail(
        db: &mut Connection,
        home: &Path,
        query: &DetailQuery,
        now: impl Fn() -> i64,
    ) -> Result<Value> {
        let cutoff = now();
        let tx = db.transaction()?;
        let revision: i64 =
            tx.query_row("SELECT revision FROM token_revision", [], |row| row.get(0))?;
        if revision != query.revision {
            return Ok(json!({"status":"stale","revision":revision}));
        }
        let not_found = || json!({"status":"not_found","revision":revision});
        // 先证明调用身份只有一个可选来源，不把有分叉的两份原文拼在一起。
        let location = {
            let mut statement = tx.prepare(
                "SELECT e.source_id,e.position,e.end,e.timestamp_ms,s.source,s.offset,s.checkpoint
                 FROM execution_observations e JOIN token_sources s ON s.id = e.source_id
                 WHERE e.session_id = (SELECT id FROM token_sessions WHERE session = ?1)
                   AND e.call_id = ?2 AND NOT e.is_output LIMIT 2",
            )?;
            let mut rows = statement.query(params![query.session_id, query.id])?;
            let Some(row) = rows.next()? else {
                return Ok(not_found());
            };
            let Some(end): Option<u64> = row.get(2)? else {
                return Ok(not_found());
            };
            let call = Location {
                start: row.get(1)?,
                end,
                at: row.get(3)?,
            };
            if !recent(call.at, cutoff) {
                return Ok(not_found());
            }
            #[derive(Deserialize)]
            struct Source {
                stamp: Option<SourceStamp>,
            }
            let source: Source = serde_json::from_slice(row.get_ref(6)?.as_blob()?)?;
            let values = (
                row.get::<_, i64>(0)?,
                call,
                decode_path(row.get(4)?)?,
                row.get::<_, u64>(5)?,
                source.stamp,
            );
            if rows.next()?.is_some() {
                return Ok(json!({"status":"partial","revision":revision,"detail":{}}));
            }
            values
        };
        let (source_id, call, path, committed_offset, expected) = location;
        let mut statement = tx.prepare(
                "WITH direct AS (
                    SELECT e.* FROM execution_observations e
                    WHERE e.session_id = (SELECT id FROM token_sessions WHERE session = ?1)
                      AND e.call_id = ?2 AND e.source_id = ?3 AND e.is_output = 1 AND e.end IS NOT NULL
                      AND e.position >= ?4 AND e.timestamp_ms >= ?5 AND e.timestamp_ms <= ?6
                 ), launches AS (
                    SELECT p.process, min(e.position) AS opened, min(e.session_id) AS session_id
                    FROM direct e JOIN execution_process p USING(source_id,position)
                    GROUP BY p.process
                 ), chain AS (
                    SELECT c.* FROM launches c
                    WHERE (SELECT count(*) FROM launches) = 1
                    AND NOT EXISTS (
                        SELECT 1 FROM execution_process r JOIN execution_observations e USING(source_id,position)
                        WHERE r.process = c.process AND e.session_id = c.session_id
                          AND e.timestamp_ms > ?7 AND e.timestamp_ms <= ?6 AND (
                            e.source_id != ?3 OR (e.is_output = 1 AND e.call_id != ?2 AND NOT EXISTS (
                                SELECT 1 FROM execution_observations poll
                                  JOIN execution_process p USING(source_id,position)
                                WHERE poll.source_id = e.source_id AND poll.session_id = e.session_id
                                  AND poll.call_id = e.call_id AND poll.is_output = 0
                                  AND p.process = c.process AND poll.position < e.position
                            ))
                          )
                    )
                 ), polls AS (
                    SELECT e.*, c.process FROM chain c JOIN execution_process p ON p.process = c.process
                    JOIN execution_observations e USING(source_id,position)
                    WHERE e.session_id = c.session_id AND e.source_id = ?3 AND e.is_output = 0
                      AND e.position > c.opened AND e.timestamp_ms >= ?5 AND e.timestamp_ms <= ?6
                      AND (SELECT count(*) FROM execution_observations peer
                           WHERE peer.session_id = e.session_id AND peer.call_id = e.call_id
                             AND peer.is_output = 0) = 1
                 )
                 SELECT position,end,timestamp_ms,call_id,NULL AS process FROM direct
                 UNION ALL
                 SELECT o.position,o.end,o.timestamp_ms,o.call_id,p.process
                 FROM polls p JOIN execution_observations o
                   ON o.source_id = p.source_id AND o.session_id = p.session_id AND o.call_id = p.call_id
                 WHERE o.is_output = 1 AND o.end IS NOT NULL AND o.position >= p.end
                   AND o.timestamp_ms >= p.timestamp_ms AND o.timestamp_ms <= ?6
                 ORDER BY position",
        )?;
        let outputs = statement.query_map(
            params![
                query.session_id,
                query.id,
                source_id,
                call.end,
                call.at,
                cutoff,
                cutoff - DETAIL_MS,
            ],
            |row| {
                Ok((
                    Location {
                        start: row.get(0)?,
                        end: row.get(1)?,
                        at: row.get(2)?,
                    },
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )?;
        if !recent(call.at, now()) {
            return Ok(not_found());
        }
        let expected = expected.context("工具原会话没有已提交的文件身份")?;
        let compressed = path.extension().is_some_and(|ext| ext == "zst");
        ensure!(
            (path.starts_with(home.join("sessions"))
                || path.starts_with(home.join("archived_sessions")))
                && log_file(&path)
                && fs::canonicalize(&path)? == path,
            "工具原会话路径已变化"
        );
        let before = FileState::source_stamp(&path)?;
        ensure!(
            before.2.is_some()
                && before.2 == expected.2
                && (before == expected || !compressed && before.0 > expected.0),
            "工具原会话文件已变化"
        );
        let file = File::open(&path)?;
        ensure!(
            Some(codex_plus_core::codex_sqlite::opened_file_identity(&file)?) == before.2,
            "工具原会话身份已变化"
        );
        let mut reader = if compressed {
            DetailReader::Compressed(Box::new(zstd::stream::read::Decoder::new(file)?), 0)
        } else {
            DetailReader::Plain(file)
        };
        let mut item = reader.read_item(&call, committed_offset, &query.id, false)?;
        let shell = item["type"] == "local_shell_call"
            || item["name"].as_str().is_some_and(|name| {
                !item["namespace"]
                    .as_str()
                    .is_some_and(|namespace| namespace.starts_with("mcp__"))
                    && matches!(name.rsplit('.').next(), Some("exec_command" | "shell"))
            });
        let input = ["arguments", "input", "action"].into_iter()
            .find_map(|key| item.get_mut(key).map(Value::take))
            .unwrap_or(Value::Null);
        let input = match input {
            Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
            value => value,
        };
        let mut positions_end = call.end;
        let mut parts = Vec::new();
        let mut partial = false;
        let mut running = None;
        let mut terminal_at = None;
        for output in outputs {
            let (location, id, process) = output?;
            if let Some(process) = &process
                && (!shell
                    || before != expected
                    || terminal_at.is_some()
                    || running.as_ref() != Some(process))
            {
                continue;
            }
            if location.start < positions_end {
                partial = true;
                break;
            }
            let mut item = reader.read_item(&location, committed_offset, &id, true)?;
            let output = item.get_mut("output").map(Value::take).unwrap_or(Value::Null);
            if shell && let Some((started, id)) = exec_receipt(&output) {
                if started {
                    let observed = id.parse::<u64>().ok().map(|id| id.to_string());
                    if process.is_some() && process != observed {
                        partial = true;
                        break;
                    }
                    running = observed;
                } else {
                    terminal_at = Some(location.at);
                }
            }
            parts.push((location.at, output));
            positions_end = location.end;
        }
        drop(statement);
        tx.commit()?;
        ensure!(
            FileState::source_stamp(&path)? == before && fs::canonicalize(&path)? == path,
            "读取期间工具原会话发生变化"
        );
        let final_now = now();
        if !recent(call.at, final_now) {
            return Ok(not_found());
        }
        let old_parts = parts.len();
        parts.retain(|(at, _)| recent(*at, final_now));
        partial |= parts.is_empty() || parts.len() != old_parts || shell && terminal_at.is_none();
        let duration = if partial {
            None
        } else {
            terminal_at
                .or_else(|| parts.last().map(|(at, _)| *at))
                .map(|at| at - call.at)
        };
        let output: Vec<_> = parts.into_iter().map(|(_, part)| part).collect();
        Ok(json!({
            "status":if partial {"partial"} else {"ok"},"revision":revision,
            "detail":{"input":input,"output":output,"durationMs":duration}
        }))
    }
}

enum DetailReader {
    Plain(File),
    Compressed(Box<dyn Read>, u64),
}

impl DetailReader {
    fn read_item(
        &mut self,
        at: &Location,
        committed_end: u64,
        id: &str,
        output: bool,
    ) -> Result<Value> {
        let size = at.end.checked_sub(at.start).context("工具定位范围无效")?;
        ensure!(
            size > 0 && at.end <= committed_end,
            "工具定位超出已提交范围"
        );
        let mut bytes = Vec::new();
        match self {
            Self::Plain(file) => {
                file.seek(SeekFrom::Start(at.start))?;
                file.take(size).read_to_end(&mut bytes)?;
            }
            Self::Compressed(reader, offset) => {
                ensure!(at.start >= *offset, "归档工具定位顺序无效");
                let skip = at.start - *offset;
                ensure!(
                    io::copy(&mut reader.take(skip), &mut io::sink())? == skip,
                    "归档工具位置缺失"
                );
                reader.take(size).read_to_end(&mut bytes)?;
                *offset = at.end;
            }
        }
        ensure!(bytes.len() as u64 == size, "工具原会话位置缺失");
        let row: Value = serde_json::from_slice(&bytes)?;
        let timestamp = row["timestamp"]
            .as_str()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp_millis());
        let mut item = row;
        let payload = item["payload"].take();
        let is_kind = match payload["type"].as_str() {
            Some("function_call" | "custom_tool_call" | "local_shell_call") => !output,
            Some(
                "function_call_output" | "custom_tool_call_output" | "local_shell_call_output",
            ) => output,
            _ => false,
        };
        ensure!(
            item["type"] == "response_item"
                && timestamp == Some(at.at)
                && is_kind
                && payload["call_id"]
                    .as_str()
                    .or_else(|| payload["id"].as_str())
                    == Some(id),
            "工具定位记录身份已变化"
        );
        Ok(payload)
    }
}

fn activity_summary(
    db: &Connection,
    query: &ExecutionQuery,
    from: i64,
    until: i64,
    scope: Option<&str>,
    list_from: i64,
    complete: bool,
) -> Result<Value> {
    let span = query.end_at - query.start_at;
    let bucket = span / 64 + i64::from(span % 64 != 0);
    let calls = CALLS.as_str();
    let sql = format!("{calls},
        activity AS (
            SELECT at, kind, j.value AS name, '' AS namespace FROM filtered JOIN json_each(skills) j
            WHERE kind = 'skill'
            UNION ALL
            SELECT at, kind, name, namespace FROM filtered WHERE kind = 'mcp'
        ), groups AS (
            SELECT kind, name, namespace, count(*) AS count FROM activity GROUP BY kind, name, namespace
        ), top AS (
            SELECT * FROM groups ORDER BY count DESC, kind, name, namespace LIMIT 100
        )
        SELECT t.kind, t.name, t.namespace, t.count, (a.at - ?7) / ?8 AS bucket,
            count(*), min(a.at), (SELECT count(*) FROM groups)
        FROM top t JOIN activity a USING (kind,name,namespace)
        GROUP BY t.kind,t.name,t.namespace,bucket
        ORDER BY t.count DESC,t.kind,t.name,t.namespace,bucket");
    let mut statement = db.prepare(&sql)?;
    let mut rows = statement.query(params![
        from,
        until,
        scope,
        list_from,
        query.kind,
        query.search,
        query.start_at,
        bucket,
    ])?;
    let mut groups: Vec<Value> = Vec::new();
    let mut group_count = 0u64;
    let mut previous = None;
    while let Some(row) = rows.next()? {
        let key = (
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        );
        if previous.as_ref() != Some(&key) {
            groups.push(json!({
                "kind":key.0,"name":key.1,"namespace":key.2,"count":row.get::<_,u64>(3)?,
                "timeline":[]
            }));
            previous = Some(key);
        }
        groups.last_mut().unwrap()["timeline"]
            .as_array_mut()
            .unwrap()
            .push(json!([
                row.get::<_, u64>(4)?,
                row.get::<_, u64>(5)?,
                row.get::<_, i64>(6)?
            ]));
        group_count = row.get(7)?;
    }
    Ok(json!({
        "groups":groups,"groupCount":group_count,"startAt":query.start_at,
        "bucketWidthMs":bucket,"bucketCount":64,"complete":complete
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "00000000-0000-4000-8000-000000000001";

    #[test]
    fn pressure_evicts_only_oldest_tool_summaries_and_persists_the_gap() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("index.sqlite");
        let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
        let cutoff = Local::now().date_naive() - chrono::Days::new(59);
        let oldest = cutoff - chrono::Days::new(2);
        let second = cutoff.pred_opt().unwrap();
        store
            .db
            .execute(
                "UPDATE token_revision SET revision=1,precise_from=?1",
                [cutoff.to_string()],
            )
            .unwrap();
        store
            .db
            .execute("INSERT INTO token_sessions VALUES(1,?1)", [SESSION])
            .unwrap();
        for (day, count) in [(oldest, 2), (second, 3), (cutoff, 5)] {
            store
                .db
                .execute(
                    "INSERT INTO execution_daily VALUES(?1,1,?2,0,0,?2,0)",
                    params![day.to_string(), count],
                )
                .unwrap();
        }
        store
            .db
            .execute_batch(
                "CREATE TRIGGER fail_eviction BEFORE INSERT ON execution_evicted
             BEGIN SELECT RAISE(ABORT,'fixture'); END;",
            )
            .unwrap();
        assert!(store.evict_execution_days(cutoff, 1).is_err());
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM execution_daily", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            3
        );
        store
            .db
            .execute_batch("DROP TRIGGER fail_eviction")
            .unwrap();
        assert_eq!(store.evict_execution_days(cutoff, 1).unwrap(), 1);
        drop(store);

        let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
        let read = |store: &mut Store, start| {
            let query = ExecutionQuery::parse(&json!({
                "startAt": local_midnight(start).unwrap(),
                "endAt": local_midnight(cutoff).unwrap(),
            }))
            .unwrap();
            Store::read_execution_page(
                &mut store.db,
                &query,
                Local::now().timestamp_millis(),
                false,
                None,
            )
            .unwrap()
        };
        let pruned = read(&mut store, oldest);
        assert_eq!(pruned["revision"], 2);
        assert_eq!(pruned["coverage"]["reason"], "storage-pruned");
        assert_eq!(pruned["summary"]["total"], 3);
        assert_eq!(pruned["complete"], false);
        assert_eq!(
            read(&mut store, second)["coverage"]["reason"],
            "outside-retention"
        );
        assert_eq!(store.evict_execution_days(cutoff, 256).unwrap(), 1);
        assert_eq!(store.evict_execution_days(cutoff, 256).unwrap(), 0);
        assert_eq!(
            store
                .db
                .query_row("SELECT day FROM execution_daily", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            cutoff.to_string()
        );
    }

    #[test]
    fn expired_tools_compact_once_in_bounded_transactions_and_keep_full_day_counts() {
        let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        let now = Local::now();
        let cutoff = now.date_naive() - chrono::Days::new(59);
        let day = cutoff.pred_opt().unwrap();
        let start = local_midnight(day).unwrap();
        let end = local_midnight(cutoff).unwrap();
        store
            .db
            .execute(
                "UPDATE token_revision SET revision=1,precise_from=?1",
                [cutoff.to_string()],
            )
            .unwrap();
        store
            .db
            .execute("INSERT INTO token_sessions VALUES(1,?1)", [SESSION])
            .unwrap();
        for (source, pending) in [(1, false), (2, false), (3, true)] {
            store
                .db
                .execute(
                    "INSERT INTO token_sources VALUES(?1,?2,1,20,?3,0)",
                    params![
                        source,
                        vec![source as u8],
                        serde_json::to_vec(&json!({"resume_pending":pending})).unwrap()
                    ],
                )
                .unwrap();
        }
        for (source, position, id, at, name) in [
            (1, 1, "a", start + 1, "exec_command"),
            (2, 1, "a", start + 1, "exec_command"),
            (1, 2, "skill", start + 2, "exec_command"),
            (1, 3, "mcp", start + 3, "mcp__fixture"),
            (1, 4, "poll", start + 4, "write_stdin"),
            (3, 1, "unfinished", start + 5, "apply_patch"),
            (1, 6, "new", end + 1, "apply_patch"),
        ] {
            store
                .db
                .execute(
                    "INSERT INTO execution_observations VALUES(?1,1,?2,?3,?4,NULL,0,?5,'')",
                    params![source, position, at, id, name],
                )
                .unwrap();
        }
        store
            .db
            .execute(
                "INSERT INTO token_activity VALUES(1,1,'skill',?1,?2,?3)",
                params![
                    start + 2,
                    day.to_string(),
                    br#"{"call":{"arguments":{"skillNames":["alpha"]}}}"#.as_slice()
                ],
            )
            .unwrap();
        store
            .db
            .execute_batch(
                "CREATE TRIGGER reject_execution_summary BEFORE INSERT ON execution_daily
             BEGIN SELECT RAISE(ABORT, 'synthetic summary failure'); END;",
            )
            .unwrap();
        let mut cursor = None;
        let failure = store
            .compact_executions(cutoff, &mut cursor, 2)
            .unwrap_err();
        assert!(failure.to_string().contains("synthetic summary failure"));
        assert!(cursor.is_none());
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM execution_observations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            7
        );
        store
            .db
            .execute_batch("DROP TRIGGER reject_execution_summary")
            .unwrap();
        let mut removed = 0;
        for _ in 0..10 {
            let count = store.compact_executions(cutoff, &mut cursor, 2).unwrap();
            assert!(count <= 2);
            removed += count;
        }
        assert_eq!(removed, 5);
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM execution_observations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        let read = |store: &mut Store, from, to| {
            Store::read_execution_page(
                &mut store.db,
                &ExecutionQuery::parse(&json!({"startAt":from,"endAt":to})).unwrap(),
                now.timestamp_millis(),
                false,
                None,
            )
            .unwrap()
        };
        let old = read(&mut store, start, end);
        assert_eq!(old["summary"]["total"], 3);
        assert_eq!(old["dailySummaries"][0]["skill"], 1);
        assert_eq!(old["dailySummaries"][0]["mcp"], 1);
        assert_eq!(old["dailySummaries"][0]["tool"], 1);
        assert_eq!(old["coverage"]["precision"], "day");
        assert_eq!(old["items"], json!([]));
        assert!(read(&mut store, start + 1, end)["summary"]["total"].is_null());
        let mixed = read(&mut store, start, end + 100);
        assert_eq!(mixed["summary"]["total"], 4);
        assert_eq!(mixed["items"], json!([]));
    }

    #[test]
    fn pages_and_search_use_committed_identity_without_changing_totals() {
        let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        let now = Local::now();
        let at = now.timestamp_millis() - 600_000;
        let db = &mut store.db;
        db.execute_batch(
            "INSERT INTO token_sources VALUES(1,x'01',1,20,x'00',0);
             INSERT INTO token_sources VALUES(2,x'02',1,20,x'00',0);
             INSERT INTO token_coverage VALUES(1,1,0,0,0);",
        )
        .unwrap();
        db.execute(
            "UPDATE token_revision SET revision=1,precise_from=?1",
            [(now.date_naive() - chrono::Days::new(2)).to_string()],
        )
        .unwrap();
        db.execute("INSERT INTO token_sessions VALUES(1,?1)", [SESSION])
            .unwrap();
        let id = "a".repeat(1200);
        let name = format!("exec_command_{}", "a".repeat(750));
        let search_term = "A".repeat(750);
        for (source, position, id, at, name) in [
            (1, 1, id.as_str(), at, name.as_str()),
            (2, 1, id.as_str(), at, name.as_str()),
            (1, 2, "b", at, "apply_patch"),
            (1, 3, "c", at - 1, "mcp__fixture"),
            (1, 4, "d", at - 3, "functions.write_stdin"),
            (1, 5, "bad", at, "apply_patch"),
            (2, 5, "bad", at + 1000, "apply_patch"),
        ] {
            db.execute(
                "INSERT INTO execution_observations VALUES(?1,1,?2,?3,?4,NULL,0,?5,'')",
                params![source, position, at, id, name],
            )
            .unwrap();
        }
        let request = json!({"startAt":at - 10,"endAt":at + 1,"limit":2});
        let read = |db: &mut Connection, request: &Value| {
            Store::read_execution_page(
                db,
                &ExecutionQuery::parse(request).unwrap(),
                now.timestamp_millis(),
                false,
                None,
            )
            .unwrap()
        };
        let page = read(db, &request);
        assert_eq!(page["summary"]["total"], 3);
        assert_eq!(page["summary"]["tool"], 2);
        assert_eq!(page["summary"]["mcp"], 1);
        assert_eq!(page["summary"]["conflicts"], 1);
        assert_eq!(page["status"], "partial");
        assert_eq!(page["items"][0]["id"], "b");
        assert_eq!(page["items"][1]["id"], id);
        assert!(page["nextCursor"].as_str().unwrap().len() > 1024);
        assert!(DetailQuery::parse(&json!({"sessionId":SESSION,"id":id,"revision":1})).is_ok());
        let mut later = request.clone();
        later["cursor"] = page["nextCursor"].clone();
        later["revision"] = page["revision"].clone();
        let tail = read(db, &later);
        assert_eq!(tail["items"][0]["id"], "c");
        assert!(tail["nextCursor"].is_null());
        let mut filtered = request;
        filtered["search"] = json!("APPLY_PATCH");
        let search = read(db, &filtered);
        assert_eq!(search["summary"]["total"], 3);
        assert_eq!(search["summary"]["matched"], 1);
        assert_eq!(search["items"][0]["id"], "b");
        filtered["search"] = json!(search_term);
        let search = read(db, &filtered);
        assert_eq!(search["summary"]["matched"], 1);
        assert_eq!(search["items"][0]["id"], id);
        db.execute("UPDATE token_revision SET revision=2", [])
            .unwrap();
        assert_eq!(read(db, &later)["status"], "stale");
    }

    #[test]
    fn seven_day_metadata_expiry_preserves_totals_activity_and_late_copy_identity() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("usage.sqlite");
        let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
        let today = Local::now().date_naive();
        let now = local_midnight(today).unwrap();
        let edge = now - LIST_MS;
        store
            .db
            .execute_batch(
                "INSERT INTO token_sources VALUES(1,x'01',1,20,x'00',0);
             INSERT INTO token_sources VALUES(2,x'02',1,20,x'00',0);
             INSERT INTO token_coverage VALUES(1,1,0,0,0);",
            )
            .unwrap();
        store
            .db
            .execute(
                "UPDATE token_revision SET revision=1,precise_from=?1",
                [(today - chrono::Days::new(59)).to_string()],
            )
            .unwrap();
        store
            .db
            .execute("INSERT INTO token_sessions VALUES(1,?1)", [SESSION])
            .unwrap();
        for (source, position, id, at, name, namespace) in [
            (1, 1, "expired", edge, "exec_command", "functions"),
            (1, 2, "listed", edge + 1, "apply_patch", "functions"),
            (1, 3, "mcp", edge - 20, "query", "mcp__example"),
            (1, 4, "skill", edge - 10, "exec_command", "functions"),
            (1, 5, "poll", edge - 9, "functions.write_stdin", ""),
            (
                2,
                1,
                "different-time",
                edge - 60,
                "exec_command",
                "functions",
            ),
            (
                1,
                6,
                "different-time",
                edge - 59,
                "exec_command",
                "functions",
            ),
        ] {
            store
                .db
                .execute(
                    "INSERT INTO execution_observations VALUES(?1,1,?2,?3,?4,NULL,0,?5,?6)",
                    params![source, position, at, id, name, namespace],
                )
                .unwrap();
        }
        store
            .db
            .execute(
                "INSERT INTO token_activity VALUES(1,1,'skill',?1,?2,?3)",
                params![
                    edge - 10,
                    (today - chrono::Days::new(8)).to_string(),
                    br#"{"call":{"arguments":{"skillNames":["alpha","beta"]}}}"#.as_slice()
                ],
            )
            .unwrap();
        let request = json!({
            "startAt": edge - 120, "endAt": edge + 120,
            "includeActivitySummary": true
        });
        let read = |store: &mut Store, request: &Value| {
            Store::read_execution_page(
                &mut store.db,
                &ExecutionQuery::parse(request).unwrap(),
                now,
                false,
                None,
            )
            .unwrap()
        };
        let before = read(&mut store, &request);
        assert_eq!(before["summary"]["total"], 4);
        assert_eq!(before["summary"]["tool"], 2);
        assert_eq!(before["summary"]["conflicts"], 1);
        assert_eq!(before["summary"]["matched"], 3);
        assert_eq!(before["items"][0]["id"], "listed");
        assert_eq!(before["items"][1]["name"], "alpha");
        assert_eq!(before["items"][2]["namespace"], "mcp__example");
        assert_eq!(
            before["activitySummary"]["groups"]
                .as_array()
                .unwrap()
                .len(),
            3
        );

        store.compact_execution_metadata(now, 1).unwrap();
        let read_name = |store: &Store, id| {
            store
                .db
                .query_row(
                    "SELECT name FROM execution_observations WHERE source_id=1 AND call_id=?1",
                    [id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap()
        };
        assert_eq!(
            read_name(&store, "expired").as_deref(),
            Some("exec_command")
        );
        store.compact_execution_metadata(now, 256).unwrap();
        drop(store);
        let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
        assert_eq!(read_name(&store, "expired"), None);
        assert_eq!(read_name(&store, "listed").as_deref(), Some("apply_patch"));
        assert_eq!(read_name(&store, "mcp").as_deref(), Some("query"));
        assert_eq!(
            read_name(&store, "poll").as_deref(),
            Some("functions.write_stdin")
        );
        assert_eq!(read(&mut store, &request), before);
        store.db.execute(
            "INSERT INTO execution_observations VALUES(2,1,2,?1,'expired',NULL,0,'exec_command','functions')",
            [edge],
        ).unwrap();
        assert_eq!(read(&mut store, &request), before);
        let mut filtered = request;
        filtered["search"] = json!("exec_command");
        let result = read(&mut store, &filtered);
        assert_eq!(result["summary"]["total"], 4);
        assert_eq!(result["summary"]["matched"], 0);
    }

    #[test]
    fn skill_summary_splits_names_and_preserves_bucket_markers() {
        let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        let now = Local::now();
        let at = now.timestamp_millis() - 60_000;
        let db = &mut store.db;
        db.execute_batch(
            "INSERT INTO token_sources VALUES(1,x'01',1,20,x'00',0);
             INSERT INTO token_coverage VALUES(1,1,0,0,0);",
        )
        .unwrap();
        db.execute(
            "UPDATE token_revision SET revision=1,precise_from=?1",
            [(now.date_naive() - chrono::Days::new(2)).to_string()],
        )
        .unwrap();
        db.execute("INSERT INTO token_sessions VALUES(1,?1)", [SESSION])
            .unwrap();
        db.execute(
            "INSERT INTO execution_observations VALUES(1,1,1,?1,'multi',NULL,0,'exec_command','')",
            [at],
        )
        .unwrap();
        db.execute(
            "INSERT INTO token_activity VALUES(1,1,'multi',?1,?2,?3)",
            params![
                at,
                now.date_naive().to_string(),
                br#"{"call":{"arguments":{"skillNames":["alpha","beta"]}}}"#.as_slice()
            ],
        )
        .unwrap();
        let query = ExecutionQuery::parse(&json!({
            "startAt":at - 6000,"endAt":at + 4000,"kind":"skill","includeActivitySummary":true,
        }))
        .unwrap();
        let page =
            Store::read_execution_page(db, &query, now.timestamp_millis(), false, None).unwrap();
        let groups = page["activitySummary"]["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["name"], "alpha");
        assert_eq!(groups[1]["name"], "beta");
        assert_eq!(groups[0]["timeline"][0][2], at);
    }

    #[test]
    fn snapshot_upgrade_does_not_certify_unobserved_tool_history() {
        let store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        store
            .db
            .execute_batch(
                "INSERT INTO token_sources VALUES(1,x'01',1,20,x'00',0);
             INSERT INTO token_coverage VALUES(1,1,0,0,0);
             DROP TABLE execution_coverage_start;",
            )
            .unwrap();
        let mut store = Store::open(store.db).unwrap();
        let now = Local::now();
        store
            .db
            .execute(
                "UPDATE token_revision SET revision=1,precise_from=?1",
                [(now.date_naive() - chrono::Days::new(1)).to_string()],
            )
            .unwrap();
        let query = ExecutionQuery::parse(&json!({
            "startAt":now.timestamp_millis() - 60_000,"endAt":now.timestamp_millis(),
        }))
        .unwrap();
        let page =
            Store::read_execution_page(&mut store.db, &query, now.timestamp_millis(), false, None)
                .unwrap();
        assert_eq!(page["status"], "partial");
        assert_eq!(page["coverage"]["reason"], "before-tool-capture");
        assert!(page["summary"]["total"].is_null());
    }
}
