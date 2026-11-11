use super::*;
use serde_json::json;

const NOW: i64 = 1_800_000_000_000;
const SESSION: &str = "00000000-0000-4000-8000-000000000001";
const OTHER_SESSION: &str = "00000000-0000-4000-8000-000000000002";

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE usage_meta(singleton INTEGER PRIMARY KEY, revision INTEGER NOT NULL);
         INSERT INTO usage_meta VALUES(1, 7);
         CREATE TABLE usage_sources(source BLOB PRIMARY KEY, identity BLOB NOT NULL, complete INTEGER NOT NULL,
             execution_indexed INTEGER NOT NULL DEFAULT 1, execution_valid INTEGER NOT NULL DEFAULT 1,
             scope_valid INTEGER NOT NULL DEFAULT 1);
         INSERT INTO usage_sources(source, identity, complete) VALUES(X'61', X'0102', 1), (X'62', X'0304', 1);",
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    initialize(&tx).unwrap();
    tx.commit().unwrap();
    db
}

fn call(
    session: &str,
    id: &str,
    timestamp_ms: i64,
    start: u64,
    name: &str,
) -> ExecutionObservation {
    ExecutionObservation {
        session: session.to_owned(),
        id: id.to_owned(),
        timestamp_ms,
        start,
        end: start + 5,
        kind: ObservationKind::Call {
            name: name.to_owned(),
            namespace: "functions".into(),
            kind: "tool".into(),
            process_id: None,
        },
    }
}

fn output(session: &str, id: &str, timestamp_ms: i64, start: u64) -> ExecutionObservation {
    ExecutionObservation {
        session: session.to_owned(),
        id: id.to_owned(),
        timestamp_ms,
        start,
        end: start + 5,
        kind: ObservationKind::Output { process_id: None },
    }
}

fn commit(db: &mut Connection, source: &[u8], replace: bool, items: &[ExecutionObservation]) {
    let tx = db.transaction().unwrap();
    apply(&tx, source, replace, items, NOW).unwrap();
    purge_details(&tx, NOW).unwrap();
    tx.commit().unwrap();
}

fn request<'a>(sessions: Option<&'a [String]>) -> ExecutionQuery<'a> {
    ExecutionQuery {
        start_ms: NOW - 2 * DETAIL_RETENTION_MS,
        end_ms: NOW + 1,
        sessions,
        limit: 2,
        cursor: None,
        revision: Some(7),
        search: "",
        kind: None,
        now_ms: NOW,
    }
}

fn page(result: PageLookup) -> ExecutionPage {
    match result {
        PageLookup::Ready(page) => page,
        PageLookup::Stale => panic!("unexpected stale page"),
    }
}

#[test]
fn pressure_recovery_keeps_locations_until_both_call_and_output_expire() {
    let mut db = database();
    let at = NOW - DETAIL_RETENTION_MS + 1_000;
    commit(&mut db, b"a", false, &[
        call(SESSION, "boundary", at, 0, "functions.exec_command"),
        output(SESSION, "boundary", at + 1, 10),
    ]);
    let tx = db.transaction().unwrap();
    assert!(!reclaim_oldest_details(&tx, NOW + 1_000, None).unwrap().changed);
    tx.commit().unwrap();
    assert_eq!(db.query_row(
        "SELECT COUNT(end) FROM execution_observations WHERE call_id = 'boundary'",
        [], |row| row.get::<_, i64>(0),
    ).unwrap(), 2);

    let tx = db.transaction().unwrap();
    assert!(reclaim_oldest_details(&tx, NOW + 1_001, None).unwrap().changed);
    tx.commit().unwrap();
    assert_eq!(db.query_row(
        "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'boundary' AND is_output = 1",
        [], |row| row.get::<_, i64>(0),
    ).unwrap(), 0);
}

#[test]
fn pressure_reclaims_only_older_synchronous_tool_locations() {
    let mut db = database();
    let old = NOW - 2 * 24 * 60 * 60 * 1000;
    let recent = NOW - 60_000;
    let mut skill = call(SESSION, "skill", old, 40, "skill");
    let mut mcp = call(SESSION, "mcp", old, 60, "mcp");
    let mut asynchronous = output(SESSION, "async", old + 1, 90);
    if let ObservationKind::Call { kind, .. } = &mut skill.kind {
        *kind = "skill".into();
    }
    if let ObservationKind::Call { kind, .. } = &mut mcp.kind {
        *kind = "mcp".into();
    }
    if let ObservationKind::Output { process_id } = &mut asynchronous.kind {
        *process_id = Some("42".into());
    }
    commit(
        &mut db,
        b"a",
        false,
        &[
            call(SESSION, "old", old, 0, "functions.exec_command"),
            output(SESSION, "old", old + 1, 10),
            call(SESSION, "recent", recent, 20, "functions.exec_command"),
            output(SESSION, "recent", recent + 1, 30),
            skill,
            output(SESSION, "skill", old + 1, 50),
            mcp,
            output(SESSION, "mcp", old + 1, 70),
            call(SESSION, "async", old, 80, "functions.exec_command"),
            asynchronous,
        ],
    );
    let before = page(query(&db, &request(None)).unwrap());
    let tx = db.transaction().unwrap();
    assert!(reclaim_oldest_details(&tx, NOW + 6 * 86_400_000, None).unwrap().changed);
    tx.commit().unwrap();
    let after = page(query(&db, &request(None)).unwrap());
    assert_eq!((after.total, after.skill, after.mcp, after.tool),
        (before.total, before.skill, before.mcp, before.tool));
    let state: Vec<(String, Option<i64>, i64)> = db
        .prepare(
            "SELECT call_id, end, is_output FROM execution_observations
             ORDER BY start",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(state.iter().filter(|row| row.0 == "old").count(), 1);
    assert_eq!(state[0], ("old".into(), None, 0));
    assert_eq!(state.iter().filter(|row| row.0 != "old").count(), 8);
    assert!(state.iter().filter(|row| row.0 != "old").all(|row| row.1.is_some()));
    let tx = db.transaction().unwrap();
    assert!(!reclaim_oldest_details(&tx, NOW + 6 * 86_400_000, None).unwrap().changed);
    tx.commit().unwrap();
}

#[test]
fn pressure_recovery_preserves_calls_with_too_many_output_segments() {
    let mut db = database();
    let old = NOW - 2 * 24 * 60 * 60 * 1000;
    let mut observations = vec![call(SESSION, "many", old, 0, "functions.exec_command")];
    observations.extend((0..PURGE_BATCH_ROWS).map(|index| {
        output(SESSION, "many", old + 1, 10 + index as u64 * 10)
    }));
    commit(&mut db, b"a", false, &observations);
    let tx = db.transaction().unwrap();
    assert!(!reclaim_oldest_details(&tx, NOW + 6 * 86_400_000, None).unwrap().changed);
    tx.commit().unwrap();
    let (calls, outputs, end): (i64, i64, Option<i64>) = db.query_row(
        "SELECT SUM(is_output = 0), SUM(is_output = 1),
                MAX(CASE WHEN is_output = 0 THEN end END)
         FROM execution_observations WHERE call_id = 'many'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!((calls, outputs, end), (1, PURGE_BATCH_ROWS as i64, Some(5)));
}

#[test]
fn pressure_recovery_preserves_calls_with_distant_output() {
    let mut db = database();
    let old = NOW - 2 * 24 * 60 * 60 * 1000;
    commit(&mut db, b"a", false, &[
        call(SESSION, "far", old, 0, "functions.exec_command"),
        output(SESSION, "far", old + 1, super::super::execution_detail::MAX_RECOVERY_ROWS as u64),
    ]);
    let tx = db.transaction().unwrap();
    assert!(!reclaim_oldest_details(&tx, NOW + 6 * 86_400_000, None).unwrap().changed);
    tx.commit().unwrap();
    let (outputs, end): (i64, Option<i64>) = db.query_row(
        "SELECT SUM(is_output = 1), MAX(CASE WHEN is_output = 0 THEN end END)
         FROM execution_observations WHERE call_id = 'far'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!((outputs, end), (1, Some(5)));
}

#[test]
fn pressure_recovery_preserves_compressed_details_beyond_read_budget() {
    let mut db = database();
    let old = NOW - 2 * 24 * 60 * 60 * 1000;
    let source = super::super::persistent_index::encode_source(
        std::path::Path::new("rollout-detail.jsonl.zst"),
    );
    db.execute(
        "INSERT INTO usage_sources(source, identity, complete) VALUES(?1, X'0102', 1)",
        [&source],
    ).unwrap();
    let start = super::super::execution_detail::MAX_DECOMPRESSED_BYTES - 3;
    commit(&mut db, &source, false, &[
        call(SESSION, "compressed", old, start - 10, "functions.exec_command"),
        output(SESSION, "compressed", old + 1, start),
    ]);
    let tx = db.transaction().unwrap();
    assert!(!reclaim_oldest_details(&tx, NOW + 6 * 86_400_000, None).unwrap().changed);
    tx.commit().unwrap();
    let retained: (i64, i64) = db.query_row(
        "SELECT COUNT(*), COUNT(end) FROM execution_observations WHERE call_id = 'compressed'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(retained, (2, 2));
}

#[test]
fn pressure_recovery_skips_duplicate_call_identities() {
    let mut db = database();
    let old = NOW - 2 * 24 * 60 * 60 * 1000;
    commit(&mut db, b"a", false, &[
        call(SESSION, "duplicate", old, 0, "functions.exec_command"),
        call(SESSION, "duplicate", old, 10, "functions.exec_command"),
        output(SESSION, "duplicate", old + 1, 20),
    ]);
    let tx = db.transaction().unwrap();
    assert!(!reclaim_oldest_details(&tx, NOW + 6 * 86_400_000, None).unwrap().changed);
    tx.commit().unwrap();
    let retained: (i64, i64) = db.query_row(
        "SELECT SUM(is_output = 1), COUNT(end) FROM execution_observations WHERE call_id = 'duplicate'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(retained, (1, 3));
}

#[test]
fn pressure_recovery_advances_past_duplicate_identities_in_bounded_batches() {
    let mut db = database();
    let old = NOW - 2 * 24 * 60 * 60 * 1000;
    let mut observations: Vec<_> = (0..PURGE_BATCH_ROWS + 44)
        .map(|index| call(SESSION, "duplicate", old, index as u64 * 10, "functions.exec_command"))
        .collect();
    observations.push(call(SESSION, "eligible", old + 1, 10_000, "functions.exec_command"));
    observations.push(output(SESSION, "eligible", old + 2, 10_010));
    commit(&mut db, b"a", false, &observations);

    let tx = db.transaction().unwrap();
    let first = reclaim_oldest_details(&tx, NOW + 6 * 86_400_000, None).unwrap();
    assert!(!first.changed);
    assert!(first.next.is_some());
    let second = reclaim_oldest_details(&tx, NOW + 6 * 86_400_000, first.next.as_ref()).unwrap();
    assert!(second.changed);
    assert!(second.next.is_none());
    tx.commit().unwrap();
    let retained: (i64, i64) = db.query_row(
        "SELECT COUNT(*), SUM(end IS NULL) FROM execution_observations WHERE call_id = 'duplicate'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(retained, ((PURGE_BATCH_ROWS + 44) as i64, 0));
}

#[test]
fn pressure_recovery_rolls_back_call_and_output_together() {
    let mut db = database();
    let old = NOW - 2 * 24 * 60 * 60 * 1000;
    commit(
        &mut db,
        b"a",
        false,
        &[
            call(SESSION, "old", old, 0, "functions.exec_command"),
            output(SESSION, "old", old + 1, 10),
        ],
    );
    {
        let tx = db.transaction().unwrap();
        assert!(reclaim_oldest_details(&tx, NOW + 6 * 86_400_000, None).unwrap().changed);
    }
    let (outputs, end): (i64, Option<i64>) = db.query_row(
        "SELECT SUM(is_output = 1), MAX(CASE WHEN is_output = 0 THEN end END)
         FROM execution_observations WHERE call_id = 'old'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!((outputs, end), (1, Some(5)));
}

#[test]
fn pagination_keeps_all_tool_times_above_the_old_in_memory_limit() {
    let mut db = database();
    let count = 12_031usize;
    let at = NOW - DETAIL_RETENTION_MS - 60_000;
    let calls = (0..count)
        .map(|index| {
            call(
                SESSION,
                &format!("call-{index:05}"),
                at + index as i64,
                index as u64 * 10,
                "functions.exec_command",
            )
        })
        .collect::<Vec<_>>();
    commit(&mut db, b"a", false, &calls);
    let mut cursor = None;
    let mut seen = std::collections::HashSet::new();
    loop {
        let mut query_options = request(None);
        query_options.limit = 200;
        query_options.cursor = cursor.as_deref();
        let result = page(query(&db, &query_options).unwrap());
        assert_eq!(result.total, count as u64);
        for item in result.items {
            assert_eq!(item.name.as_deref(), Some("functions.exec_command"));
            assert!(seen.insert(item.id));
        }
        cursor = result.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), count);
}

#[test]
fn parses_function_custom_and_mcp_calls_without_capturing_bodies() {
    let timestamp = DateTime::from_timestamp_millis(NOW).unwrap().to_rfc3339();
    for (kind, name) in [
        ("function_call", "functions.exec_command"),
        ("function_call", "mcp__server__search"),
        ("custom_tool_call", "functions.apply_patch"),
    ] {
        let line = json!({"type":"response_item","timestamp":timestamp,"payload":{
            "type":kind,"call_id":"call-1","name":name,"arguments":"private command"
        }});
        let bytes = serde_json::to_vec(&line).unwrap();
        let found = parse_line(&bytes, 10, 20, SESSION, false).unwrap().unwrap();
        assert_eq!(found.id, "call-1");
        assert_eq!(found.timestamp_ms, NOW);
        assert!(!format!("{found:?}").contains("private command"));
        assert!(parse_line(&bytes, 10, 20, SESSION, true).unwrap().is_none());
    }
    let local = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"local_shell_call","id":"local","action":{"command":["echo","hello"]}
    }});
    let found = parse_line(&serde_json::to_vec(&local).unwrap(), 0, 10, SESSION, false)
        .unwrap().unwrap();
    assert!(matches!(found.kind, ObservationKind::Call { ref name, ref kind, .. }
        if name == "local_shell" && kind == "tool"));
    let local_output = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"local_shell_call_output","call_id":"local","output":"hello"
    }});
    let found = parse_line(&serde_json::to_vec(&local_output).unwrap(), 10, 20, SESSION, false)
        .unwrap().unwrap();
    assert!(matches!(found.kind, ObservationKind::Output { .. }));
    let output_line = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call_output","call_id":"call-1","output":"private output"
    }});
    let found = parse_line(
        &serde_json::to_vec(&output_line).unwrap(),
        20,
        30,
        SESSION,
        false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(found.kind, ObservationKind::Output { process_id: None });
    assert!(!format!("{found:?}").contains("private output"));
    let invalid = json!({"type":"response_item","payload":{
        "type":"function_call","call_id":"call-1","name":"functions.exec_command"
    }});
    assert!(
        parse_line(
            &serde_json::to_vec(&invalid).unwrap(),
            0,
            10,
            SESSION,
            false
        )
        .is_err()
    );
    let invalid = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call","name":"functions.exec_command"
    }});
    assert!(
        parse_line(
            &serde_json::to_vec(&invalid).unwrap(),
            0,
            10,
            SESSION,
            false
        )
        .is_err()
    );
}

