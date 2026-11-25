use super::*;

pub(super) fn parse(
    state: &FileState,
    payload: &Value,
    timestamp: String,
    date: NaiveDate,
) -> Option<UsageEvent> {
    if state.turn.is_empty()
        || payload["thread_id"] != state.session
        || payload["turn_id"] != state.turn
    {
        return None;
    }
    let response = payload["response_id"].as_str().filter(|id| {
        !id.is_empty() && id.len() <= 256 && id.bytes().all(|c| c.is_ascii_graphic())
    })?;
    let counts = |key: &str| {
        let value = &payload[key];
        Tokens::parse(value).filter(|tokens| {
            tokens.cached_known
                && tokens
                    .input_tokens
                    .checked_add(tokens.output_tokens)
                    .is_some()
                && (!value
                    .as_object()
                    .is_some_and(|obj| obj.contains_key("total_tokens"))
                    || value["total_tokens"].as_u64() == Some(tokens.total_tokens))
        })
    };
    let last = counts("usage").filter(|tokens| tokens.total_tokens > 0)?;
    let total = counts("thread_token_usage")?;
    if !total.is_monotonic_after(last) || total.cached_input_tokens < last.cached_input_tokens {
        return None;
    }
    // The same final response has one identity across restarts and copied logs.
    let mut digest = Sha256::new();
    digest.update(b"usage-native-response-v1\0");
    digest.update(state.session.as_bytes());
    digest.update([0]);
    digest.update(response.as_bytes());
    let id = digest.finalize().into();
    Some(UsageEvent {
        native_response_id: None,
        terminal_response: Some(response.into()),
        id,
        original_id: id,
        session: state.session.clone(),
        turn: state.turn.clone(),
        model: state.model.clone(),
        timestamp,
        date,
        total,
        delta: last,
        has_file_baseline: false,
        last: Some(last),
        count_uncertain: false,
        count_incomplete: false,
        count_pending: false,
        rollout_position: None,
        provider: state.provider.clone(),
        provider_source: None,
        role: state.role.clone(),
        inherited: false,
        legacy_identity: None,
        context_conflicts: ContextConflicts {
            model: state.model_unavailable,
            ..ContextConflicts::default()
        },
    })
}

fn signature(event: &UsageEvent) -> Option<(&str, &str, u64, u64, u64, u64)> {
    let last = event.last?;
    Some((
        &event.session,
        &event.turn,
        event.total.input_tokens,
        event.total.output_tokens,
        last.input_tokens,
        last.output_tokens,
    ))
}

fn compatible(known: Tokens, received: Tokens) -> bool {
    known.input_tokens == received.input_tokens
        && known.output_tokens == received.output_tokens
        && (!known.cached_known || known.cached_input_tokens == received.cached_input_tokens)
        && known
            .reasoning_output_tokens
            .zip(received.reasoning_output_tokens)
            .is_none_or(|(a, b)| a == b)
}

fn close_in_time(a: &UsageEvent, b: &UsageEvent) -> bool {
    let at = |event: &UsageEvent| {
        DateTime::parse_from_rfc3339(&event.timestamp)
            .expect("parsed usage timestamp")
            .timestamp_millis()
    };
    at(a).abs_diff(at(b)) <= DELAYED_REQUEST_MATCH_WINDOW_MS as u64
}

pub(super) fn wholly_accounted_in_batch(raw: &[UsageEvent], projected: &[UsageEvent]) -> bool {
    if !raw.iter().any(|event| event.terminal_response.is_some()) {
        return true;
    }
    let mut counters = HashMap::new();
    for event in projected {
        if event.terminal_response.is_some() {
            return false;
        }
        if let Some(key) = signature(event) {
            counters
                .entry(key)
                .and_modify(|entry| *entry = None)
                .or_insert(Some(event));
        }
    }
    let mut responses = HashSet::new();
    raw.iter()
        .filter(|event| event.terminal_response.is_some())
        .all(|receipt| {
            responses.insert(receipt.original_id)
                && signature(receipt)
                    .and_then(|key| counters.remove(&key).flatten())
                    .is_some_and(|event| {
                        event.date == receipt.date
                            && event.total == receipt.total
                            && event.last == receipt.last
                            && event.delta == receipt.delta
                            && !event.context_conflicts.any()
                            && !event.count_uncertain
                            && !event.count_incomplete
                            && !event.count_pending
                            && close_in_time(receipt, event)
                    })
        })
}

pub(super) fn project(index: &Index, snapshot: &mut Snapshot) -> HashMap<[u8; 32], Tokens> {
    reconcile(
        index
            .files
            .values()
            .filter(|state| !state.identity_conflict)
            .flat_map(|state| &state.events),
        &mut snapshot.events,
        &index.provider_names,
    )
}

