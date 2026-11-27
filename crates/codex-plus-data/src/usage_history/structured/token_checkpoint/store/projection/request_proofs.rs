use super::*;

pub(super) fn retain(
    db: &Connection,
    cutoff: &str,
    events: &mut [UsageEvent],
    collisions: &HashSet<[u8; 32]>,
    receipts: &HashMap<[u8; 32], Counter>,
) -> Result<HashMap<[u8; 32], EventClassification>> {
    let mut sessions = BTreeMap::<String, Vec<usize>>::new();
    for (i, event) in events.iter().enumerate() {
        sessions.entry(event.session.clone()).or_default().push(i);
    }
    let mut bounds = db.prepare_cached(
        "SELECT min(timestamp_ms), max(timestamp_ms) FROM token_resolved INDEXED BY token_classified_time
         WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1) AND day >= ?2
         AND json_type(CAST(payload AS TEXT), '$.classification_origin') = 'object'
         AND json_extract(CAST(payload AS TEXT), '$.provider_source') IN
             ('request-log-exact','request-log-recovered-supplier-only','session-context-inferred','model-namespace')",
    )?;
    let mut previous = db.prepare_cached(
        "SELECT event_id, timestamp_ms, payload FROM token_resolved
         WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1) AND day >= ?2
         AND timestamp_ms BETWEEN ?3 AND ?4",
    )?;
    let mut retained = HashMap::new();
    // 同一请求的两个竞争事件相距最多两个延迟匹配窗口。
    let radius = 2 * DELAYED_REQUEST_MATCH_WINDOW_MS;
    for (session, indices) in sessions {
        if indices.iter().any(|&i| events[i].count_incomplete) {
            continue;
        }
        let (Some(first), Some(last)): (Option<i64>, Option<i64>) = bounds
            .query_row(params![session, cutoff], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
        else {
            continue;
        };
        let start = first.saturating_sub(radius);
        let end = last.saturating_add(radius);
        let mut incoming = HashMap::new();
        for i in indices {
            let event = &events[i];
            let at = DateTime::parse_from_rfc3339(&event.timestamp)?.timestamp_millis();
            if (start..=end).contains(&at) {
                incoming.insert(event.id, (at, i));
            }
        }
        let mut changes = BTreeSet::new();
        let mut proofs = Vec::new();
        let mut rows = previous.query(params![session, cutoff, start, end])?;
        while let Some(row) = rows.next()? {
            let id = row.get_ref(0)?.as_blob()?.try_into()?;
            let at: i64 = row.get(1)?;
            let Some((next_at, i)) = incoming.remove(&id) else {
                changes.insert(at);
                continue;
            };
            let mut saved: Observation = serde_json::from_slice(row.get_ref(2)?.as_blob()?)?;
            let origin = saved.classification_origin.take();
            let receipt = saved.receipt_count_origin.map(|counter| counter.0);
            let mut native = saved.into_event(id, session.clone())?;
            let proof = EventClassification::of(&native);
            if let Some(origin) = &origin {
                origin.apply(&mut native);
            }
            if native != events[i] || receipt != receipts.get(&id).map(|counter| counter.0) {
                changes.extend([at, next_at]);
            } else if let Some(origin) = origin
                && matches!(proof.provider_source.as_deref(),
                    Some("request-log-exact" | "request-log-recovered-supplier-only" | "session-context-inferred" | "model-namespace"))
                && !proof.count_uncertain
                && !collisions.contains(&native.original_id)
            {
                proofs.push((i, at, origin, proof));
            }
        }
        let mut appended_times = BTreeSet::new();
        let mut appended_counts = BTreeSet::new();
        for (at, i) in incoming.into_values() {
            let event = &events[i];
            appended_times.insert(at);
            if event.count_uncertain
                || event.count_pending
                || !event.delta.cached_known
                || event.context_conflicts.any()
            {
                changes.insert(at);
            } else {
                appended_counts.insert((
                    event.delta.input_tokens,
                    event.delta.output_tokens,
                    at,
                    event.delta.cached_input_tokens,
                ));
            }
        }
        for (i, at, origin, proof) in proofs {
            let window = at.saturating_sub(radius)..=at.saturating_add(radius);
            // 供应商终态证明不依赖缓存计数，输入或输出不同的完整新增不会与它竞争。
            let competes = match proof.provider_source.as_deref() {
                Some(source @ ("request-log-exact" | "request-log-recovered-supplier-only")) => {
                    let delta = events[i].delta;
                    appended_counts.range(
                        (delta.input_tokens, delta.output_tokens, *window.start(), 0)
                        ..=(delta.input_tokens, delta.output_tokens, *window.end(), u64::MAX)
                    ).any(|&(_, _, _, cached)| {
                        source == "request-log-recovered-supplier-only"
                            || cached == delta.cached_input_tokens
                    })
                }
                _ => appended_times.range(window.clone()).next().is_some(),
            };
            if !competes && changes.range(window).next().is_none() {
                // 仅跨计数重算保留未变的归属；下一轮仍从 origin 重新核对请求证据。
                proof.apply(&mut events[i]);
                retained.insert(events[i].id, origin);
            }
        }
    }
    Ok(retained)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_history::structured::index_budget::Budget;
    use crate::usage_history::structured::token_checkpoint::ingest::SourceReader;
    use std::io::Write;

    #[test]
    fn reprojected_supplier_proofs_keep_their_kind_and_remain_revocable() -> Result<()> {
        const SESSION: &str = "00000000-0000-4000-8000-000000000071";
        for evidence in [
            "request-log-exact",
            "request-log-recovered-supplier-only",
            "session-context-inferred",
            "model-namespace",
        ] {
            let dir = tempfile::tempdir()?;
            let root = dir.path().canonicalize()?;
            let source = root.join("rollout-counts.jsonl");
            let peer = root.join("rollout-peer.jsonl");
            let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
            let names = BTreeMap::new();
            let limit = 1 << 20;
            let mut store = Store::open(Connection::open_in_memory()?)?;
            let usage = |n| json!({"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n});
            let counter = |at: &str, n, cached: bool| {
                let mut row = json!({"type":"event_msg","timestamp":at,"payload":{"type":"token_count",
                    "info":{"last_token_usage":usage(1),"total_token_usage":usage(n)}}});
                if !cached {
                    for key in ["last_token_usage", "total_token_usage"] {
                        row["payload"]["info"][key].as_object_mut().unwrap().remove("cached_input_tokens");
                    }
                }
                row
            };
            let receipt = |id| {
                json!({"type":"token_usage_record","timestamp":"2026-01-02T15:00:01Z",
                "payload":{"thread_id":SESSION,"turn_id":"same","response_id":id,
                    "usage":usage(1),"thread_token_usage":usage(2)}})
            };
            let header = json!({"type":"session_meta","timestamp":"2026-01-02T12:00:00Z",
                "payload":{"id":SESSION,"model_provider":"custom"}});
            for (path, rows) in [
                (&source, vec![
                    header.clone(),
                    json!({"type":"turn_context","timestamp":"2026-01-02T12:00:00Z",
                        "payload":{"turn_id":"same","model":"fixture"}}),
                    counter("2026-01-02T12:00:01Z", 1, false),
                    counter("2026-01-02T15:00:00Z", 2, false),
                    receipt("claim-one"),
                    counter("2026-01-02T18:00:00Z", 3, true),
                ]),
                (&peer, vec![header]),
            ] {
                std::fs::write(path, rows.iter().map(|row| format!("{row}\n")).collect::<String>())?;
                let mut state = FileState::default();
                assert!(state.read_with_budget(path, cutoff, &mut Budget::new(std::iter::empty()))?);
                let prepared = store.prepare(SourceUpdate {
                    source: path, state: &state, previous_revision: None, replace: false,
                }, cutoff, names.clone(), limit)?;
                store.publish(prepared)?;
            }
            store.record_coverage(Coverage {
                pending_sources: 0, issues: 0, maintenance_blocked: false,
            }, false)?;
            assert!(store.reclassify_session(SESSION, limit, &BTreeSet::new(), |events, _| {
                for event in events {
                    event.provider = "supplier-a".into();
                    event.provider_source = Some(evidence.into());
                }
                Ok(())
            })?.is_some());
            let inspect = |store: &mut Store| -> Result<Vec<UsageEvent>> {
                let mut rows = Vec::new();
                Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
                    rows.push(event);
                    Ok(())
                })?;
                Ok(rows)
            };
            let before = inspect(&mut store)?;
            assert_eq!(before.len(), 3);
            assert!(!before[0].delta.cached_known);
            assert!(before[1].delta.cached_known);
            let mut reader = SourceReader::restore(&store, &source)?;
            let mut append = |row: &Value, store: &mut Store| -> Result<Vec<UsageEvent>> {
                writeln!(std::fs::OpenOptions::new().append(true).open(&source)?, "{row}")?;
                assert!(reader.step(store, cutoff, &mut Budget::new(std::iter::empty()), &names, limit)?);
                inspect(store)
            };
            let mut distinct = counter("2026-01-02T18:01:00Z", 4, true);
            distinct["payload"]["info"]["total_token_usage"]["input_tokens"] = json!(450);
            distinct["payload"]["info"]["last_token_usage"]["input_tokens"] = json!(150);
            let nearby = append(&distinct, &mut store)?;
            assert!(nearby[..2] == before[..2], "{evidence}");
            assert_eq!(nearby[2].id, before[2].id);
            if matches!(evidence, "request-log-exact" | "request-log-recovered-supplier-only") {
                assert!(nearby[2] == before[2], "{evidence}: distinct counts cannot compete");
                let mut competitor = counter("2026-01-02T18:04:00Z", 5, true);
                competitor["payload"]["info"]["total_token_usage"]["input_tokens"] = json!(550);
                if evidence == "request-log-recovered-supplier-only" {
                    competitor["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(101);
                    competitor["payload"]["info"]["last_token_usage"]["cached_input_tokens"] = json!(21);
                }
                let competing = append(&competitor, &mut store)?;
                assert_eq!(competing[2].id, before[2].id);
                assert_eq!(competing[2].provider_source, None, "equal counts must revalidate");
            } else {
                assert_eq!(nearby[2].provider_source, None, "{evidence}");
            }
            let corrected = append(&receipt("claim-two"), &mut store)?;
            assert!(corrected[0] == before[0], "{evidence}");
            assert_eq!(corrected[1].id, before[1].id);
            assert_eq!(corrected[1].delta.total_tokens, before[1].delta.total_tokens);
            assert!(!corrected[1].delta.cached_known);
            assert_eq!(corrected[1].provider_source, None, "{evidence}");

            store.record_coverage(Coverage {
                pending_sources: 0, issues: 0, maintenance_blocked: false,
            }, false)?;
            assert!(store.reclassify_session(SESSION, limit, &BTreeSet::new(), |events, _| {
                assert!(events.iter().all(|event| event.provider == "custom"));
                for event in events {
                    event.provider = "supplier-b".into();
                    event.provider_source = Some(evidence.into());
                }
                Ok(())
            })?.is_some());
            let refreshed = inspect(&mut store)?;
            assert!(refreshed.iter().all(|event|
                event.provider == "supplier-b" && event.provider_source.as_deref() == Some(evidence)));
            assert!(refreshed.iter().zip(&corrected).all(|(after, prior)|
                after.id == prior.id && after.delta == prior.delta && after.total == prior.total));
        }
        Ok(())
    }
}