#[test]
fn classifies_skills_mcp_and_normal_commands_without_retaining_inputs() {
    let timestamp = DateTime::from_timestamp_millis(NOW).unwrap().to_rfc3339();
    for (name, arguments, expected) in [
        (
            "functions.exec_command",
            json!(json!({"cmd":"cat C:/skills/pdf/SKILL.md"}).to_string()),
            "skill",
        ),
        (
            "functions.exec_command",
            json!({"cmd":"cat C:/skills/pdf/SKILL.md"}),
            "skill",
        ),
        ("mcp__server__search", json!("{}"), "mcp"),
        (
            "functions.exec_command",
            json!(json!({"cmd":"echo hello"}).to_string()),
            "tool",
        ),
        ("functions.apply_patch", json!("private patch"), "tool"),
    ] {
        let line = json!({"type":"response_item","timestamp":timestamp,"payload":{
            "type":"function_call","call_id":"call","name":name,"arguments":arguments
        }});
        let found = parse_line(&serde_json::to_vec(&line).unwrap(), 0, 20, SESSION, false)
            .unwrap()
            .unwrap();
        let ObservationKind::Call { kind, name: display_name, .. } = found.kind else {
            panic!("expected call");
        };
        assert_eq!(kind, expected, "{name}");
        if expected == "skill" { assert_eq!(display_name, "pdf"); }
    }
}

#[test]
fn duplicate_sources_stable_pagination_search_and_revision() {
    let mut db = database();
    let one = call(SESSION, "one", NOW - 10, 0, "functions.exec_command");
    let two = call(SESSION, "two", NOW - 10, 10, "mcp__server__search");
    let three = call(
        OTHER_SESSION,
        "three",
        NOW - 20,
        20,
        "functions.apply_patch",
    );
    let old = call(
        SESSION,
        "old",
        NOW - DETAIL_RETENTION_MS,
        30,
        "private-old-name",
    );
    commit(&mut db, b"a", false, &[one.clone(), two, three, old]);
    commit(&mut db, b"b", false, &[one]);
    let first = page(query(&db, &request(None)).unwrap());
    assert_eq!(first.total, 4);
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.conflicts, 0);
    let cursor = first.next_cursor.unwrap();
    let mut next_request = request(None);
    next_request.cursor = Some(&cursor);
    let second = page(query(&db, &next_request).unwrap());
    assert_eq!(second.total, 4);
    assert_eq!(second.items.len(), 2);
    assert!(second.next_cursor.is_none());
    let all = first
        .items
        .into_iter()
        .chain(second.items)
        .collect::<Vec<_>>();
    assert_eq!(all.iter().filter(|item| item.id == "one").count(), 1);
    let expired = all.iter().find(|item| item.id == "old").unwrap();
    assert_eq!(expired.name.as_deref(), Some("private-old-name"));
    assert_eq!(expired.kind.as_deref(), Some("tool"));
    let serialized = serde_json::to_value(expired).unwrap();
    assert_eq!(serialized.as_object().unwrap().len(), 6);

    let scope = vec![OTHER_SESSION.to_owned()];
    let scoped = page(query(&db, &request(Some(&scope))).unwrap());
    assert_eq!(scoped.total, 1);
    assert_eq!(scoped.items[0].id, "three");

    let mut search = request(None);
    search.search = "SEARCH";
    let matches = page(query(&db, &search).unwrap());
    assert!(matches.search_applied);
    assert_eq!(matches.total, 4);
    assert_eq!(matches.matched, 1);
    assert_eq!(matches.items[0].id, "two");
    search.search = "";
    search.kind = Some("tool");
    let matches = page(query(&db, &search).unwrap());
    assert_eq!(matches.total, 4);
    assert_eq!(matches.matched, 3);
    assert_eq!(matches.recent_tool, 3);
    assert_eq!(matches.tool, 4);
    search.kind = None;
    search.search = "private-old-name";
    let matches = page(query(&db, &search).unwrap());
    assert_eq!(matches.total, 4);
    assert_eq!(matches.matched, 0);

    next_request.revision = Some(6);
    assert!(matches!(
        query(&db, &next_request).unwrap(),
        PageLookup::Stale
    ));
    next_request.revision = None;
    assert!(query(&db, &next_request).is_err());
}

