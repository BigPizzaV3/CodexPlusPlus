//! 工具索引只读接口；范围、作用域和版本在同一个 SQLite 快照中确定。
use super::execution_detail;
use super::execution_index::{self, DetailLookup, ExecutionQuery, PageLookup};
use anyhow::{Context, ensure};
use chrono::{DateTime, Local, TimeZone};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::time::Duration;

pub(super) fn open_index(home: &Path, settings: &Path) -> anyhow::Result<Connection> {
    let path = super::provider_cache::storage_dir(home, settings).join("usage-index.sqlite");
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(100))?;
    Ok(db)
}

pub(super) fn scope_sessions(
    db: &Connection,
    requested: Option<&str>,
) -> anyhow::Result<Option<Vec<String>>> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    let mut known_sessions = BTreeSet::new();
    {
        let mut statement = db.prepare(
            "SELECT DISTINCT scope_session FROM usage_sources
             WHERE scope_indexed = 1 AND scope_valid = 1 AND scope_session != ''",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            known_sessions.insert(row?);
        }
    }
    let mut aliases = BTreeMap::<String, BTreeSet<String>>::new();
    {
        let mut statement = db.prepare(
            "SELECT DISTINCT a.alias, a.session
             FROM usage_scope_aliases a JOIN usage_sources s ON s.source = a.source
             WHERE s.scope_indexed = 1 AND s.scope_valid = 1",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        for row in rows {
            let (alias, session) = row?;
            aliases.entry(alias).or_default().insert(session);
        }
    }
    let canonical = |id: &str| -> Option<String> {
        if known_sessions.contains(id) {
            return Some(id.to_owned());
        }
        match aliases.get(id) {
            Some(values) if values.len() == 1 => values.first().cloned(),
            Some(_) => None,
            None => Some(id.to_owned()),
        }
    };
    let Some(root) = canonical(requested) else {
        return Ok(Some(Vec::new()));
    };
    if !known_sessions.contains(&root) {
        return Ok(Some(Vec::new()));
    }
    let mut candidates = BTreeMap::<String, BTreeSet<Option<String>>>::new();
    {
        let mut statement = db.prepare(
            "SELECT DISTINCT p.session, p.parent
             FROM usage_scope_parents p JOIN usage_sources s ON s.source = p.source
             WHERE s.scope_indexed = 1 AND s.scope_valid = 1",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (child, parent) = row?;
            let parent = if parent.is_empty() { None } else { canonical(&parent) };
            candidates.entry(child).or_default().insert(parent);
        }
    }
    let parents: BTreeMap<_, _> = candidates
        .into_iter()
        .filter_map(|(child, parents)| {
            (parents.len() == 1)
                .then(|| parents.into_iter().next().flatten().map(|parent| (child, parent)))
                .flatten()
        })
        .collect();
    let mut children = BTreeMap::<&str, Vec<&str>>::new();
    for (child, parent) in &parents {
        let mut seen = HashSet::new();
        let mut node = child.as_str();
        let mut valid = true;
        while let Some(next) = parents.get(node) {
            if !seen.insert(node) {
                valid = false;
                break;
            }
            node = next;
        }
        if valid {
            children.entry(parent).or_default().push(child);
        }
    }
    let mut sessions = BTreeSet::from([root]);
    let mut pending = sessions.iter().cloned().collect::<Vec<_>>();
    while let Some(parent) = pending.pop() {
        for child in children.get(parent.as_str()).into_iter().flatten() {
            if sessions.insert((*child).to_owned()) {
                pending.push((*child).to_owned());
            }
        }
    }
    Ok(Some(sessions.into_iter().collect()))
}

struct ParsedExecutionRequest<'a> {
    start_ms: i64,
    end_ms: i64,
    scope: Option<String>,
    limit: usize,
    revision: Option<i64>,
    cursor: Option<&'a str>,
    search: &'a str,
    kind: Option<&'a str>,
    include_activity: bool,
}

