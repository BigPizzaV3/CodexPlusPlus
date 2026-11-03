//! 实时回执只补分类，不产生用量。证据必须逐字段匹配，且不能被两个回执复用。
use super::*;

const MAX_RECEIPTS: usize = 128;
const MAX_MERGED: usize = 8;
const COMPLETION_LAG_MS: i64 = 1_000;

#[cfg(test)]
mod captured_tests;

fn within_receipt_window(record: &RequestEvidence, receipt_at: i64, lookback_ms: i64) -> bool {
    // 用量通知可能先于网关完成日志到达；只接受通知前已开始的请求。
    record.started_at_ms <= receipt_at
        && (-COMPLETION_LAG_MS..=lookback_ms)
            .contains(&receipt_at.saturating_sub(record.timestamp_ms))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    id: String,
    session_id: String,
    timestamp: i64,
    before: Counts,
    after: Counts,
    #[serde(default)]
    model: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    request_ids: Vec<String>,
    #[serde(default)]
    request_signatures: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
struct Counts {
    input: u64,
    output: u64,
    cached: u64,
}

impl Receipt {
    fn delta(&self) -> Option<Counts> {
        let counts = Counts {
            input: self.after.input.checked_sub(self.before.input)?,
            output: self.after.output.checked_sub(self.before.output)?,
            cached: self.after.cached.checked_sub(self.before.cached)?,
        };
        (self.timestamp > 0
            && !self.id.is_empty()
            && self.id.len() <= 512
            && !self.session_id.is_empty()
            && self.session_id.len() <= 256
            && self.model.len() <= 256
            && self.provider.len() <= 256
            && self.request_ids.len() <= MAX_MERGED
            && self.request_signatures.len() <= MAX_MERGED
            && self
                .request_ids
                .iter()
                .all(|id| !id.is_empty() && id.len() <= 256)
            && counts.cached <= counts.input
            && counts.input.checked_add(counts.output)? > 0
            && self.before.cached <= self.before.input
            && self.after.cached <= self.after.input)
            .then_some(counts)
    }
}

fn receipt_model_matches(model: &str, actual: &str, provider: &str) -> bool {
    let model = request_base_model(model);
    model == actual
        || model
            .strip_prefix(provider)
            .and_then(|suffix| suffix.strip_prefix('/'))
            == Some(actual)
}

pub(super) fn match_captured_receipts(
    storage: &Path,
    records: &[RequestEvidence],
    aliases: &BTreeMap<String, String>,
    input: &Value,
) -> Value {
    let Some(rows) = input
        .as_array()
        .filter(|rows| !rows.is_empty() && rows.len() <= MAX_RECEIPTS)
    else {
        return json!([]);
    };
    let receipts: Vec<_> = rows
        .iter()
        .filter_map(|row| serde_json::from_value::<Receipt>(row.clone()).ok())
        .filter(|receipt| receipt.delta().is_some())
        .collect();
    if receipts.is_empty() {
        return json!([]);
    }
    let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    let Ok(cache) = provider_cache::ProviderCache::read_snapshot(storage, cutoff) else {
        return json!([]);
    };
    // 保留矛盾边的两个目标，整组别名一起失效，不能隐藏旧目标里的竞争请求。
    let resolved = RequestSessionAliases::from_pairs(cache.aliases.iter().chain(aliases.iter()));
    let mut combined_aliases = cache.aliases.clone();
    combined_aliases.extend(aliases.clone());
    for (session, target) in &resolved.canonical {
        if target.is_none() {
            combined_aliases.insert(session.clone(), String::new());
        }
    }
    let mut targets: BTreeMap<String, Vec<&Receipt>> = BTreeMap::new();
    for receipt in &receipts {
        if let Some(session) = resolved.resolve(&receipt.session_id) {
            targets.entry(session).or_default().push(receipt);
        }
    }
    let mut combined = records.to_vec();
    let mut observed: HashMap<_, _> = records
        .iter()
        .map(|row| (row.id.clone(), row.clone()))
        .collect();
    let mut incomplete_sessions = HashSet::new();
    for (session, receipts) in targets {
        let since = receipts
            .iter()
            .map(|row| row.timestamp)
            .min()
            .unwrap()
            .saturating_sub(DELAYED_REQUEST_MATCH_WINDOW_MS);
        let until = receipts
            .iter()
            .map(|row| row.timestamp)
            .max()
            .unwrap()
            .saturating_add(COMPLETION_LAG_MS);
        let mut sources = vec![session.clone()];
        sources.extend(
            resolved
                .canonical
                .keys()
                .filter(|alias| {
                    **alias != session && resolved.resolve(alias).as_ref() == Some(&session)
                })
                .cloned(),
        );
        if sources.len() > 16 {
            return json!([]);
        }
        let mut floor = 0;
        let mut captured = Vec::new();
        for source in sources {
            let Ok(batch) = cache.captured_proxy(&source, since, until) else {
                return json!([]);
            };
            if !batch.complete {
                return json!([]);
            }
            floor = floor.max(batch.floor);
            if captured.len().saturating_add(batch.rows.len()) > MAX_ROWS_CAPTURED_PROXY {
                return json!([]);
            }
            captured.extend(batch.rows);
        }
        for receipt in receipts {
            if receipt
                .timestamp
                .saturating_sub(DELAYED_REQUEST_MATCH_WINDOW_MS)
                < floor
            {
                incomplete_sessions.insert(session.clone());
            }
        }
        for row in captured.into_iter().filter(|row| row.timestamp_ms >= floor) {
            if observed.get(&row.id) == Some(&row) {
                continue;
            }
            if combined.len() >= MAX_ROWS_CAPTURED_PROXY {
                return json!([]);
            }
            observed.insert(row.id.clone(), row.clone());
            combined.push(row);
        }
    }
    let Ok(rejected) = cache.rejected_proxy_ids_read_only(&combined) else {
        return json!([]);
    };
    for row in &mut combined {
        if rejected.contains(&row.id) {
            row.success = false;
        }
    }
    // 仍由同一匹配器检查签名、计数、完整竞争和双向唯一性。
    let mut result = match_receipts(&combined, &combined_aliases, input);
    let output = result.as_array_mut().unwrap();
    let incomplete: HashSet<_> = receipts
        .iter()
        .filter(|row| {
            resolved
                .resolve(&row.session_id)
                .is_some_and(|session| incomplete_sessions.contains(&session))
        })
        .map(|row| row.id.clone())
        .collect();
    output.retain(|row| row["id"].as_str().is_none_or(|id| !incomplete.contains(id)));
    for receipt in receipts.iter().filter(|row| incomplete.contains(&row.id)) {
        let delta = receipt.delta().unwrap();
        output.push(json!({"id":receipt.id,"sessionId":receipt.session_id,
            "timestamp":receipt.timestamp,"input":delta.input,"output":delta.output,
            "cached":delta.cached,"rejected":true}));
    }
    result
}

pub(super) fn match_receipts(
    records: &[RequestEvidence],
    aliases: &BTreeMap<String, String>,
    input: &Value,
) -> Value {
    let Some(rows) = input
        .as_array()
        .filter(|rows| !rows.is_empty() && rows.len() <= MAX_RECEIPTS)
    else {
        return json!([]);
    };
    let aliases = RequestSessionAliases::new(aliases);
    let mut ids: HashMap<&str, &RequestEvidence> = HashMap::new();
    let mut conflicts = HashSet::new();
    for record in records {
        if ids.insert(&record.id, record).is_some() {
            conflicts.insert(record.id.as_str());
        }
    }
    let mut sessions: HashMap<String, Vec<&RequestEvidence>> = HashMap::new();
    for record in records {
        if let Some(session) = aliases.resolve(&record.thread_id) {
            sessions.entry(session).or_default().push(record);
        }
    }
    for records in sessions.values_mut() {
        records.sort_unstable_by_key(|record| (record.timestamp_ms, &record.id));
    }
    let receipts: Vec<_> = rows
        .iter()
        .map(|row| serde_json::from_value::<Receipt>(row.clone()).ok())
        .collect();
    let mut receipt_ids = HashMap::new();
    for receipt in receipts.iter().flatten() {
        *receipt_ids.entry(&receipt.id).or_insert(0usize) += 1;
    }
    let mut candidates: Vec<(usize, Vec<String>)> = vec![(0, Vec::new()); receipts.len()];
    let mut rejected = HashSet::new();
    let eligible: HashSet<_> = ids
        .values()
        .filter(|record| {
            !conflicts.contains(record.id.as_str())
                && !record.keys().is_empty()
                && record.attempts == Some(1)
                && record.auto_retries == Some(0)
        })
        .map(|record| record.id.as_str())
        .collect();
    // 即使一个回执本身有多个候选，也占用对应证据，避免另一回执误取其子集。
    let mut owners: HashMap<String, Option<usize>> = HashMap::new();
    let claim = |owners: &mut HashMap<String, Option<usize>>, id: &String, index| {
        owners
            .entry(id.clone())
            .and_modify(|owner| {
                if *owner != Some(index) {
                    *owner = None;
                }
            })
            .or_insert(Some(index));
    };
    for (index, receipt) in receipts.iter().enumerate() {
        let Some(receipt) = receipt else {
            continue;
        };
        let Some(delta) = receipt.delta() else {
            continue;
        };
        if !receipt.request_ids.is_empty()
            && (receipt.request_ids.iter().collect::<HashSet<_>>().len()
                != receipt.request_ids.len()
                || receipt.request_signatures.len() != receipt.request_ids.len()
                || receipt.request_ids.iter().any(|id| {
                    receipt.request_signatures.get(id).is_none_or(|value| {
                        value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                }))
        {
            rejected.insert(index);
        }
        let Some(session) = aliases.resolve(&receipt.session_id) else {
            if !receipt.request_ids.is_empty() {
                rejected.insert(index);
            }
            continue;
        };
        for id in &receipt.request_ids {
            claim(&mut owners, id, index);
            if let Some(record) = ids.get(id.as_str()) {
                if conflicts.contains(id.as_str())
                    || record.keys().is_empty()
                    || record.attempts != Some(1)
                    || record.auto_retries != Some(0)
                    || aliases.resolve(&record.thread_id).as_deref() != Some(session.as_str())
                    || record.actual_model().is_none_or(|model| {
                        !receipt_model_matches(&receipt.model, model, &record.relay_id)
                    })
                    || (!receipt.provider.is_empty() && receipt.provider != record.relay_id)
                    || !within_receipt_window(
                        record,
                        receipt.timestamp,
                        DELAYED_REQUEST_MATCH_WINDOW_MS,
                    )
                    || receipt
                        .request_signatures
                        .get(id)
                        .is_some_and(|saved| saved != &request_signature(record))
                {
                    rejected.insert(index);
                }
            }
        }
        let missing_saved_request = !receipt.request_ids.is_empty()
            && receipt
                .request_ids
                .iter()
                .any(|id| !ids.contains_key(id.as_str()));
        // 缺失字段或日志轮转不能使竞争请求消失，也不能静默保留旧证明。
        if let Some(records) = sessions.get(&session) {
            let start = records.partition_point(|record| {
                record.timestamp_ms
                    < receipt
                        .timestamp
                        .saturating_sub(DELAYED_REQUEST_MATCH_WINDOW_MS)
            });
            let end = records.partition_point(|record| {
                record.timestamp_ms <= receipt.timestamp.saturating_add(COMPLETION_LAG_MS)
            });
            for record in &records[start..end] {
                let uncertain =
                    record.actual_model().is_none() || !eligible.contains(record.id.as_str());
                let new_after_rotation =
                    missing_saved_request && !receipt.request_ids.contains(&record.id);
                if (uncertain || new_after_rotation)
                    && within_receipt_window(
                        record,
                        receipt.timestamp,
                        DELAYED_REQUEST_MATCH_WINDOW_MS,
                    )
                    && record.could_be_success()
                    && record.could_fit_counts(delta.input, delta.output, delta.cached)
                {
                    claim(&mut owners, &record.id, index);
                    rejected.insert(index);
                }
            }
        }
        // 轮转后只复核仍存在的旧证据，不允许另组同量请求替换既有证明。
        if missing_saved_request {
            continue;
        }
        let Some(records) = sessions.get(&session) else {
            continue;
        };
        let first_end = records.partition_point(|record| {
            record.timestamp_ms < receipt.timestamp.saturating_sub(REQUEST_MATCH_WINDOW_MS)
        });
        let last_end = records.partition_point(|record| {
            record.timestamp_ms <= receipt.timestamp.saturating_add(COMPLETION_LAG_MS)
        });
        for end in first_end..last_end {
            let last = records[end];
            if !within_receipt_window(last, receipt.timestamp, REQUEST_MATCH_WINDOW_MS) {
                continue;
            }
            let mut sum = Counts {
                input: 0,
                output: 0,
                cached: 0,
            };
            let mut model: Option<&str> = None;
            let mut provider: Option<&str> = None;
            let mut request_ids = Vec::new();
            for start in (end.saturating_sub(MAX_MERGED - 1)..=end).rev() {
                let record = records[start];
                if !eligible.contains(record.id.as_str())
                    || !within_receipt_window(
                        record,
                        receipt.timestamp,
                        DELAYED_REQUEST_MATCH_WINDOW_MS,
                    )
                    || (start < end && record.timestamp_ms > records[start + 1].started_at_ms)
                {
                    break;
                }
                let Some(normalized) = record.actual_model() else {
                    break;
                };
                if model.is_some_and(|previous| previous != normalized)
                    || provider.is_some_and(|previous| previous != record.relay_id)
                {
                    break;
                }
                model = Some(normalized);
                provider = Some(&record.relay_id);
                let usage = record.usage.unwrap();
                let Some(input) = sum.input.checked_add(usage.input_tokens.unwrap()) else {
                    break;
                };
                let Some(output) = sum.output.checked_add(usage.output_tokens.unwrap()) else {
                    break;
                };
                let Some(cached) = sum.cached.checked_add(usage.cached_tokens.unwrap()) else {
                    break;
                };
                sum = Counts {
                    input,
                    output,
                    cached,
                };
                request_ids.push(record.id.clone());
                if sum.input > delta.input || sum.output > delta.output || sum.cached > delta.cached
                {
                    break;
                }
                if sum == delta {
                    for id in &request_ids {
                        claim(&mut owners, id, index);
                    }
                    // 已知模型与请求不一致也保留占用，不能把证据转给另一个回执。
                    if (unknown_model(&receipt.model)
                        || codex_plus_core::aggregate_catalog::is_aggregate_model(&receipt.model)
                        || receipt_model_matches(&receipt.model, normalized, &record.relay_id))
                        && (unknown_provider(&receipt.provider)
                            || receipt.provider == record.relay_id)
                    {
                        candidates[index].0 += 1;
                        if candidates[index].0 == 1 {
                            candidates[index].1 = request_ids.clone();
                        }
                        if !receipt.request_ids.is_empty()
                            && receipt.request_ids.iter().collect::<HashSet<_>>()
                                != request_ids.iter().collect::<HashSet<_>>()
                        {
                            rejected.insert(index);
                        }
                    } else {
                        rejected.insert(index);
                    }
                }
            }
        }
    }
    let mut result = Vec::new();
    for (index, (count, candidate_ids)) in candidates.into_iter().enumerate() {
        let Some(receipt) = receipts[index].as_ref() else {
            continue;
        };
        let Some(delta) = receipt.delta() else {
            continue;
        };
        let reused = receipt
            .request_ids
            .iter()
            .chain(candidate_ids.iter())
            .any(|id| owners.get(id).is_some_and(|owner| owner.is_none()));
        let complete_proof_present = !receipt.request_ids.is_empty()
            && receipt
                .request_ids
                .iter()
                .all(|id| ids.contains_key(id.as_str()));
        if rejected.contains(&index)
            || reused
            || count > 1
            || (complete_proof_present && count != 1)
            || receipt_ids[&receipt.id] != 1
        {
            result.push(json!({"id":receipt.id,"sessionId":receipt.session_id,
                "timestamp":receipt.timestamp,"input":delta.input,"output":delta.output,
                "cached":delta.cached,"rejected":true}));
        } else if count == 1 {
            let record = ids[candidate_ids[0].as_str()];
            result.push(json!({
                "id":receipt.id,"sessionId":receipt.session_id,"timestamp":receipt.timestamp,
                "input":delta.input,"output":delta.output,"cached":delta.cached,
                "model":record.actual_model().unwrap(),
                "provider":record.relay_id,"providerName":record.relay_name,
                "requestIds":candidate_ids,
                "requestSignatures":candidate_ids.iter().map(|id|
                    (id.clone(),request_signature(ids[id.as_str()]))).collect::<BTreeMap<_,_>>()
            }));
        }
    }
    json!(result)
}

#[cfg(test)]
mod completion_lag_tests {
    use super::*;

    pub(super) fn request(
        id: &str,
        receipt_at: i64,
        lag: i64,
        input: u64,
        output: u64,
        cached: u64,
    ) -> RequestEvidence {
        serde_json::from_value(json!({
            "id": id, "threadId": "session", "model": "gpt-6.1-sol",
            "upstreamModel": "gpt-6.1-sol", "relayId": "supplier", "relayName": "Supplier",
            "success": true, "state": "success", "statusCode": 200,
            "timestampMs": receipt_at + lag, "startedAtMs": receipt_at.min(receipt_at + lag) - 20_000,
            "attempts": 1, "autoRetries": 0,
            "usage": {"inputTokens": input, "outputTokens": output, "cachedTokens": cached}
        }))
        .unwrap()
    }

    pub(super) fn receipt(id: &str, at: i64, input: u64, output: u64, cached: u64) -> Value {
        json!({"id": id, "sessionId": "session", "timestamp": at,
            "before": {"input": 0, "output": 0, "cached": 0},
            "after": {"input": input, "output": output, "cached": cached}})
    }

    pub(super) fn positive(rows: &Value) -> bool {
        rows.as_array()
            .unwrap()
            .iter()
            .any(|row| row["provider"] == "supplier" && row["rejected"] != true)
    }

    #[test]
    fn recovers_three_observed_late_completions_and_revalidates_saved_proof() {
        for (at, lag, input, output, cached) in [
            (1793563902902, 44, 218771, 528, 218624),
            (1793563819499, 97, 99111, 891, 98176),
            (1793562404501, 58, 168344, 2526, 4096),
        ] {
            let record = request("a", at, lag, input, output, cached);
            let mut row = receipt("one", at, input, output, cached);
            let proof = match_receipts(&[record.clone()], &BTreeMap::new(), &json!([row.clone()]));
            assert!(positive(&proof), "late by {lag}ms: {proof}");
            for key in ["model", "provider", "requestIds", "requestSignatures"] {
                row[key] = proof[0][key].clone();
            }
            assert!(positive(&match_receipts(
                &[record],
                &BTreeMap::new(),
                &json!([row])
            )));
        }
    }

    #[test]
    fn late_boundary_does_not_expand_old_window_or_accept_future_starts() {
        for (lag, accepted) in [
            (-30_001, false),
            (-30_000, true),
            (1_000, true),
            (1_001, false),
        ] {
            let record = request("a", 100_000, lag, 10, 2, 3);
            let rows = match_receipts(
                &[record],
                &BTreeMap::new(),
                &json!([receipt("one", 100_000, 10, 2, 3)]),
            );
            assert_eq!(positive(&rows), accepted, "lag {lag}: {rows}");
        }
        let mut future = request("a", 100_000, 100, 10, 2, 3);
        future.started_at_ms = 100_001;
        assert!(!positive(&match_receipts(
            &[future],
            &BTreeMap::new(),
            &json!([receipt("one", 100_000, 10, 2, 3)])
        )));
    }

    #[test]
    fn late_competitors_and_receipt_reuse_stay_ambiguous() {
        let a = request("a", 100_000, -10, 10, 2, 3);
        let b = request("b", 100_000, 97, 10, 2, 3);
        let row = receipt("one", 100_000, 10, 2, 3);
        assert!(!positive(&match_receipts(
            &[a.clone(), b.clone()],
            &BTreeMap::new(),
            &json!([row.clone()])
        )));
        assert!(!positive(&match_receipts(
            &[b.clone()],
            &BTreeMap::new(),
            &json!([row.clone(), receipt("two", 100_000, 10, 2, 3)])
        )));
        let proof = match_receipts(&[b.clone()], &BTreeMap::new(), &json!([row.clone()]));
        let mut saved = row;
        for key in ["model", "provider", "requestIds", "requestSignatures"] {
            saved[key] = proof[0][key].clone();
        }
        assert!(!positive(&match_receipts(
            &[a, b],
            &BTreeMap::new(),
            &json!([saved])
        )));
    }

    #[test]
    fn late_merged_proofs_still_require_sequential_nonoverlapping_requests() {
        let a = request("a", 100_000, -100, 10, 2, 3);
        let mut b = request("b", 100_000, 97, 20, 4, 6);
        b.started_at_ms = 99_950;
        let row = receipt("merged", 100_000, 30, 6, 9);
        assert!(positive(&match_receipts(
            &[a.clone(), b.clone()],
            &BTreeMap::new(),
            &json!([row.clone()])
        )));
        b.started_at_ms = 99_899;
        assert!(!positive(&match_receipts(
            &[a, b],
            &BTreeMap::new(),
            &json!([row])
        )));
    }
}

fn request_signature(record: &RequestEvidence) -> String {
    // 只散列已脱敏的匹配字段，用于日志部分轮转后复核仍存在的单条请求。
    let digest = Sha256::digest(serde_json::to_vec(record).unwrap_or_default());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