#[test]
fn tool_filter_and_search_exclude_expired_tools_but_keep_skill_and_mcp() {
    let mut db = database();
    let boundary = NOW - DETAIL_RETENTION_MS;
    let old_tool = call(SESSION, "old-tool", boundary, 0, "shared");
    let new_tool = call(SESSION, "new-tool", boundary + 1, 10, "shared");
    let mut skill = call(SESSION, "old-skill", boundary, 20, "shared");
    let mut mcp = call(SESSION, "old-mcp", boundary, 30, "shared");
    if let ObservationKind::Call { kind, .. } = &mut skill.kind { *kind = "skill".into(); }
    if let ObservationKind::Call { kind, .. } = &mut mcp.kind { *kind = "mcp".into(); }
    commit(&mut db, b"a", false, &[old_tool, new_tool, skill, mcp]);
    let mut options = request(None);
    options.limit = 10;
    options.kind = Some("tool");
    let tools = page(query(&db, &options).unwrap());
    assert_eq!(tools.items.len(), 1);
    assert_eq!(tools.items[0].id, "new-tool");
    assert_eq!(tools.recent_tool, 1);
    assert_eq!(tools.tool, 2);
    options.kind = None;
    options.search = "shared";
    let all = page(query(&db, &options).unwrap());
    assert_eq!(all.matched, 3);
    assert!(all.items.iter().any(|item| item.id == "old-skill"));
    assert!(all.items.iter().any(|item| item.id == "old-mcp"));
    assert!(!all.items.iter().any(|item| item.id == "old-tool"));
}

#[test]
fn continuous_insertions_invalidate_stale_pages_and_details() {
    let mut db = database();
    commit(&mut db, b"a", false, &[
        call(SESSION, "first", NOW - 1000, 0, "functions.exec_command"),
        call(SESSION, "second", NOW - 999, 10, "functions.exec_command"),
        call(SESSION, "third", NOW - 998, 20, "functions.exec_command"),
    ]);
    for index in 0..50 {
        let mut initial = request(None);
        initial.revision = None;
        let first = page(query(&db, &initial).unwrap());
        let cursor = first.next_cursor.unwrap();
        let selected = &first.items[0];
        assert!(matches!(
            detail(&db, &selected.session_id, &selected.id, first.revision, NOW).unwrap(),
            DetailLookup::Ready(_)
        ));
        commit(&mut db, b"a", false, &[call(
            SESSION,
            &format!("appended-{index}"),
            NOW - 900 + index,
            100 + index as u64 * 10,
            "functions.exec_command",
        )]);
        db.execute("UPDATE usage_meta SET revision = revision + 1", []).unwrap();
        let mut next = request(None);
        next.revision = Some(first.revision);
        next.cursor = Some(&cursor);
        assert!(matches!(query(&db, &next).unwrap(), PageLookup::Stale));
        assert!(matches!(
            detail(&db, &selected.session_id, &selected.id, first.revision, NOW).unwrap(),
            DetailLookup::Stale
        ));
    }
}

#[test]
fn removes_expired_details_and_late_outputs_without_losing_time() {
    let mut db = database();
    let threshold = NOW - DETAIL_RETENTION_MS;
    let old = call(SESSION, "old", threshold, 0, "secret-tool-name");
    let newer = call(SESSION, "new", threshold + 1, 10, "visible");
    commit(
        &mut db,
        b"a",
        false,
        &[
            output(SESSION, "old", NOW - 1, 20),
            old,
            output(SESSION, "old", NOW, 30),
            newer,
            output(SESSION, "new", NOW, 40),
        ],
    );
    let saved: (Option<i64>, Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT end, name, namespace, call_kind FROM execution_observations WHERE call_id = 'old'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(saved, (None, Some("secret-tool-name".into()), Some("functions".into()), Some("tool".into())));
    let old_outputs: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'old' AND is_output = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_outputs, 0);
    assert!(matches!(
        detail(&db, SESSION, "old", 7, NOW).unwrap(),
        DetailLookup::NotFound
    ));
    assert!(matches!(
        detail(&db, SESSION, "new", 7, NOW).unwrap(),
        DetailLookup::Ready(_)
    ));

    let tx = db.transaction().unwrap();
    purge_details(&tx, NOW + 1).unwrap();
    tx.commit().unwrap();
    assert!(matches!(
        detail(&db, SESSION, "new", 7, NOW + 1).unwrap(),
        DetailLookup::NotFound
    ));
    let remaining_outputs: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM execution_observations WHERE is_output = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_outputs, 0);
}

#[test]
fn expired_details_are_cleaned_in_bounded_batches_without_exposing_them() {
    let mut db = database();
    let expired = NOW - DETAIL_RETENTION_MS;
    let original_now = NOW - 30_000;
    let mut old_items = vec![call(SESSION, "old", expired, 0, "private")];
    old_items.extend((0..600).map(|i| output(SESSION, "old", original_now - 1, 10 + i * 10)));
    old_items.push(call(SESSION, "recent", NOW - 60_000, 7_000, "visible"));
    old_items.push(output(SESSION, "recent", NOW - 59_999, 7_010));
    {
        let tx = db.transaction().unwrap();
        apply(&tx, b"a", false, &old_items, original_now).unwrap();
        apply(
            &tx,
            b"b",
            false,
            &[call(SESSION, "old", expired, 0, "private")],
            original_now,
        )
        .unwrap();
        tx.commit().unwrap();
    }

    for round in 0..10 {
        let tx = db.transaction().unwrap();
        let before = tx.total_changes();
        purge_details(&tx, NOW).unwrap();
        let changed = tx.total_changes() - before;
        assert!(changed <= 256, "round {round} changed {changed} rows");
        tx.commit().unwrap();

        let result = page(query(&db, &request(None)).unwrap());
        assert_eq!(result.total, 2);
        let old = result.items.iter().find(|item| item.id == "old").unwrap();
        assert_eq!(old.name.as_deref(), Some("private"));
        assert!(matches!(
            detail(&db, SESSION, "old", 7, NOW).unwrap(),
            DetailLookup::NotFound
        ));
        assert!(matches!(
            detail(&db, SESSION, "recent", 7, NOW).unwrap(),
            DetailLookup::Ready(_)
        ));
        if changed == 0 {
            break;
        }
        assert!(round < 9, "expiry did not finish");
    }
    let old_outputs: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM execution_observations
             WHERE call_id = 'old' AND is_output = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_outputs, 0);
    let retained_calls: (i64, i64) = db
        .query_row(
            "SELECT COUNT(*), COUNT(end) FROM execution_observations
             WHERE call_id = 'old' AND is_output = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(retained_calls, (2, 0));
}