fn parse_execution_request(request: &Value) -> anyhow::Result<ParsedExecutionRequest<'_>> {
    let start_ms = request["startAt"].as_i64().context("缺少有效的开始时间")?;
    let end_ms = request["endAt"].as_i64().context("缺少有效的结束时间")?;
    ensure!(start_ms < end_ms, "工具时间范围无效");
    DateTime::from_timestamp_millis(start_ms).context("工具开始时间超出范围")?;
    DateTime::from_timestamp_millis(end_ms - 1).context("工具结束时间超出范围")?;
    let scope = match request.get("scopeSessionId") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .and_then(super::super::normalize_session_id)
                .context("会话范围 ID 无效")?,
        ),
    };
    let limit = match request.get("limit") {
        None => 50,
        Some(value) => usize::try_from(value.as_u64().context("工具分页大小无效")?)?,
    };
    ensure!((1..=200).contains(&limit), "工具分页大小无效");
    let revision = match request.get("revision") {
        None => None,
        Some(value) => Some(value.as_i64().filter(|revision| *revision >= 0).context("工具版本无效")?),
    };
    let cursor = request.get("cursor").map(|value| value.as_str().context("工具游标无效")).transpose()?;
    let search = request.get("search").map(|value| value.as_str().context("工具搜索无效")).transpose()?.unwrap_or("");
    let kind = request.get("kind").map(|value| value.as_str().context("工具类别无效")).transpose()?;
    let include_activity = request.get("includeActivitySummary")
        .map(|value| value.as_bool().context("工具活动汇总选项无效")).transpose()?.unwrap_or(false);
    ensure!(search.len() <= 512, "工具搜索过长");
    ensure!(kind.is_none_or(|kind| matches!(kind, "skill" | "mcp" | "tool")), "工具类别无效");
    if let Some(cursor) = cursor {
        ensure!(revision.is_some(), "工具游标需要版本");
        execution_index::decode_cursor(cursor)?;
    }
    Ok(ParsedExecutionRequest {
        start_ms, end_ms, scope, limit, revision, cursor, search, kind, include_activity,
    })
}

#[cfg(test)]
fn query_page(db: &mut Connection, request: &Value, now_ms: i64) -> anyhow::Result<Value> {
    query_parsed_page(db, &parse_execution_request(request)?, now_ms)
}