pub(super) fn unique_claims<'a>(mut receipts: impl Iterator<Item = &'a UsageEvent>) -> bool {
    let mut signatures = HashSet::new();
    receipts.all(|event| signature(event).is_some_and(|key| signatures.insert(key)))
}

pub(super) fn reconcile<'a>(
    observations: impl Iterator<Item = &'a UsageEvent>,
    events: &mut Vec<UsageEvent>,
    provider_names: &BTreeMap<String, String>,
) -> HashMap<[u8; 32], Tokens> {
    let mut origins = HashMap::new();
    let mut receipts = BTreeMap::<_, (UsageEvent, bool)>::new();
    for event in observations.filter(|event| event.terminal_response.is_some()) {
        receipts
            .entry(event.original_id)
            .and_modify(|(prior, conflict)| {
                *conflict |= prior.total != event.total
                    || prior.delta != event.delta
                    || prior.turn != event.turn
                    || prior.date != event.date;
                if event.timestamp < prior.timestamp {
                    *prior = event.clone();
                }
            })
            .or_insert_with(|| (event.clone(), false));
    }
    if receipts.is_empty() {
        return origins;
    }
    let mut matches = HashMap::new();
    let mut latest = HashMap::<&str, (&str, Tokens)>::new();
    for (i, event) in events.iter().enumerate() {
        if let Some(signature) = signature(event) {
            matches
                .entry(signature)
                .and_modify(|row| *row = None)
                .or_insert(Some(i));
        }
        latest
            .entry(&event.session)
            .and_modify(|(at, total)| {
                if event.timestamp.as_str() > *at {
                    (*at, *total) = (&event.timestamp, event.total);
                }
            })
            .or_insert((&event.timestamp, event.total));
    }
    let mut claims = HashMap::new();
    let mut decisions = Vec::new();
    for (event, conflict) in receipts.values() {
        let paired = signature(event).and_then(|key| matches.get(&key).copied().flatten());
        if let Some(i) = paired {
            *claims.entry(i).or_insert(0usize) += 1;
        }
        let preceding = latest.get(event.session.as_str());
        let after_last_counter = preceding.is_none_or(|(at, total)| {
            event.timestamp.as_str() > *at
                && event.total.is_monotonic_after(*total)
                && event.total.input_tokens - event.delta.input_tokens >= total.input_tokens
                && event.total.output_tokens - event.delta.output_tokens >= total.output_tokens
        });
        decisions.push((event, *conflict, paired, after_last_counter));
    }
    drop(matches);
    drop(latest);
    // A receipt is never a predecessor for a different turn's native cumulative count.
    decisions.sort_unstable_by(|(a, ..), (b, ..)| {
        (&a.session, &a.timestamp, a.id).cmp(&(&b.session, &b.timestamp, b.id))
    });
    let mut preceding_receipt = HashMap::<&str, Tokens>::new();
    for (receipt, conflict, paired, after_last_counter) in decisions {
        if let Some(i) = paired {
            let event = &mut events[i];
            if !conflict && claims[&i] == 1 {
                let native = event.delta;
                if apply_receipt_counts(event, receipt) {
                    origins.insert(event.id, native);
                }
            }
            continue;
        }
        if !after_last_counter {
            continue;
        }
        let isolated = preceding_receipt
            .get(receipt.session.as_str())
            .is_none_or(|before| {
                receipt.total.input_tokens - receipt.delta.input_tokens >= before.input_tokens
                    && receipt.total.output_tokens - receipt.delta.output_tokens
                        >= before.output_tokens
            });
        let mut event = receipt.clone();
        event.count_uncertain = conflict || !isolated;
        if !event.count_uncertain {
            preceding_receipt.insert(receipt.session.as_str(), receipt.total);
        }
        event.provider = usage_provider(&event.model, &event.provider, provider_names).to_owned();
        events.push(event);
    }
    origins
}

pub(super) fn apply_receipt_counts(event: &mut UsageEvent, receipt: &UsageEvent) -> bool {
    if signature(event) != signature(receipt)
        || event.count_uncertain
        || event.count_incomplete
        || !close_in_time(receipt, event)
        || event.context_conflicts.turn
        || !compatible(event.total, receipt.total)
        || !event
            .last
            .is_some_and(|last| compatible(last, receipt.delta))
        || !compatible(event.delta, receipt.delta)
    {
        return false;
    }
    let native = event.delta;
    event.delta.cached_input_tokens = receipt.delta.cached_input_tokens;
    event.delta.cached_known = true;
    event.delta.reasoning_output_tokens = event
        .delta
        .reasoning_output_tokens
        .or(receipt.delta.reasoning_output_tokens);
    event.delta != native
}