#[test]
fn expiry_preserves_recent_output_from_a_conflicting_source() {
    let mut db = database();
    let initial_now = NOW - 1_000;
    {
        let tx = db.transaction().unwrap();
        apply(
            &tx,
            b"a",
            false,
            &[
                call(SESSION, "shared", NOW - DETAIL_RETENTION_MS, 0, "old"),
                output(SESSION, "shared", NOW - 3_000, 10),
            ],
            initial_now,
        )
        .unwrap();
        apply(
            &tx,
            b"b",
            false,
            &[
                call(SESSION, "shared", NOW - 2_000, 0, "new"),
                output(SESSION, "shared", NOW - 1_500, 10),
            ],
            initial_now,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    {
        let tx = db.transaction().unwrap();
        purge_details(&tx, NOW).unwrap();
        apply(&tx, b"a", true, &[], NOW).unwrap();
        tx.commit().unwrap();
    }
    let DetailLookup::Ready(found) = detail(&db, SESSION, "shared", 7, NOW).unwrap() else {
        panic!("recent source should remain readable after old source replacement");
    };
    assert_eq!(found.call.source, b"b");
    assert_eq!(found.outputs.len(), 1);
    assert_eq!(found.outputs[0].source, b"b");
}

#[test]
fn recent_output_is_not_rejected_by_an_expired_call_in_another_source() {
    let mut db = database();
    commit(
        &mut db,
        b"a",
        false,
        &[call(
            SESSION,
            "shared",
            NOW - DETAIL_RETENTION_MS,
            0,
            "old",
        )],
    );
    {
        let tx = db.transaction().unwrap();
        apply(
            &tx,
            b"b",
            false,
            &[
                call(SESSION, "shared", NOW - 2_000, 0, "new"),
                output(SESSION, "shared", NOW - 1_000, 10),
            ],
            NOW,
        )
        .unwrap();
        apply(&tx, b"a", true, &[], NOW).unwrap();
        tx.commit().unwrap();
    }
    let DetailLookup::Ready(found) = detail(&db, SESSION, "shared", 7, NOW).unwrap() else {
        panic!("recent source should remain readable");
    };
    assert_eq!(found.outputs.len(), 1);
    assert_eq!(found.outputs[0].source, b"b");
}

#[test]
fn expiry_preserves_conflicting_recent_output_in_the_same_source() {
    let mut db = database();
    {
        let tx = db.transaction().unwrap();
        apply(
            &tx,
            b"a",
            false,
            &[
                call(SESSION, "shared", NOW - DETAIL_RETENTION_MS, 0, "old"),
                call(SESSION, "shared", NOW - 2_000, 10, "new"),
                output(SESSION, "shared", NOW - 1_500, 20),
            ],
            NOW - 1_000,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    {
        let tx = db.transaction().unwrap();
        purge_details(&tx, NOW).unwrap();
        tx.commit().unwrap();
    }
    let retained: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM execution_observations
             WHERE source = X'61' AND call_id = 'shared' AND is_output = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, 1);
    assert!(matches!(
        detail(&db, SESSION, "shared", 7, NOW).unwrap(),
        DetailLookup::NotFound
    ));
    {
        let tx = db.transaction().unwrap();
        purge_details(&tx, NOW + DETAIL_RETENTION_MS).unwrap();
        tx.commit().unwrap();
    }
    let remaining: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM execution_observations
             WHERE source = X'61' AND call_id = 'shared' AND is_output = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn late_expired_call_does_not_delete_an_unbounded_number_of_outputs() {
    let mut db = database();
    let outputs = (0..600)
        .map(|i| output(SESSION, "old", NOW - 1, 10 + i * 10))
        .collect::<Vec<_>>();
    {
        let tx = db.transaction().unwrap();
        apply(&tx, b"a", false, &outputs, NOW).unwrap();
        tx.commit().unwrap();
    }
    {
        let tx = db.transaction().unwrap();
        let before = tx.total_changes();
        apply(
            &tx,
            b"a",
            false,
            &[call(
                SESSION,
                "old",
                NOW - DETAIL_RETENTION_MS,
                0,
                "private",
            )],
            NOW,
        )
        .unwrap();
        assert_eq!(tx.total_changes() - before, 1);
        tx.commit().unwrap();
    }
    assert!(matches!(
        detail(&db, SESSION, "old", 7, NOW).unwrap(),
        DetailLookup::NotFound
    ));
    for round in 0..10 {
        let tx = db.transaction().unwrap();
        let before = tx.total_changes();
        purge_details(&tx, NOW).unwrap();
        let changed = tx.total_changes() - before;
        assert!(changed <= 256, "round {round} changed {changed} rows");
        tx.commit().unwrap();
        if changed == 0 {
            break;
        }
        assert!(round < 9, "expiry did not finish");
    }
    let remaining_outputs: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM execution_observations
             WHERE call_id = 'old' AND is_output = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_outputs, 0);
    let result = page(query(&db, &request(None)).unwrap());
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].name.as_deref(), Some("private"));
}

#[test]
fn failed_expiry_rolls_back_outputs_and_call_marker() {
    let mut db = database();
    {
        let tx = db.transaction().unwrap();
        apply(
            &tx,
            b"a",
            false,
            &[
                call(SESSION, "old", NOW - DETAIL_RETENTION_MS, 0, "private"),
                output(SESSION, "old", NOW - 1, 10),
            ],
            NOW - 30_000,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    db.execute_batch(
        "CREATE TRIGGER fail_expiry BEFORE UPDATE ON execution_observations
         WHEN OLD.call_id = 'old' AND OLD.is_output = 0
         BEGIN SELECT RAISE(ABORT, 'synthetic expiry failure'); END;",
    )
    .unwrap();
    {
        let tx = db.transaction().unwrap();
        let error = purge_details(&tx, NOW).unwrap_err();
        assert!(format!("{error:#}").contains("synthetic expiry failure"));
    }
    let remaining: (i64, i64) = db
        .query_row(
            "SELECT SUM(is_output), COUNT(end) FROM execution_observations WHERE call_id = 'old'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(remaining, (1, 2));
    db.execute_batch("DROP TRIGGER fail_expiry;").unwrap();
    let tx = db.transaction().unwrap();
    purge_details(&tx, NOW).unwrap();
    tx.commit().unwrap();
    let remaining: (i64, i64) = db
        .query_row(
            "SELECT SUM(is_output), COUNT(end) FROM execution_observations WHERE call_id = 'old'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(remaining, (0, 0));
}

#[test]
fn ambiguous_copies_cannot_supply_details_or_hide_conflicts() {
    let mut db = database();
    commit(
        &mut db,
        b"a",
        false,
        &[call(SESSION, "one", NOW - 10, 0, "first")],
    );
    commit(
        &mut db,
        b"b",
        false,
        &[call(SESSION, "one", NOW - 10, 0, "different")],
    );
    let result = page(query(&db, &request(None)).unwrap());
    assert_eq!(result.total, 1);
    assert_eq!(result.conflicts, 1);
    assert!(result.items[0].name.is_none());
    assert!(matches!(
        detail(&db, SESSION, "one", 7, NOW).unwrap(),
        DetailLookup::NotFound
    ));
    commit(&mut db, b"b", true, &[]);
    assert!(matches!(
        detail(&db, SESSION, "one", 7, NOW).unwrap(),
        DetailLookup::Ready(_)
    ));
}

#[test]
fn copies_outside_the_requested_range_still_reveal_conflicts() {
    let mut db = database();
    commit(
        &mut db,
        b"a",
        false,
        &[call(SESSION, "one", NOW - 10, 0, "first")],
    );
    commit(
        &mut db,
        b"b",
        false,
        &[call(SESSION, "one", NOW - 100, 0, "first")],
    );
    let mut scoped = request(None);
    scoped.start_ms = NOW - 20;
    let result = page(query(&db, &scoped).unwrap());
    assert_eq!(result.total, 1);
    assert_eq!(result.conflicts, 1);
    assert_eq!(result.items[0].timestamp_ms, NOW - 10);
    assert!(result.items[0].name.is_none());
    assert!(matches!(
        detail(&db, SESSION, "one", 7, NOW).unwrap(),
        DetailLookup::NotFound
    ));
}

#[test]
fn future_calls_are_not_in_recent_summaries_or_search_results() {
    let mut db = database();
    commit(
        &mut db,
        b"a",
        false,
        &[
            call(SESSION, "present", NOW, 0, "functions.exec_command"),
            call(SESSION, "future", NOW + 1, 10, "functions.exec_command"),
        ],
    );
    let mut scoped = request(None);
    scoped.end_ms = NOW + 2;
    let result = page(query(&db, &scoped).unwrap());
    assert_eq!(result.total, 2);
    assert_eq!(result.recent_tool, 1);
    let future = result.items.iter().find(|item| item.id == "future").unwrap();
    assert!(future.name.is_none() && future.namespace.is_none() && future.kind.is_none());
    scoped.search = "exec_command";
    assert_eq!(page(query(&db, &scoped).unwrap()).matched, 1);
    scoped.search = "";
    scoped.kind = Some("tool");
    assert_eq!(page(query(&db, &scoped).unwrap()).matched, 1);
    assert!(matches!(
        detail(&db, SESSION, "future", 7, NOW).unwrap(),
        DetailLookup::NotFound
    ));
}

#[test]
fn details_use_one_source_and_preserve_order_and_identity() {
    let mut db = database();
    let one = call(SESSION, "one", NOW - 10, 0, "first");
    commit(&mut db, b"a", false, &[one.clone()]);
    commit(
        &mut db,
        b"b",
        false,
        &[
            one,
            output(SESSION, "one", NOW - 8, 10),
            output(SESSION, "one", NOW - 5, 20),
        ],
    );
    let DetailLookup::Ready(found) = detail(&db, SESSION, "one", 7, NOW).unwrap() else {
        panic!("missing detail");
    };
    assert_eq!(found.call.source, b"b");
    assert_eq!(found.call.identity, vec![3, 4]);
    assert_eq!(
        found
            .outputs
            .iter()
            .map(|item| item.start)
            .collect::<Vec<_>>(),
        [10, 20]
    );
    assert!(
        found
            .outputs
            .iter()
            .all(|item| item.source == found.call.source)
    );
    assert!(!found.outputs_truncated);
    assert!(matches!(
        detail(&db, SESSION, "one", 6, NOW).unwrap(),
        DetailLookup::Stale
    ));
}

#[test]
fn detail_source_selection_ignores_expired_outputs() {
    let mut db = database();
    let now_before_expiry = NOW - 2_000;
    {
        let tx = db.transaction().unwrap();
        apply(
            &tx,
            b"a",
            false,
            &[
                call(SESSION, "one", NOW - 5_000, 0, "functions.exec_command"),
                output(SESSION, "one", NOW - DETAIL_RETENTION_MS, 10),
                output(SESSION, "one", NOW - DETAIL_RETENTION_MS, 20),
            ],
            now_before_expiry,
        )
        .unwrap();
        apply(
            &tx,
            b"b",
            false,
            &[
                call(SESSION, "one", NOW - 5_000, 0, "functions.exec_command"),
                output(SESSION, "one", NOW - 3_000, 10),
            ],
            now_before_expiry,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let DetailLookup::Ready(found) = detail(&db, SESSION, "one", 7, NOW).unwrap() else {
        panic!("missing detail");
    };
    assert_eq!(found.call.source, b"b");
    assert_eq!(found.outputs.len(), 1);
}

#[test]
fn details_report_an_incomplete_source_without_hiding_available_output() {
    let mut db = database();
    commit(
        &mut db,
        b"a",
        false,
        &[
            call(SESSION, "one", NOW - 10, 0, "functions.exec_command"),
            output(SESSION, "one", NOW - 8, 10),
        ],
    );
    db.execute(
        "UPDATE usage_sources SET execution_valid = 0 WHERE source = X'61'",
        [],
    )
    .unwrap();
    let DetailLookup::Ready(found) = detail(&db, SESSION, "one", 7, NOW).unwrap() else {
        panic!("missing detail");
    };
    assert_eq!(found.outputs.len(), 1);
    assert!(found.outputs_truncated);
}

fn running(
    session: &str,
    id: &str,
    timestamp_ms: i64,
    start: u64,
    process: &str,
) -> ExecutionObservation {
    let mut event = output(session, id, timestamp_ms, start);
    event.kind = ObservationKind::Output {
        process_id: Some(process.to_owned()),
    };
    event
}

fn poll(
    session: &str,
    id: &str,
    timestamp_ms: i64,
    start: u64,
    process: &str,
) -> ExecutionObservation {
    let mut event = call(session, id, timestamp_ms, start, "functions.write_stdin");
    let ObservationKind::Call { process_id, .. } = &mut event.kind else {
        unreachable!()
    };
    *process_id = Some(process.to_owned());
    event
}

#[test]
fn links_only_explicit_poll_session_and_preserves_empty_and_failed_segments() {
    let mut db = database();
    commit(
        &mut db,
        b"a",
        false,
        &[
            call(SESSION, "root", NOW - 20, 0, "functions.exec_command"),
            running(SESSION, "root", NOW - 19, 10, "77"),
            call(SESSION, "other", NOW - 18, 20, "functions.exec_command"),
            running(SESSION, "other", NOW - 17, 30, "88"),
            poll(SESSION, "empty-poll", NOW - 16, 40, "77"),
            running(SESSION, "empty-poll", NOW - 15, 50, "77"),
            poll(SESSION, "final-poll", NOW - 14, 60, "77"),
            output(SESSION, "final-poll", NOW - 13, 70),
            poll(SESSION, "other-poll", NOW - 12, 80, "88"),
            output(SESSION, "other-poll", NOW - 11, 90),
        ],
    );
    let DetailLookup::Ready(found) = detail(&db, SESSION, "root", 7, NOW).unwrap() else {
        panic!("missing detail");
    };
    assert_eq!(
        found
            .outputs
            .iter()
            .map(|item| item.call_id.as_str())
            .collect::<Vec<_>>(),
        ["root", "empty-poll", "final-poll"]
    );
    assert!(!found.outputs_truncated);
    assert_eq!(page(query(&db, &request(None)).unwrap()).total, 2);
}

#[test]
fn future_outputs_do_not_affect_recent_details_or_process_linking() {
    let mut db = database();
    commit(
        &mut db,
        b"a",
        false,
        &[
            call(SESSION, "root", NOW - 20, 0, "functions.exec_command"),
            running(SESSION, "root", NOW - 19, 10, "77"),
            running(SESSION, "root", NOW + 1, 20, "88"),
            poll(SESSION, "poll", NOW - 17, 30, "77"),
            output(SESSION, "poll", NOW - 16, 40),
        ],
    );
    let DetailLookup::Ready(found) = detail(&db, SESSION, "root", 7, NOW).unwrap() else {
        panic!("missing detail");
    };
    assert_eq!(
        found
            .outputs
            .iter()
            .map(|item| (item.call_id.as_str(), item.timestamp_ms))
            .collect::<Vec<_>>(),
        [("root", NOW - 19), ("poll", NOW - 16)]
    );
    assert!(!found.outputs_truncated);
}

#[test]
fn refuses_reused_process_ids_and_marks_foreign_poll_as_partial() {
    let mut db = database();
    commit(
        &mut db,
        b"a",
        false,
        &[
            call(SESSION, "root", NOW - 20, 0, "functions.exec_command"),
            running(SESSION, "root", NOW - 19, 10, "77"),
            poll(SESSION, "local-poll", NOW - 18, 20, "77"),
            output(SESSION, "local-poll", NOW - 17, 30),
        ],
    );
    commit(
        &mut db,
        b"b",
        false,
        &[
            call(SESSION, "other", NOW - 16, 0, "functions.exec_command"),
            running(SESSION, "other", NOW - 15, 10, "77"),
        ],
    );
    let DetailLookup::Ready(found) = detail(&db, SESSION, "root", 7, NOW).unwrap() else {
        panic!("missing detail");
    };
    assert_eq!(
        found
            .outputs
            .iter()
            .map(|item| item.call_id.as_str())
            .collect::<Vec<_>>(),
        ["root"]
    );
    assert!(found.outputs_truncated);

    commit(
        &mut db,
        b"b",
        true,
        &[
            poll(SESSION, "foreign-poll", NOW - 14, 20, "77"),
            output(SESSION, "foreign-poll", NOW - 13, 30),
        ],
    );
    let DetailLookup::Ready(found) = detail(&db, SESSION, "root", 7, NOW).unwrap() else {
        panic!("missing detail");
    };
    assert_eq!(
        found
            .outputs
            .iter()
            .map(|item| item.call_id.as_str())
            .collect::<Vec<_>>(),
        ["root", "local-poll"]
    );
    assert!(found.outputs_truncated);
}

#[test]
fn parses_running_receipt_and_poll_id_without_copying_output() {
    let timestamp = DateTime::from_timestamp_millis(NOW).unwrap().to_rfc3339();
    let raw_output = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call_output","call_id":"root",
        "output":"Process running with session ID 77\nOutput:\nprivate output"
    }});
    let found = parse_line(
        &serde_json::to_vec(&raw_output).unwrap(),
        0,
        10,
        SESSION,
        false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        found.kind,
        ObservationKind::Output {
            process_id: Some("77".into())
        }
    );
    assert!(!format!("{found:?}").contains("private output"));
    let arguments = json!({"session_id":77,"chars":"private input"});
    for arguments in [json!(arguments.to_string()), arguments] {
        let raw_poll = json!({"type":"response_item","timestamp":timestamp,"payload":{
            "type":"function_call","call_id":"poll","name":"functions.write_stdin",
            "arguments":arguments
        }});
        let found = parse_line(
            &serde_json::to_vec(&raw_poll).unwrap(),
            10,
            20,
            SESSION,
            false,
        )
        .unwrap()
        .unwrap();
        assert!(
            matches!(found.kind, ObservationKind::Call { process_id: Some(ref id), .. } if id == "77")
        );
        assert!(!format!("{found:?}").contains("private input"));
    }
}

#[test]
fn parses_only_complete_bounded_output_receipt_prefixes() {
    let timestamp = DateTime::from_timestamp_millis(NOW).unwrap().to_rfc3339();
    let output = format!(
        "Chunk ID: abc\nProcess running with session ID 77\n{}",
        "private output".repeat(MAX_RECEIPT_PREFIX_BYTES)
    );
    let raw_output = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call_output","call_id":"root","output":output
    }});
    let found = parse_line(
        &serde_json::to_vec(&raw_output).unwrap(),
        0,
        10,
        SESSION,
        false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        found.kind,
        ObservationKind::Output {
            process_id: Some("77".into())
        }
    );
    assert!(!format!("{found:?}").contains("private output"));

    let truncated = format!(
        "Process running with session ID {}",
        "7".repeat(MAX_RECEIPT_PREFIX_BYTES)
    );
    let raw_output = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call_output","call_id":"root","output":truncated
    }});
    let found = parse_line(
        &serde_json::to_vec(&raw_output).unwrap(),
        0,
        10,
        SESSION,
        false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(found.kind, ObservationKind::Output { process_id: None });

    let raw_output = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call_output","call_id":"root","output":{"text":"7".repeat(300_000)}
    }});
    let found = parse_line(
        &serde_json::to_vec(&raw_output).unwrap(),
        0,
        10,
        SESSION,
        false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(found.kind, ObservationKind::Output { process_id: None });
}

