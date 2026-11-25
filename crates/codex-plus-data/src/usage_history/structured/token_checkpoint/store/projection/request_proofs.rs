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
         AND json_extract(CAST(payload AS TEXT), '$.provider_source') = 'request-log-exact'",
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
                && proof.provider_source.as_deref() == Some("request-log-exact")
                && !proof.count_uncertain
                && !collisions.contains(&native.original_id)
            {
                proofs.push((i, at, origin, proof));
            }
        }
        changes.extend(incoming.into_values().map(|(at, _)| at));
        for (i, at, origin, proof) in proofs {
            let window = at.saturating_sub(radius)..=at.saturating_add(radius);
            if changes.range(window).next().is_none() {
                proof.apply(&mut events[i]);
                retained.insert(events[i].id, origin);
            }
        }
    }
    Ok(retained)
}