fn query_parsed_page(
    db: &mut Connection,
    request: &ParsedExecutionRequest<'_>,
    now_ms: i64,
) -> anyhow::Result<Value> {
    let ParsedExecutionRequest {
        start_ms, end_ms, scope, limit, revision, cursor, search, kind, include_activity,
    } = request;
    let (start_ms, end_ms) = (*start_ms, *end_ms);
    let tx = db.transaction()?;
    let sessions = scope_sessions(&tx, scope.as_deref())?;
    let options = ExecutionQuery {
            start_ms,
            end_ms,
            sessions: sessions.as_deref(),
            limit: *limit,
            cursor: *cursor,
            revision: *revision,
            search,
            kind: *kind,
            now_ms,
        };
    let result = execution_index::query(&tx, &options)?;
    let value = match result {
        PageLookup::Stale => json!({"status":"stale","complete":false}),
        PageLookup::Ready(page) => {
            let first = DateTime::from_timestamp_millis(start_ms)
                .context("工具开始时间超出范围")?
                .with_timezone(&Local)
                .date_naive();
            let last = DateTime::from_timestamp_millis(end_ms - 1)
                .context("工具结束时间超出范围")?
                .with_timezone(&Local)
                .date_naive();
            let after_last = last.succ_opt().context("工具结束日期超出范围")?;
            let daily =
                execution_index::daily_summary(&tx, first, after_last, sessions.as_deref())?;
            let mut daily_count = 0u64;
            let mut partial_day = false;
            for row in &daily {
                let day = chrono::NaiveDate::parse_from_str(&row.day, "%Y-%m-%d")?;
                let day_start = Local
                    .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
                    .earliest()
                    .context("工具汇总日期没有本地零点")?
                    .timestamp_millis();
                let next_day = day.succ_opt().context("工具汇总日期超出范围")?;
                let day_end = Local
                    .from_local_datetime(&next_day.and_hms_opt(0, 0, 0).unwrap())
                    .earliest()
                    .context("工具汇总日期没有结束零点")?
                    .timestamp_millis();
                partial_day |= start_ms > day_start || end_ms < day_end;
                daily_count = daily_count
                    .checked_add(row.count)
                    .context("工具汇总次数超出范围")?;
            }
            let total = (!partial_day)
                .then(|| page.total.checked_add(daily_count).context("工具总次数超出范围"))
                .transpose()?;
            let classification_complete = !partial_day && page.classified == page.total
                && daily.iter().all(|row| row.classification_complete);
            let skill = page.skill + daily.iter().map(|row| row.skill).sum::<u64>();
            let mcp = page.mcp + daily.iter().map(|row| row.mcp).sum::<u64>();
            let tool = page.tool + daily.iter().map(|row| row.tool).sum::<u64>();
            let matched = if search.is_empty() && kind.is_none() {
                (!partial_day)
                    .then(|| {
                        page.matched
                            .checked_add(daily_count)
                            .context("工具匹配次数超出范围")
                    })
                    .transpose()?
            } else if search.is_empty() {
                if *kind == Some("tool") {
                    Some(page.matched)
                } else {
                    classification_complete.then_some(match *kind {
                        Some("skill") => skill,
                        Some("mcp") => mcp,
                        _ => unreachable!("execution query validated kind"),
                    })
                }
            } else {
                Some(page.matched)
            };
            let activity = if *include_activity && search.is_empty() {
                let mut activity = execution_index::activity_summary(&tx, &options)?;
                activity["complete"] = json!(classification_complete && daily.is_empty());
                Some(activity)
            } else { None };
            let precision = if daily.is_empty() { "exact" } else { "day" };
            json!({
                "status":"partial",
                "complete":false,
                "revision":page.revision,
                "items":page.items,
                "nextCursor":page.next_cursor,
                "dailySummaries":daily,
                "summary":{
                    "total":total,
                    "matched":matched,
                    "conflicts":page.conflicts,
                    "recentSkill":page.recent_skill,
                    "recentMcp":page.recent_mcp,
                    "recentTool":page.recent_tool,
                    "skill":(!partial_day).then_some(skill),
                    "mcp":(!partial_day).then_some(mcp),
                    "tool":(!partial_day).then_some(tool),
                    "classificationComplete":classification_complete
                },
                "coverage":{"complete":false,"precision":precision,
                    "reason":"discovery-unconfirmed"},
                "searchApplied":page.search_applied,
                "activitySummary":activity
            })
        }
    };
    tx.commit()?;
    Ok(value)
}