#[test]
fn failed_transaction_does_not_publish_calls() {
    let mut db = database();
    {
        let tx = db.transaction().unwrap();
        apply(
            &tx,
            b"a",
            false,
            &[call(SESSION, "one", NOW, 0, "first")],
            NOW,
        )
        .unwrap();
    }
    assert_eq!(page(query(&db, &request(None)).unwrap()).total, 0);
}

#[test]
fn old_completed_calls_compact_while_an_unrelated_source_is_incomplete() {
    let mut db = database();
    let old_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let old_day = DateTime::from_timestamp_millis(old_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    commit(
        &mut db,
        b"a",
        false,
        &[
            call(SESSION, "completed", old_ms, 0, "completed"),
            call(SESSION, "shared", old_ms + 1, 10, "shared"),
        ],
    );
    commit(
        &mut db,
        b"b",
        false,
        &[
            call(SESSION, "pending", old_ms + 2, 0, "pending"),
            call(SESSION, "shared", old_ms + 1, 10, "shared"),
        ],
    );
    db.execute("UPDATE usage_sources SET complete = 0 WHERE source = X'62'", [])
        .unwrap();

    let tx = db.transaction().unwrap();
    assert_eq!(compact_completed(&tx, NOW).unwrap(), 1);
    tx.commit().unwrap();
    let summary = daily_summary(&db, old_day, old_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summary[0].count, 1);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        3
    );

    db.execute("UPDATE usage_sources SET complete = 1 WHERE source = X'62'", [])
        .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(compact_completed(&tx, NOW).unwrap(), 3);
    tx.commit().unwrap();
    let summary = daily_summary(&db, old_day, old_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summary[0].count, 3);
    assert_eq!(summary[0].tool, 3);
    assert!(summary[0].classification_complete);
    assert!(!summary[0].partial);
}

#[test]
fn named_activity_is_bounded_and_deduplicated_without_a_detail_time_limit() {
    let mut db = database();
    let old = NOW - DETAIL_RETENTION_MS - 10_000;
    let calls = (0..150).map(|i| {
        let mut item = call(SESSION, &format!("call-{i:03}"), old + i, i as u64 * 10,
            &format!("skill-{i:03}"));
        if let ObservationKind::Call { kind, .. } = &mut item.kind { *kind = "skill".into(); }
        item
    }).collect::<Vec<_>>();
    commit(&mut db, b"a", false, &calls);
    commit(&mut db, b"b", false, &calls);
    let mut options = request(None);
    options.kind = Some("skill");
    let summary = activity_summary(&db, &options).unwrap();
    assert_eq!(summary["groupCount"], 150);
    let groups = summary["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 100);
    for group in groups {
        assert_eq!(group["count"], 1);
        let timeline = group["timeline"].as_array().unwrap();
        assert_eq!(timeline.len(), 1);
        assert!(timeline[0][0].as_u64().unwrap() < 64);
        assert_eq!(timeline[0][1], 1);
    }
    assert_eq!(page(query(&db, &options).unwrap()).skill, 150);
}

#[test]
fn activity_hides_expired_tool_names_without_changing_counts_or_named_skills() {
    let mut db = database();
    let old = NOW - DETAIL_RETENTION_MS;
    let mut skill = call(SESSION, "skill", old, 20, "pdf");
    let mut mcp = call(SESSION, "mcp", old, 30, "mcp__server__search");
    if let ObservationKind::Call { kind, .. } = &mut skill.kind { *kind = "skill".into(); }
    if let ObservationKind::Call { kind, .. } = &mut mcp.kind { *kind = "mcp".into(); }
    let calls = [
        call(SESSION, "old", old, 0, "private-old-tool"),
        call(SESSION, "recent", old + 1, 10, "recent-tool"),
        skill,
        mcp,
    ];
    commit(&mut db, b"a", false, &calls);
    commit(&mut db, b"b", false, &calls);
    let summary = activity_summary(&db, &request(None)).unwrap();
    let groups = summary["groups"].as_array().unwrap();
    assert_eq!(summary["groupCount"], 4);
    for (kind, name) in [
        ("tool", ""),
        ("tool", "recent-tool"),
        ("skill", "pdf"),
        ("mcp", "mcp__server__search"),
    ] {
        let group = groups.iter().find(|group| group["kind"] == kind && group["name"] == name).unwrap();
        assert_eq!(group["count"], 1);
        assert_eq!(group["timeline"][0][1], 1);
    }
    assert!(!summary.to_string().contains("private-old-tool"));
    let page = page(query(&db, &request(None)).unwrap());
    assert_eq!((page.total, page.skill, page.mcp, page.tool), (4, 1, 1, 2));
}

#[test]
fn old_skill_and_mcp_daily_compaction_keeps_classification_counts() {
    let mut db = database();
    let old = NOW - 70 * DETAIL_RETENTION_MS;
    let mut skill = call(SESSION, "skill", old, 0, "pdf");
    let mut mcp = call(SESSION, "mcp", old + 1, 10, "mcp__test__query");
    if let ObservationKind::Call { kind, .. } = &mut skill.kind { *kind = "skill".into(); }
    if let ObservationKind::Call { kind, .. } = &mut mcp.kind { *kind = "mcp".into(); }
    commit(&mut db, b"a", false, &[skill, mcp]);
    let tx = db.transaction().unwrap();
    assert_eq!(compact_completed(&tx, NOW).unwrap(), 2);
    tx.commit().unwrap();
    let day = DateTime::from_timestamp_millis(old).unwrap().with_timezone(&Local).date_naive();
    let summary = daily_summary(&db, day, day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summary[0].count, 2);
    assert_eq!(summary[0].skill, 1);
    assert_eq!(summary[0].mcp, 1);
    assert!(summary[0].classification_complete);
}

#[test]
fn legacy_metadata_repair_is_bounded_and_does_not_restore_expired_details() {
    use super::super::persistent_index::{encode_identity, encode_source};
    use std::fs::{self, File};
    let root = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(root.path()).unwrap();
    fs::create_dir(home.join("sessions")).unwrap();
    let path = home.join("sessions/metadata.jsonl");
    let at = NOW - 2 * DETAIL_RETENTION_MS;
    let timestamp = DateTime::from_timestamp_millis(at).unwrap().to_rfc3339();
    let mut log = String::new();
    let mut observations = Vec::new();
    for i in 0..70 {
        let line = format!("{}\n", json!({"type":"response_item","timestamp":timestamp,
            "payload":{"type":"function_call","call_id":format!("call-{i}"),
                "name":"mcp__fixture__query","arguments":"{}"}}));
        observations.push(parse_line(line.as_bytes(), log.len() as u64,
            (log.len() + line.len()) as u64, SESSION, false).unwrap().unwrap());
        log.push_str(&line);
    }
    fs::write(&path, log).unwrap();
    let path = fs::canonicalize(path).unwrap();
    let source = encode_source(&path);
    let identity = encode_identity(
        codex_plus_core::codex_sqlite::opened_file_identity(&File::open(&path).unwrap()).unwrap());
    let mut db = database();
    db.execute("INSERT INTO usage_sources(source, identity, complete) VALUES(?1, ?2, 1)",
        params![source, identity]).unwrap();
    commit(&mut db, &source, false, &observations);
    db.execute("UPDATE execution_observations SET name = NULL, namespace = NULL, call_kind = NULL",
        []).unwrap();
    let mut options = request(None);
    options.start_ms = at - 1;
    options.revision = None;
    let before = page(query(&db, &options).unwrap());
    assert_eq!(before.total, 70);
    assert_eq!(before.classified, 0);
    let first = repair_metadata(&mut db, &home, None).unwrap();
    assert!(first.next.is_some());
    assert!(first.failures.is_empty());
    let partial = page(query(&db, &options).unwrap());
    assert_eq!(partial.total, 70);
    assert_eq!(partial.mcp, 64);
    let second = repair_metadata(&mut db, &home, first.next.as_ref()).unwrap();
    assert!(second.next.is_none());
    assert!(second.failures.is_empty());
    let after = page(query(&db, &options).unwrap());
    assert_eq!(after.total, 70);
    assert_eq!(after.classified, 70);
    assert!(matches!(detail(&db, SESSION, "call-0", after.revision, NOW).unwrap(),
        DetailLookup::NotFound));
    assert_eq!(db.query_row("SELECT COUNT(end) FROM execution_observations", [],
        |row| row.get::<_, i64>(0)).unwrap(), 0);

    db.execute("UPDATE execution_observations SET name = NULL, namespace = NULL, call_kind = NULL
        WHERE call_id = 'call-0'", []).unwrap();
    db.execute("UPDATE execution_observations SET name = 'functions.exec_command', call_kind = 'skill'
        WHERE call_id = 'call-1'", []).unwrap();
    fs::rename(&path, path.with_extension("unavailable")).unwrap();
    let unavailable = repair_metadata(&mut db, &home, None).unwrap();
    assert!(unavailable.next.is_some());
    assert_eq!(unavailable.failures.len(), 2);
    let finished = repair_metadata(&mut db, &home, unavailable.next.as_ref()).unwrap();
    assert!(finished.next.is_none());
    assert!(finished.failures.is_empty());
    let incomplete = page(query(&db, &options).unwrap());
    assert_eq!(incomplete.total, 70);
    assert_eq!(incomplete.classified, 69);
    assert_eq!(incomplete.skill, 1);
}

#[test]
fn invalid_metadata_source_reports_failure_without_blocking_committed_counts() {
    let mut db = database();
    commit(&mut db, b"a", false, &[call(SESSION, "old", NOW - 1, 0, "tool")]);
    db.execute("UPDATE execution_observations SET name = NULL, namespace = NULL, call_kind = NULL",
        []).unwrap();
    let repair = repair_metadata(&mut db, std::path::Path::new("fixture-home"), None).unwrap();
    assert!(repair.next.is_none());
    assert_eq!(repair.failures.len(), 1);
    assert!(repair.failures[0].1.contains("invalid execution metadata source in index"));
    let result = page(query(&db, &request(None)).unwrap());
    assert_eq!(result.total, 1);
    assert_eq!(result.classified, 0);
    assert_eq!(result.revision, 7);
}

#[test]
fn metadata_repair_advances_over_known_rows_before_reading_later_missing_classification() {
    use super::super::persistent_index::{encode_identity, encode_source};
    use std::fs::{self, File};
    let root = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(root.path()).unwrap();
    fs::create_dir(home.join("sessions")).unwrap();
    let path = home.join("sessions/metadata-pages.jsonl");
    let at = NOW - DETAIL_RETENTION_MS - 1;
    let timestamp = DateTime::from_timestamp_millis(at).unwrap().to_rfc3339();
    let mut log = String::new();
    let mut observations = Vec::new();
    for i in 0..65 {
        let line = format!("{}\n", json!({"type":"response_item","timestamp":timestamp,
            "payload":{"type":"function_call","call_id":format!("call-{i}"),
                "name":"mcp__fixture__query","arguments":"{}"}}));
        observations.push(parse_line(line.as_bytes(), log.len() as u64,
            (log.len() + line.len()) as u64, SESSION, false).unwrap().unwrap());
        log.push_str(&line);
    }
    fs::write(&path, log).unwrap();
    let path = fs::canonicalize(path).unwrap();
    let source = encode_source(&path);
    let identity = encode_identity(
        codex_plus_core::codex_sqlite::opened_file_identity(&File::open(&path).unwrap()).unwrap());
    let mut db = database();
    db.execute("INSERT INTO usage_sources(source, identity, complete) VALUES(?1, ?2, 1)",
        params![source, identity]).unwrap();
    commit(&mut db, &source, false, &observations);
    db.execute("UPDATE execution_observations SET name = NULL, namespace = NULL, call_kind = NULL
        WHERE call_id = 'call-64'", []).unwrap();

    let temporarily_unavailable = path.with_extension("unavailable");
    fs::rename(&path, &temporarily_unavailable).unwrap();
    let first = repair_metadata(&mut db, &home, None).unwrap();
    assert!(first.next.is_some());
    assert!(first.failures.is_empty());
    assert_eq!(revision(&db).unwrap(), 7);
    fs::rename(&temporarily_unavailable, &path).unwrap();
    let second = repair_metadata(&mut db, &home, first.next.as_ref()).unwrap();
    assert!(second.next.is_none());
    assert!(second.failures.is_empty());
    let mut options = request(None);
    options.revision = None;
    let result = page(query(&db, &options).unwrap());
    assert_eq!(result.total, 65);
    assert_eq!(result.mcp, 65);
    assert_eq!(result.revision, 8);
    assert_eq!(db.query_row("SELECT COUNT(end) FROM execution_observations", [],
        |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn old_calls_with_missing_outputs_compact_as_partial_without_weakening_scope_checks() {
    let mut db = database();
    let old_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let old_day = DateTime::from_timestamp_millis(old_ms)
        .unwrap().with_timezone(&Local).date_naive();
    commit(&mut db, b"a", false, &[
        call(SESSION, "missing-output", old_ms, 0, "functions.exec_command"),
        call(SESSION, "duplicate", old_ms, 10, "functions.exec_command"),
    ]);
    commit(&mut db, b"b", false, &[
        call(SESSION, "duplicate", old_ms, 0, "functions.exec_command"),
        call(OTHER_SESSION, "scope-invalid", old_ms, 10, "functions.exec_command"),
    ]);
    db.execute("UPDATE usage_sources SET execution_valid = 0 WHERE source = X'61'", []).unwrap();
    db.execute("UPDATE usage_sources SET scope_valid = 0 WHERE source = X'62'", []).unwrap();
    let tx = db.transaction().unwrap();
    let batch = compact_batch(&tx, NOW, None, false).unwrap();
    assert_eq!(batch.deleted, 1);
    tx.commit().unwrap();
    let summary = daily_summary(&db, old_day, old_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summary[0].count, 1);
    assert_eq!(summary[0].tool, 1);
    assert!(summary[0].partial);

    db.execute("UPDATE usage_sources SET scope_valid = 1 WHERE source = X'62'", []).unwrap();
    let tx = db.transaction().unwrap();
    let batch = compact_batch(&tx, NOW, None, false).unwrap();
    assert_eq!(batch.deleted, 3);
    tx.commit().unwrap();
    let summary = daily_summary(&db, old_day, old_day.succ_opt().unwrap(),
        Some(&[SESSION.to_owned()])).unwrap();
    let affected = &summary[0];
    assert_eq!(affected.count, 2);
    assert_eq!(affected.tool, 2);
    assert!(affected.partial);
    let summary = daily_summary(&db, old_day, old_day.succ_opt().unwrap(),
        Some(&[OTHER_SESSION.to_owned()])).unwrap();
    let independent = &summary[0];
    assert_eq!(independent.count, 1);
    assert!(!independent.partial);
}

#[test]
fn storage_pressure_compacts_incomplete_old_calls_as_partial_and_preserves_recent_details() {
    let mut db = database();
    let old = NOW - 70 * 24 * 60 * 60 * 1000;
    let recent = NOW - 60_000;
    let mut skill = call(SESSION, "skill", old, 0, "pdf");
    let mut mcp = call(SESSION, "mcp", old + 1, 10, "mcp__fixture__query");
    if let ObservationKind::Call { kind, .. } = &mut skill.kind { *kind = "skill".into(); }
    if let ObservationKind::Call { kind, .. } = &mut mcp.kind { *kind = "mcp".into(); }
    let items = [skill, mcp, call(SESSION, "recent", recent, 20, "functions.exec_command")];
    commit(&mut db, b"a", false, &items);
    db.execute("UPDATE usage_sources SET complete = 0 WHERE source = X'61'", []).unwrap();

    let tx = db.transaction().unwrap();
    assert_eq!(compact_batch(&tx, NOW, None, false).unwrap().deleted, 0);
    tx.commit().unwrap();
    let tx = db.transaction().unwrap();
    let pressure = compact_batch(&tx, NOW, None, true).unwrap();
    assert_eq!(pressure.deleted, 2);
    assert!(pressure.summarized);
    tx.rollback().unwrap();
    assert_eq!(db.query_row("SELECT COUNT(*) FROM execution_observations", [], |row| row.get::<_, u64>(0)).unwrap(), 3);

    let tx = db.transaction().unwrap();
    assert_eq!(compact_batch(&tx, NOW, None, true).unwrap().deleted, 2);
    tx.commit().unwrap();
    let day = DateTime::from_timestamp_millis(old).unwrap().with_timezone(&Local).date_naive();
    let summary = daily_summary(&db, day, day.succ_opt().unwrap(), None).unwrap();
    assert_eq!((summary[0].count, summary[0].skill, summary[0].mcp), (2, 1, 1));
    assert!(summary[0].partial);
    let mut options = request(None);
    options.revision = None;
    assert_eq!(page(query(&db, &options).unwrap()).recent_tool, 1);
    assert!(matches!(detail(&db, SESSION, "recent", 7, NOW).unwrap(), DetailLookup::Ready(_)));

    commit(&mut db, b"a", false, &[call(SESSION, "late", old + 2, 30, "tool")]);
    let summary = daily_summary(&db, day, day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summary[0].count, 2, "late calls keep a marked lower bound without double counting");
    assert!(summary[0].partial);
    commit(&mut db, b"a", true, &[call(SESSION, "replacement", old + 3, 40, "tool")]);
    let summary = daily_summary(&db, day, day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summary[0].count, 2);
    assert!(summary[0].partial);
}

#[test]
fn old_call_compaction_is_bounded_and_rolls_back_as_one_transaction() {
    let mut db = database();
    let old_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let old_day = DateTime::from_timestamp_millis(old_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    let calls = (0..300)
        .map(|i| call(SESSION, &format!("call-{i:03}"), old_ms + i, i as u64 * 10, "call"))
        .collect::<Vec<_>>();
    commit(&mut db, b"a", false, &calls);
    {
        let tx = db.transaction().unwrap();
        let removed = compact_completed(&tx, NOW).unwrap();
        assert!(removed > 0 && removed <= 256, "removed {removed} rows");
        tx.rollback().unwrap();
    }
    assert!(daily_summary(&db, old_day, old_day.succ_opt().unwrap(), None)
        .unwrap()
        .is_empty());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        300
    );
    loop {
        let tx = db.transaction().unwrap();
        let removed = compact_completed(&tx, NOW).unwrap();
        tx.commit().unwrap();
        assert!(removed <= 256);
        if removed == 0 {
            break;
        }
    }
    let summary = daily_summary(&db, old_day, old_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summary[0].count, 300);
    assert!(!summary[0].partial);
}

#[test]
fn oversized_identical_observations_are_reduced_without_losing_conflicts() {
    let mut db = database();
    let first_ms = NOW - 71 * 24 * 60 * 60 * 1000;
    let later_ms = first_ms + 24 * 60 * 60 * 1000;
    let first_day = DateTime::from_timestamp_millis(first_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    let later_day = DateTime::from_timestamp_millis(later_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    let duplicates = (0..600)
        .map(|i| call(SESSION, "duplicate", first_ms, i * 10, "call"))
        .collect::<Vec<_>>();
    commit(&mut db, b"a", false, &duplicates);
    commit(
        &mut db,
        b"b",
        false,
        &[
            call(SESSION, "duplicate", later_ms, 0, "call"),
            call(OTHER_SESSION, "independent", later_ms + 1, 10, "call"),
        ],
    );
    let mut range = request(None);
    range.start_ms = first_ms - 1;
    assert_eq!(page(query(&db, &range).unwrap()).total, 2);
    {
        let tx = db.transaction().unwrap();
        let batch = compact_batch(&tx, NOW, None, false).unwrap();
        assert!(batch.deleted > 0 && batch.deleted <= 256);
        tx.rollback().unwrap();
    }
    let before: usize = db
        .query_row("SELECT COUNT(*) FROM execution_observations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, 602);

    let mut cursor = None;
    for _ in 0..20 {
        let tx = db.transaction().unwrap();
        let batch = compact_batch(&tx, NOW, cursor.as_ref(), false).unwrap();
        assert!(batch.deleted <= 256);
        cursor = batch.next;
        tx.commit().unwrap();
        let remaining: usize = db
            .query_row("SELECT COUNT(*) FROM execution_observations", [], |row| row.get(0))
            .unwrap();
        let summarized: u64 = db
            .query_row("SELECT COALESCE(SUM(count), 0) FROM execution_daily", [], |row| {
                row.get(0)
            })
            .unwrap();
        let uncompressed = page(query(&db, &range).unwrap()).total;
        assert_eq!(uncompressed + summarized, 2);
        if remaining == 0 {
            break;
        }
    }
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM execution_observations", [], |row| row
            .get::<_, usize>(0))
            .unwrap(),
        0
    );
    let first = daily_summary(&db, first_day, first_day.succ_opt().unwrap(), None).unwrap();
    let later = daily_summary(&db, later_day, later_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(first[0].count, 1);
    assert!(first[0].partial);
    assert_eq!(later[0].count, 1);
    assert!(later[0].partial);
    let links: usize = db
        .query_row(
            "SELECT COUNT(*) FROM execution_compacted_sources WHERE session = ?1",
            [SESSION],
            |row| row.get(0),
        )
        .unwrap();
    assert!(links >= 2);
}

#[test]
fn an_oversized_duplicate_group_is_reduced_before_compaction() {
    let mut db = database();
    let old_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let duplicates = (0..257)
        .map(|i| call(SESSION, "duplicate", old_ms, i * 10, "call"))
        .collect::<Vec<_>>();
    commit(&mut db, b"a", false, &duplicates);
    commit(
        &mut db,
        b"b",
        false,
        &[call(OTHER_SESSION, "independent", old_ms + 1, 0, "call")],
    );
    let tx = db.transaction().unwrap();
    assert_eq!(compact_completed(&tx, NOW).unwrap(), 256);
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        2
    );
    let tx = db.transaction().unwrap();
    assert_eq!(compact_completed(&tx, NOW).unwrap(), 2);
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT SUM(count) FROM execution_daily", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        2
    );
}

#[test]
fn compaction_resumes_after_a_bounded_scan_of_incomplete_sources() {
    let mut db = database();
    let old_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let pending = (0..2_500)
        .map(|i| call(SESSION, &format!("pending-{i:05}"), old_ms + i, i as u64 * 10, "call"))
        .collect::<Vec<_>>();
    commit(&mut db, b"b", false, &pending);
    commit(
        &mut db,
        b"a",
        false,
        &[call(OTHER_SESSION, "completed", old_ms + 3_000, 0, "call")],
    );
    db.execute("UPDATE usage_sources SET complete = 0 WHERE source = X'62'", [])
        .unwrap();

    let mut cursor = None;
    for _ in 0..2 {
        let tx = db.transaction().unwrap();
        let batch = compact_batch(&tx, NOW, cursor.as_ref(), false).unwrap();
        assert_eq!(batch.deleted, 0);
        assert!(batch.next.is_some());
        cursor = batch.next;
        tx.commit().unwrap();
    }
    db.execute_batch(
        "CREATE TRIGGER fail_compaction BEFORE INSERT ON execution_daily
         BEGIN SELECT RAISE(ABORT, 'synthetic compaction failure'); END;",
    )
    .unwrap();
    {
        let tx = db.transaction().unwrap();
        let error = compact_batch(&tx, NOW, cursor.as_ref(), false).err().unwrap();
        assert!(format!("{error:#}").contains("synthetic compaction failure"));
    }
    db.execute_batch("DROP TRIGGER fail_compaction").unwrap();
    let tx = db.transaction().unwrap();
    let batch = compact_batch(&tx, NOW, cursor.as_ref(), false).unwrap();
    assert_eq!(batch.deleted, 1);
    assert!(batch.next.is_none());
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap(),
        2_500
    );
}

#[test]
fn compaction_cursor_advances_past_a_partly_reduced_duplicate_group() {
    let mut db = database();
    let old_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let duplicates = (0..2_000)
        .map(|i| call(SESSION, "duplicate", old_ms, i * 10, "call"))
        .collect::<Vec<_>>();
    commit(&mut db, b"a", false, &duplicates);
    commit(
        &mut db,
        b"b",
        false,
        &[call(OTHER_SESSION, "independent", old_ms + 1, 0, "call")],
    );
    let tx = db.transaction().unwrap();
    let first = compact_batch(&tx, NOW, None, false).unwrap();
    assert_eq!(first.deleted, 256);
    assert!(first.next.is_some());
    tx.commit().unwrap();

    let tx = db.transaction().unwrap();
    let second = compact_batch(&tx, NOW, first.next.as_ref(), false).unwrap();
    assert_eq!(second.deleted, 1);
    assert!(second.next.is_none());
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap(),
        1_744
    );
}

#[test]
fn oversized_distinct_timestamps_remain_visible_and_do_not_block_other_calls() {
    let mut db = database();
    let old_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let conflicts = (0..257)
        .map(|i| call(SESSION, "conflict", old_ms + i, i as u64 * 10, "call"))
        .collect::<Vec<_>>();
    commit(&mut db, b"a", false, &conflicts);
    commit(
        &mut db,
        b"b",
        false,
        &[call(OTHER_SESSION, "independent", old_ms + 300, 0, "call")],
    );
    let tx = db.transaction().unwrap();
    let batch = compact_batch(&tx, NOW, None, false).unwrap();
    assert_eq!(batch.deleted, 1);
    assert!(!batch.reduced);
    tx.commit().unwrap();
    let mut range = request(None);
    range.start_ms = old_ms - 1;
    let page = page(query(&db, &range).unwrap());
    assert_eq!(page.total, 1);
    assert_eq!(page.conflicts, 1);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap(),
        257
    );
}

#[test]
fn compaction_respects_the_local_day_boundary_and_newer_duplicate() {
    let mut db = database();
    let before_day = DateTime::from_timestamp_millis(NOW).unwrap()
        .with_timezone(&Local).date_naive()
        .checked_sub_days(chrono::Days::new(59)).unwrap();
    let boundary = Local
        .from_local_datetime(&before_day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap()
        .timestamp_millis();
    commit(
        &mut db,
        b"a",
        false,
        &[
            call(SESSION, "old", boundary - 1, 0, "call"),
            call(SESSION, "boundary", boundary, 10, "call"),
            call(SESSION, "duplicate", boundary - 2, 20, "call"),
            call(SESSION, "duplicate", boundary, 30, "call"),
        ],
    );
    let tx = db.transaction().unwrap();
    assert_eq!(compact_completed(&tx, NOW).unwrap(), 1);
    tx.commit().unwrap();
    let remaining: Vec<String> = db
        .prepare("SELECT call_id FROM execution_observations ORDER BY start")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(remaining, ["boundary", "duplicate", "duplicate"]);
    let old_day = before_day.pred_opt().unwrap();
    let summary = daily_summary(&db, old_day, before_day, None).unwrap();
    assert_eq!((summary[0].count, summary[0].tool), (1, 1));
    let mut options = request(None);
    options.start_ms = boundary - 10;
    assert_eq!(page(query(&db, &options).unwrap()).total + summary[0].count, 3);
}

#[test]
fn compaction_marks_every_day_of_a_conflicting_call_partial() {
    let mut db = database();
    let first_ms = NOW - 71 * 24 * 60 * 60 * 1000;
    let later_ms = first_ms + 24 * 60 * 60 * 1000;
    let first_day = DateTime::from_timestamp_millis(first_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    let later_day = DateTime::from_timestamp_millis(later_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    commit(
        &mut db,
        b"a",
        false,
        &[call(SESSION, "conflict", first_ms, 0, "call")],
    );
    commit(
        &mut db,
        b"b",
        false,
        &[call(SESSION, "conflict", later_ms, 0, "call")],
    );

    let tx = db.transaction().unwrap();
    assert_eq!(compact_completed(&tx, NOW).unwrap(), 2);
    tx.commit().unwrap();
    let summaries = daily_summary(&db, first_day, later_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].day, first_day.to_string());
    assert_eq!(summaries[0].count, 1);
    assert!(summaries[0].partial);
    assert_eq!(summaries[1].day, later_day.to_string());
    assert_eq!(summaries[1].count, 0);
    assert!(summaries[1].partial);

    db.execute("UPDATE execution_daily SET partial = 0", []).unwrap();
    commit(&mut db, b"b", true, &[]);
    let summaries = daily_summary(&db, first_day, later_day.succ_opt().unwrap(), None).unwrap();
    assert!(summaries.iter().all(|summary| summary.partial));
    assert_eq!(summaries.iter().map(|summary| summary.count).sum::<u64>(), 1);
}

#[test]
fn late_old_duplicate_on_another_day_does_not_double_count_a_sealed_session() {
    let mut db = database();
    let first_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let later_ms = first_ms + 24 * 60 * 60 * 1000;
    let first_day = DateTime::from_timestamp_millis(first_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    let later_day = DateTime::from_timestamp_millis(later_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    commit(
        &mut db,
        b"a",
        false,
        &[call(SESSION, "shared", first_ms, 0, "call")],
    );
    let tx = db.transaction().unwrap();
    assert_eq!(compact_completed(&tx, NOW).unwrap(), 1);
    tx.commit().unwrap();
    commit(
        &mut db,
        b"b",
        false,
        &[call(SESSION, "shared", later_ms, 0, "call")],
    );
    let summaries = daily_summary(&db, first_day, later_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries.iter().map(|item| item.count).sum::<u64>(), 1);
    assert!(summaries.iter().all(|item| item.partial));
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    commit(
        &mut db,
        b"b",
        false,
        &[call(
            SESSION,
            "recent",
            NOW - 58 * 24 * 60 * 60 * 1000,
            10,
            "call",
        )],
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'recent'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn sixty_day_compaction_waits_for_duplicate_sources_and_preserves_late_gaps() {
    let mut db = database();
    let old_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let old_day = DateTime::from_timestamp_millis(old_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    let recent_ms = NOW - 58 * 24 * 60 * 60 * 1000;
    let old = call(SESSION, "old", old_ms, 0, "old");
    commit(
        &mut db,
        b"a",
        false,
        &[
            old.clone(),
            call(SESSION, "recent", recent_ms, 10, "recent"),
        ],
    );
    commit(&mut db, b"b", false, &[old]);
    {
        let tx = db.transaction().unwrap();
        tx.execute(
            "UPDATE usage_sources SET complete = 0 WHERE source = ?1",
            [b"a".as_slice()],
        )
        .unwrap();
        assert_eq!(compact_completed(&tx, NOW).unwrap(), 0);
        tx.rollback().unwrap();
    }
    let tx = db.transaction().unwrap();
    assert_eq!(compact_completed(&tx, NOW).unwrap(), 2);
    tx.commit().unwrap();
    let summaries = daily_summary(&db, old_day, old_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].count, 1);
    assert!(!summaries[0].partial);
    let retained: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'recent'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, 1);

    commit(
        &mut db,
        b"a",
        false,
        &[call(SESSION, "late", old_ms + 1, 50, "late")],
    );
    let summaries = daily_summary(&db, old_day, old_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summaries[0].count, 1);
    assert!(summaries[0].partial);
    let late: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'late'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(late, 0);
}

#[test]
fn sixty_day_compacted_source_replacement_marks_existing_summary_partial() {
    let mut db = database();
    let old_ms = NOW - 70 * 24 * 60 * 60 * 1000;
    let old_day = DateTime::from_timestamp_millis(old_ms)
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    commit(
        &mut db,
        b"a",
        false,
        &[call(SESSION, "old", old_ms, 0, "old")],
    );
    let tx = db.transaction().unwrap();
    compact_completed(&tx, NOW).unwrap();
    tx.commit().unwrap();
    commit(&mut db, b"a", true, &[]);
    let summaries = daily_summary(&db, old_day, old_day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summaries[0].count, 1);
    assert!(summaries[0].partial);
}