fn query_detail(
    db: &mut Connection,
    home: &Path,
    request: &Value,
    now_ms: impl Fn() -> i64,
) -> anyhow::Result<Value> {
    let session = request["sessionId"]
        .as_str()
        .and_then(super::super::normalize_session_id)
        .context("会话 ID 无效")?;
    let id = request["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 300)
        .context("工具调用 ID 无效")?;
    let revision = request["revision"]
        .as_i64()
        .filter(|revision| *revision >= 0)
        .context("工具版本无效")?;
    let tx = db.transaction()?;
    let lookup = execution_index::detail(&tx, &session, id, revision, now_ms())?;
    tx.commit()?;
    let value = match lookup {
        DetailLookup::Stale => json!({"status":"stale"}),
        DetailLookup::NotFound => json!({"status":"not_found","revision":revision}),
        DetailLookup::Ready(locators) => match execution_detail::read(home, id, &locators, now_ms)? {
            Some(detail) => {
                let mut value = json!({
                    "status":if detail.partial { "partial" } else { "ok" },
                    "revision":revision,
                    "detail":{"input":detail.input,"output":detail.output}
                });
                if let Some(duration_ms) = detail.duration_ms {
                    value["detail"]["durationMs"] = json!(duration_ms);
                }
                value
            },
            None => json!({"status":"not_found","revision":revision}),
        },
    };
    Ok(value)
}

pub(super) fn query_executions(home: &Path, settings: &Path, request: &Value) -> Value {
    if !super::history_enabled(settings) {
        return json!({"status":"unavailable","enabled":false,"complete":false});
    }
    let parsed = match parse_execution_request(request) {
        Ok(parsed) => parsed,
        Err(error) => return json!({"status":"unavailable","complete":false,"message":format!("{error:#}")}),
    };
    let scan = match super::committed::touch_for_executions(
        home, settings, parsed.scope.as_deref(),
    ) {
        Ok(scan) => scan,
        Err(error) => return json!({"status":"unavailable","complete":false,"message":format!("{error:#}")}),
    };
    match open_index(home, settings)
        .and_then(|mut db| query_parsed_page(&mut db, &parsed, chrono::Utc::now().timestamp_millis())) {
        Ok(mut value) => {
            value["scan"] = scan;
            value
        },
        Err(error) => {
            let message = scan.get("error").and_then(Value::as_str)
                .map(str::to_owned).unwrap_or_else(|| format!("{error:#}"));
            json!({"status":"unavailable","complete":false,"message":message,"scan":scan})
        },
    }
}

pub(super) fn query_execution_detail(home: &Path, settings: &Path, request: &Value) -> Value {
    if !super::history_enabled(settings) {
        return json!({"status":"unavailable","enabled":false});
    }
    match open_index(home, settings).and_then(|mut db| {
        query_detail(&mut db, home, request, || chrono::Utc::now().timestamp_millis())
    }) {
        Ok(value) => value,
        Err(error) => json!({"status":"unavailable","message":format!("{error:#}")}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::execution_index::{ExecutionObservation, ObservationKind};
    use rusqlite::params;
    use std::fs::{self, File};
    use std::path::PathBuf;

    const NOW: i64 = 1_800_000_000_000;
    const PARENT: &str = "00000000-0000-4000-8000-000000000001";
    const CHILD: &str = "00000000-0000-4000-8000-000000000002";
    const OTHER: &str = "00000000-0000-4000-8000-000000000003";
    const ALIAS: &str = "00000000-0000-4000-8000-000000000004";

    fn database() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE usage_meta(singleton INTEGER PRIMARY KEY, revision INTEGER NOT NULL);
             INSERT INTO usage_meta VALUES (1, 7);
             CREATE TABLE usage_sources(
                 source BLOB PRIMARY KEY, identity BLOB NOT NULL,
                 scope_session TEXT NOT NULL, scope_indexed INTEGER NOT NULL, scope_valid INTEGER NOT NULL,
                 execution_indexed INTEGER NOT NULL DEFAULT 1, execution_valid INTEGER NOT NULL DEFAULT 1
             );
             CREATE TABLE usage_scope_parents(source BLOB NOT NULL, session TEXT NOT NULL, parent TEXT NOT NULL);
             CREATE TABLE usage_scope_aliases(source BLOB NOT NULL, alias TEXT NOT NULL, session TEXT NOT NULL);
             INSERT INTO usage_sources(source, identity, scope_session, scope_indexed, scope_valid)
                 VALUES (X'61', X'01', '00000000-0000-4000-8000-000000000001', 1, 1),
                 (X'62', X'02', '00000000-0000-4000-8000-000000000002', 1, 1),
                 (X'63', X'03', '00000000-0000-4000-8000-000000000003', 1, 1);
             INSERT INTO usage_scope_parents VALUES
                 (X'61', '00000000-0000-4000-8000-000000000001', ''),
                 (X'62', '00000000-0000-4000-8000-000000000002', '00000000-0000-4000-8000-000000000004'),
                 (X'63', '00000000-0000-4000-8000-000000000003', '');
             INSERT INTO usage_scope_aliases VALUES
                 (X'61', '00000000-0000-4000-8000-000000000004', '00000000-0000-4000-8000-000000000001');",
        )
        .unwrap();
        let tx = db.transaction().unwrap();
        execution_index::initialize(&tx).unwrap();
        tx.commit().unwrap();
        db
    }

    fn add_call(db: &mut Connection, source: &[u8], session: &str, id: &str, kind: &str, at: i64) {
        let tx = db.transaction().unwrap();
        execution_index::apply(
            &tx,
            source,
            false,
            &[ExecutionObservation {
                session: session.into(),
                id: id.into(),
                timestamp_ms: at,
                start: 0,
                end: 5,
                kind: ObservationKind::Call {
                    name: format!("test.{kind}"),
                    namespace: "test".into(),
                    kind: kind.into(),
                    process_id: None,
                },
            }],
            NOW,
        )
        .unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn scoped_pages_use_children_keep_unfiltered_summary_and_reject_stale_revision() {
        let mut db = database();
        add_call(&mut db, b"a", PARENT, "one", "tool", NOW - 1);
        add_call(&mut db, b"b", CHILD, "two", "mcp", NOW - 2);
        add_call(&mut db, b"c", OTHER, "three", "skill", NOW - 3);
        let request = json!({"startAt":NOW-100,"endAt":NOW+1,"scopeSessionId":ALIAS,"limit":1});
        let page = query_page(&mut db, &request, NOW).unwrap();
        assert_eq!(page["status"], "partial");
        assert_eq!(page["coverage"]["complete"], false);
        assert_eq!(page["summary"]["total"], 2);
        assert_eq!(page["summary"]["recentTool"], 1);
        assert_eq!(page["summary"]["recentMcp"], 1);
        assert_eq!(page["summary"]["recentSkill"], 0);
        assert_eq!(page["items"][0]["id"], "one");
        let cursor = page["nextCursor"].as_str().unwrap();
        let second = query_page(
            &mut db,
            &json!({"startAt":NOW-100,"endAt":NOW+1,"scopeSessionId":ALIAS,
                "limit":1,"revision":7,"cursor":cursor}),
            NOW,
        )
        .unwrap();
        assert_eq!(second["items"][0]["id"], "two");
        let filtered = query_page(
            &mut db,
            &json!({"startAt":NOW-100,"endAt":NOW+1,"scopeSessionId":ALIAS,"kind":"mcp"}),
            NOW,
        )
        .unwrap();
        assert_eq!(filtered["summary"]["total"], 2);
        assert_eq!(filtered["summary"]["matched"], 1);
        assert_eq!(filtered["items"][0]["id"], "two");
        let stale = query_page(
            &mut db,
            &json!({"startAt":NOW-100,"endAt":NOW+1,"revision":6}),
            NOW,
        )
        .unwrap();
        assert_eq!(stale["status"], "stale");
    }

    #[test]
    fn conflicting_aliases_or_parents_do_not_claim_unrelated_children() {
        let mut db = database();
        add_call(&mut db, b"a", PARENT, "one", "tool", NOW - 1);
        add_call(&mut db, b"b", CHILD, "two", "tool", NOW - 2);
        db.execute(
            "INSERT INTO usage_scope_parents VALUES (X'63', ?1, ?2)",
            params![CHILD, OTHER],
        )
        .unwrap();
        let request = json!({"startAt":NOW-100,"endAt":NOW+1,"scopeSessionId":PARENT});
        let page = query_page(&mut db, &request, NOW).unwrap();
        assert_eq!(page["summary"]["total"], 1);
        db.execute(
            "INSERT INTO usage_scope_aliases VALUES (X'63', ?1, ?2)",
            params![ALIAS, OTHER],
        )
        .unwrap();
        let page = query_page(
            &mut db,
            &json!({"startAt":NOW-100,"endAt":NOW+1,"scopeSessionId":ALIAS}),
            NOW,
        )
        .unwrap();
        assert_eq!(page["summary"]["total"], 0);
        assert_eq!(page["coverage"]["complete"], false);
    }

    #[test]
    fn direct_session_identity_takes_precedence_over_a_conflicting_filename_alias() {
        let mut db = database();
        add_call(&mut db, b"a", PARENT, "one", "tool", NOW - 1);
        db.execute(
            "INSERT INTO usage_scope_aliases VALUES (X'63', ?1, ?2)",
            params![PARENT, OTHER],
        )
        .unwrap();
        let page = query_page(
            &mut db,
            &json!({"startAt":NOW-100,"endAt":NOW+1,"scopeSessionId":PARENT}),
            NOW,
        )
        .unwrap();
        assert_eq!(page["summary"]["total"], 1);
        assert_eq!(page["items"][0]["id"], "one");
    }

    #[test]
    fn compacted_daily_summaries_share_scope_and_do_not_become_execution_items() {
        let mut db = database();
        let day = DateTime::from_timestamp_millis(NOW - 70 * 24 * 60 * 60 * 1000)
            .unwrap()
            .with_timezone(&Local)
            .date_naive();
        let start = Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .timestamp_millis();
        let end = Local
            .from_local_datetime(&day.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .timestamp_millis();
        add_call(&mut db, b"a", PARENT, "retained", "tool", start + 3_000);
        for (session, count, partial) in [(PARENT, 3, false), (CHILD, 2, true), (OTHER, 9, false)] {
            db.execute(
                "INSERT INTO execution_daily(day, session, count, partial) VALUES (?1, ?2, ?3, ?4)",
                params![day.to_string(), session, count, partial],
            )
            .unwrap();
        }

        let request = json!({"startAt":start,"endAt":end,"scopeSessionId":ALIAS});
        let result = query_page(&mut db, &request, NOW).unwrap();
        assert_eq!(result["revision"], 7);
        assert_eq!(result["summary"]["total"], 6);
        assert_eq!(result["summary"]["matched"], 6);
        assert_eq!(result["coverage"]["precision"], "day");
        assert_eq!(
            result["dailySummaries"],
            json!([{"day":day.to_string(),"count":5,"partial":true,
                "skill":0,"mcp":0,"tool":0,"classificationComplete":false}])
        );
        assert_eq!(result["summary"]["classificationComplete"], false);
        assert_eq!(result["summary"]["tool"], 1);
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(result["items"][0]["id"], "retained");

        let partial = query_page(
            &mut db,
            &json!({"startAt":start+1,"endAt":end,"scopeSessionId":ALIAS}),
            NOW,
        )
        .unwrap();
        assert!(partial["summary"]["total"].is_null());
        assert!(partial["summary"]["matched"].is_null());
        assert!(partial["summary"]["skill"].is_null());
        assert!(partial["summary"]["mcp"].is_null());
        assert!(partial["summary"]["tool"].is_null());
        assert_eq!(partial["dailySummaries"][0]["count"], 5);
        assert_eq!(partial["items"].as_array().unwrap().len(), 1);

        let searched = query_page(
            &mut db,
            &json!({"startAt":start,"endAt":end,"scopeSessionId":ALIAS,"search":"retained"}),
            NOW,
        )
        .unwrap();
        assert_eq!(searched["summary"]["total"], 6);
        assert_eq!(searched["summary"]["matched"], 0);
        assert!(searched["items"].as_array().unwrap().is_empty());
    }

    fn detail_fixture() -> (tempfile::TempDir, PathBuf, Connection) {
        let root = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(root.path()).unwrap();
        fs::create_dir(home.join("sessions")).unwrap();
        let path = home.join("sessions").join("rollout-execution-api.jsonl");
        let timestamp = chrono::DateTime::from_timestamp_millis(NOW - 1_000)
            .unwrap()
            .to_rfc3339();
        let call = json!({"type":"response_item","timestamp":timestamp,"payload":{
            "type":"function_call","call_id":"call","name":"functions.exec_command",
            "arguments":"{\"cmd\":\"Get-Content C:\\\\work\\\\file.txt\"}"}});
        let output = json!({"type":"response_item","timestamp":timestamp,"payload":{
            "type":"function_call_output","call_id":"call",
            "output":"Chunk ID: test\nWall time: 1 seconds\nProcess exited with code 0\nFinal output:\nresult"}});
        let first = format!("{call}\n");
        let second = format!("{output}\n");
        fs::write(&path, format!("{first}{second}")).unwrap();
        let path = fs::canonicalize(path).unwrap();
        let source = super::super::persistent_index::encode_source(&path);
        let identity = super::super::persistent_index::encode_identity(
            codex_plus_core::codex_sqlite::opened_file_identity(&File::open(&path).unwrap()).unwrap(),
        );
        let mut db = database();
        db.execute(
            "INSERT INTO usage_sources(source, identity, scope_session, scope_indexed, scope_valid)
             VALUES (?1, ?2, ?3, 1, 1)",
            params![source, identity, PARENT],
        )
        .unwrap();
        let tx = db.transaction().unwrap();
        execution_index::apply(
            &tx,
            &source,
            false,
            &[
                ExecutionObservation {
                    session: PARENT.into(),
                    id: "call".into(),
                    timestamp_ms: NOW - 1_000,
                    start: 0,
                    end: first.len() as u64,
                    kind: ObservationKind::Call {
                        name: "functions.exec_command".into(),
                        namespace: "functions".into(),
                        kind: "tool".into(),
                        process_id: None,
                    },
                },
                ExecutionObservation {
                    session: PARENT.into(),
                    id: "call".into(),
                    timestamp_ms: NOW - 1_000,
                    start: first.len() as u64,
                    end: (first.len() + second.len()) as u64,
                    kind: ObservationKind::Output { process_id: None },
                },
            ],
            NOW,
        )
        .unwrap();
        tx.commit().unwrap();
        (root, home, db)
    }

    #[test]
    fn detail_reads_original_source_and_honors_revision_and_expiry() {
        let (_root, home, mut db) = detail_fixture();
        let request = json!({"sessionId":PARENT,"id":"call","revision":7});
        let detail = query_detail(&mut db, &home, &request, || NOW).unwrap();
        assert_eq!(detail["status"], "ok");
        assert_eq!(detail["detail"]["input"]["cmd"], "Get-Content C:\\work\\file.txt");
        assert_eq!(detail["detail"]["output"], json!(["Chunk ID: test\nWall time: 1 seconds\nProcess exited with code 0\nFinal output:\nresult"]));
        assert_eq!(detail["detail"]["durationMs"], 0);
        let stale = query_detail(
            &mut db,
            &home,
            &json!({"sessionId":PARENT,"id":"call","revision":6}),
            || NOW,
        )
        .unwrap();
        assert_eq!(stale["status"], "stale");
        let expired = query_detail(&mut db, &home, &request, || NOW + execution_index::DETAIL_RETENTION_MS - 1_000)
            .unwrap();
        assert_eq!(expired["status"], "not_found");
    }

    #[test]
    fn incomplete_source_returns_available_detail_as_partial() {
        let (_root, home, mut db) = detail_fixture();
        db.execute(
            "UPDATE usage_sources SET execution_valid = 0 WHERE scope_session = ?1 AND identity != X'01'",
            [PARENT],
        )
        .unwrap();
        let detail = query_detail(
            &mut db,
            &home,
            &json!({"sessionId":PARENT,"id":"call","revision":7}),
            || NOW,
        )
        .unwrap();
        assert_eq!(detail["status"], "partial");
        assert_eq!(detail["detail"]["input"]["cmd"], "Get-Content C:\\work\\file.txt");
        assert_eq!(detail["detail"]["output"], json!(["Chunk ID: test\nWall time: 1 seconds\nProcess exited with code 0\nFinal output:\nresult"]));
    }

    #[test]
    fn detail_recovers_previously_expired_positions_without_mutating_index() {
        let (_root, home, mut db) = detail_fixture();
        db.execute("DELETE FROM execution_observations WHERE is_output = 1", []).unwrap();
        db.execute("UPDATE execution_observations SET end = NULL WHERE is_output = 0", []).unwrap();
        let request = json!({"sessionId":PARENT,"id":"call","revision":7});
        let detail = query_detail(&mut db, &home, &request, || NOW).unwrap();
        assert_eq!(detail["status"], "ok");
        assert_eq!(detail["detail"]["input"]["cmd"], "Get-Content C:\\work\\file.txt");
        assert_eq!(detail["detail"]["output"].as_array().unwrap().len(), 1);
        assert_eq!(detail["detail"]["durationMs"], 0);
        let retained: (i64, i64) = db.query_row(
            "SELECT COUNT(*), COUNT(end) FROM execution_observations", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(retained, (1, 0));
    }

    #[test]
    fn detail_remains_available_during_pressure_within_seven_days() {
        let (_root, home, mut db) = detail_fixture();
        let now = NOW + 2 * 24 * 60 * 60 * 1000;
        let tx = db.transaction().unwrap();
        assert!(!execution_index::reclaim_oldest_details(&tx, now, None).unwrap().changed);
        tx.commit().unwrap();
        let detail = query_detail(
            &mut db,
            &home,
            &json!({"sessionId":PARENT,"id":"call","revision":7}),
            || now,
        )
        .unwrap();
        assert_eq!(detail["status"], "ok");
        assert_eq!(detail["detail"]["input"]["cmd"], "Get-Content C:\\work\\file.txt");
        assert_eq!(detail["detail"]["output"].as_array().unwrap().len(), 1);
        assert_eq!(detail["detail"]["durationMs"], 0);
    }

    #[test]
    fn old_skill_and_mcp_counts_and_activity_outlive_detail_retention() {
        let mut db = database();
        let old = NOW - 8 * 24 * 60 * 60 * 1000;
        add_call(&mut db, b"a", PARENT, "skill", "skill", old);
        add_call(&mut db, b"b", CHILD, "mcp", "mcp", old + 1);
        let result = query_page(&mut db, &json!({
            "startAt":old-1,"endAt":NOW+1,"includeActivitySummary":true,"scopeSessionId":PARENT
        }), NOW).unwrap();
        assert_eq!(result["summary"]["skill"], 1);
        assert_eq!(result["summary"]["mcp"], 1);
        assert_eq!(result["summary"]["recentSkill"], 0);
        assert_eq!(result["summary"]["recentMcp"], 0);
        assert_eq!(result["summary"]["classificationComplete"], true);
        assert_eq!(result["items"][0]["kind"], "mcp");
        assert_eq!(result["activitySummary"]["groups"].as_array().unwrap().len(), 2);
        assert_eq!(result["activitySummary"]["groups"][0]["timeline"], json!([[0,1,old + 1]]));
        assert_eq!(result["activitySummary"]["complete"], true);
        let filtered = query_page(&mut db, &json!({
            "startAt":old-1,"endAt":NOW+1,"kind":"skill"
        }), NOW).unwrap();
        assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["summary"]["matched"], 1);
        let searched = query_page(&mut db, &json!({
            "startAt":old-1,"endAt":NOW+1,"search":"test.mcp","includeActivitySummary":true
        }), NOW).unwrap();
        assert_eq!(searched["summary"]["matched"], 1);
        assert!(searched["activitySummary"].is_null());
        let detail = query_detail(&mut db, Path::new("unused"),
            &json!({"sessionId":PARENT,"id":"skill","revision":7}), || NOW).unwrap();
        assert_eq!(detail["status"], "not_found");
    }

    #[test]
    fn activity_timeline_uses_the_first_actual_call_in_each_bucket() {
        let mut db = database();
        let start = NOW - 3_600_000;
        add_call(&mut db, b"a", PARENT, "first", "skill", start + 2_000);
        add_call(&mut db, b"b", PARENT, "second", "skill", start + 5_000);
        add_call(&mut db, b"c", PARENT, "third", "skill", start + 61_000);
        let result = query_page(&mut db, &json!({
            "startAt":start,"endAt":NOW,"kind":"skill","includeActivitySummary":true
        }), NOW).unwrap();
        assert_eq!(result["summary"]["skill"], 3);
        assert_eq!(result["activitySummary"]["groups"][0]["timeline"],
            json!([[0,2,start + 2_000], [1,1,start + 61_000]]));
    }

    #[test]
    fn missing_classification_keeps_known_skill_and_mcp_counts_as_lower_bounds() {
        let mut db = database();
        let old = NOW - 3 * 24 * 60 * 60 * 1000;
        add_call(&mut db, b"a", PARENT, "skill", "skill", old);
        add_call(&mut db, b"b", CHILD, "mcp", "mcp", old + 1);
        add_call(&mut db, b"c", OTHER, "unknown", "tool", old + 2);
        db.execute(
            "UPDATE execution_observations SET name = NULL, namespace = NULL, call_kind = NULL
             WHERE source = X'63'",
            [],
        ).unwrap();
        let result = query_page(&mut db, &json!({
            "startAt":old-1,"endAt":NOW+1,"includeActivitySummary":true
        }), NOW).unwrap();
        assert_eq!(result["summary"]["total"], 3);
        assert_eq!(result["summary"]["skill"], 1);
        assert_eq!(result["summary"]["mcp"], 1);
        assert_eq!(result["summary"]["tool"], 0);
        assert_eq!(result["summary"]["classificationComplete"], false);
        assert_eq!(result["activitySummary"]["groups"].as_array().unwrap().len(), 2);
        assert_eq!(result["activitySummary"]["complete"], false);
    }
}
