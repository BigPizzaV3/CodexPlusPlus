//! 共享后台索引：正文留在后台，只向拓展提供用量和已完成工具回执。
use super::{Aggregate, CacheAccounting, Query, Record, Tokens, file_identity};
use chrono::{DateTime, Datelike, Duration as Days, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

mod index_budget;
mod live_receipts;
mod native_endpoints;
mod oversized;
mod provider_cache;
mod provider_capture;
mod segments;
mod supplier_evidence;
mod terminal_receipts;
mod token_checkpoint;
mod tool_receipts;

const RETENTION_DAYS: i64 = 365;
const MAX_ACTIVITY: usize = 12_000;
const MAX_EVENTS: usize = 100_000;
const MAX_FILES: usize = 4096;
// 单行保护与文件大小无关；所有文件均可增量读取。巨大正文不传入渲染进程。
const MAX_LINE: usize = 1024 * 1024;
const REFRESH_INTERVAL: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REQUEST_LOG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REQUEST_LOG_LINE: usize = 16 * 1024;
const MAX_REQUEST_EVIDENCE: usize = 8192;
const MAX_ROWS_CAPTURED_PROXY: usize = 20_000;
const REQUEST_MATCH_WINDOW_MS: i64 = 30_000;
const DELAYED_REQUEST_MATCH_WINDOW_MS: i64 = 120_000;
const MAX_NATIVE_RECEIPT_LINE: usize = 8192;
const REQUEST_SUPPLIER_PROOF: &str = "request-log-recovered-supplier-only";

fn observed_file_identity(path: &Path, metadata: &fs::Metadata) -> io::Result<Option<(u64, u64)>> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

        let _ = metadata;
        let file = fs::OpenOptions::new()
            .access_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)?;
        codex_plus_core::codex_sqlite::opened_file_identity(&file).map(Some)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(file_identity(metadata))
    }
}

#[derive(Clone, Default, serde::Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
struct RequestEvidence {
    id: String,
    thread_id: String,
    model: String,
    upstream_model: String,
    relay_id: String,
    relay_name: String,
    success: bool,
    state: String,
    status_code: u16,
    timestamp_ms: i64,
    started_at_ms: i64,
    attempts: Option<u32>,
    auto_retries: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    final_response_evidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    final_response_id: Option<String>,
    usage: Option<RequestEvidenceUsage>,
    #[serde(
        default,
        deserialize_with = "request_agent",
        skip_serializing_if = "String::is_empty"
    )]
    agent: String,
}

fn request_agent<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let agent = Option::<String>::deserialize(deserializer)?.unwrap_or_default();
    let agent = agent.trim();
    Ok(if agent.eq_ignore_ascii_case("codex") {
        String::new()
    } else {
        agent.to_owned()
    })
}

#[derive(Clone, Copy, Default, serde::Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
struct RequestEvidenceUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cached_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cached_tokens_known: Option<bool>,
}

#[derive(Clone, Hash, serde::Serialize, Deserialize, PartialEq, Eq)]
struct RequestMatchKey {
    session: String,
    model: String,
    input: u64,
    output: u64,
    cached: u64,
}

impl RequestMatchKey {
    fn counts_key(&self) -> Self {
        Self {
            model: String::new(),
            ..self.clone()
        }
    }
}

struct RequestSessionAliases {
    canonical: HashMap<String, Option<String>>,
}

impl RequestSessionAliases {
    fn identity(session: &str) -> String {
        super::normalize_session_id(session).unwrap_or_else(|| session.to_owned())
    }

    fn new(aliases: &BTreeMap<String, String>) -> Self {
        Self::from_pairs(aliases.iter())
    }

    fn from_pairs<'a>(aliases: impl Iterator<Item = (&'a String, &'a String)>) -> Self {
        let mut edges: HashMap<String, String> = HashMap::new();
        let mut neighbors: HashMap<String, Vec<String>> = HashMap::new();
        let mut conflicts = HashSet::new();
        for (alias, target) in aliases {
            let alias = Self::identity(alias);
            let target = Self::identity(target);
            if alias.is_empty() || target.is_empty() {
                conflicts.insert(alias.clone());
                conflicts.insert(target.clone());
            }
            if edges
                .insert(alias.clone(), target.clone())
                .is_some_and(|old| old != target)
            {
                conflicts.insert(alias.clone());
            }
            neighbors
                .entry(alias.clone())
                .or_default()
                .push(target.clone());
            neighbors.entry(target).or_default().push(alias);
        }
        let mut canonical = HashMap::new();
        for start in neighbors.keys() {
            if canonical.contains_key(start) {
                continue;
            }
            let mut pending = vec![start.clone()];
            let mut component = HashSet::new();
            let mut terminal = None;
            let mut invalid = false;
            while let Some(session) = pending.pop() {
                if !component.insert(session.clone()) {
                    continue;
                }
                invalid |= conflicts.contains(&session);
                // 自指是正常文件名与会话头一致的声明；多节点循环没有终点。
                if edges.get(&session).is_none_or(|target| target == &session) {
                    invalid |= terminal.replace(session.clone()).is_some();
                }
                pending.extend(neighbors[&session].iter().cloned());
            }
            let resolved = terminal.filter(|_| !invalid);
            for session in component {
                canonical.insert(session, resolved.clone());
            }
        }
        Self { canonical }
    }

    fn resolve(&self, session: &str) -> Option<String> {
        let session = Self::identity(session);
        if session.is_empty() {
            return None;
        }
        self.canonical
            .get(&session)
            .cloned()
            .unwrap_or(Some(session))
    }
}

fn request_base_model(model: &str) -> &str {
    if codex_plus_core::aggregate_catalog::is_aggregate_model(model) {
        model.split_once('/').map_or(model, |(_, model)| model)
    } else {
        model
    }
}

fn unknown_provider(provider: &str) -> bool {
    let provider = provider.trim();
    provider.is_empty()
        || [
            "custom",
            "codex_plus_managed",
            "codex-plus-gateway",
            "unknown",
            "null",
            "undefined",
        ]
        .iter()
        .any(|generic| provider.eq_ignore_ascii_case(generic))
}

fn unknown_model(model: &str) -> bool {
    let model = model.trim();
    if codex_plus_core::aggregate_catalog::is_aggregate_model(model) {
        return false;
    }
    model.is_empty()
        || ["unknown", "default", "null", "undefined", "auto"]
            .iter()
            .any(|placeholder| model.eq_ignore_ascii_case(placeholder))
        || model
            .split_once('/')
            .is_some_and(|(namespace, _)| namespace.eq_ignore_ascii_case("group"))
}

impl RequestEvidence {
    fn legacy_projection(&self) -> Self {
        let mut projection = self.clone();
        projection.final_response_id = None;
        if let Some(usage) = &mut projection.usage {
            usage.cached_tokens_known = None;
        }
        projection
    }

    fn known_cached(&self) -> Option<u64> {
        let usage = self.usage?;
        let cached = usage.cached_tokens?;
        usage
            .cached_tokens_known
            .unwrap_or(
                cached > 0 || self.final_response_evidence.as_deref() == Some("final-response-v1"),
            )
            .then_some(cached)
    }

    // 同一次完成请求跨格式重读：只补缺失的回执和缓存计数，不覆盖已知证据。
    fn merge_observation<'a>(&'a self, other: &Self) -> Option<std::borrow::Cow<'a, Self>> {
        use std::borrow::Cow;
        if !self.valid_capture()
            || !other.valid_capture()
            || self.id != other.id
            || self.thread_id != other.thread_id
            || self.model != other.model
            || self.upstream_model != other.upstream_model
            || self.relay_id != other.relay_id
            || self.agent != other.agent
            || self.success != other.success
            || self.state != other.state
            || self.status_code != other.status_code
            || self.timestamp_ms != other.timestamp_ms
            || self.started_at_ms != other.started_at_ms
            || self.attempts != other.attempts
            || self.auto_retries != other.auto_retries
            || self.final_response_evidence != other.final_response_evidence
            || self
                .final_response_id
                .as_ref()
                .zip(other.final_response_id.as_ref())
                .is_some_and(|(a, b)| a != b)
            || self.usage.map(|u| (u.input_tokens, u.output_tokens))
                != other.usage.map(|u| (u.input_tokens, u.output_tokens))
        {
            return None;
        }
        let (left, right) = (self.known_cached(), other.known_cached());
        if left.zip(right).is_some_and(|(a, b)| a != b) {
            return None;
        }
        let mut merged = Cow::Borrowed(self);
        if left.is_none() && right.is_some() {
            merged.to_mut().usage = other.usage;
        }
        if self.final_response_id.is_none() && other.final_response_id.is_some() {
            merged
                .to_mut()
                .final_response_id
                .clone_from(&other.final_response_id);
        }
        Some(merged)
    }

    fn provider_name(&self) -> &str {
        if self.relay_name.trim().is_empty() {
            &self.relay_id
        } else {
            &self.relay_name
        }
    }

    fn could_be_success(&self) -> bool {
        !self.thread_id.is_empty()
            && self.agent.is_empty()
            && self.success
            && self.state == "success"
            && (200..300).contains(&self.status_code)
            && self.timestamp_ms > 0
    }

    fn could_fit_counts(&self, input: u64, output: u64, cached: u64) -> bool {
        self.usage.is_none_or(|usage| {
            usage.input_tokens.is_none_or(|count| count <= input)
                && usage.output_tokens.is_none_or(|count| count <= output)
                && self.known_cached().is_none_or(|count| count <= cached)
        })
    }

    fn valid_request(&self) -> bool {
        let strings = [
            &self.id,
            &self.thread_id,
            &self.model,
            &self.upstream_model,
            &self.relay_id,
            &self.relay_name,
        ];
        if strings.iter().any(|s| s.len() > 256)
            || self.id.is_empty()
            || self.thread_id.is_empty()
            || !self.agent.is_empty()
            || !self.success
            || self.state != "success"
            || !(200..300).contains(&self.status_code)
            || self
                .attempts
                .zip(self.auto_retries)
                .is_none_or(|(attempts, retries)| attempts == 0 || retries >= attempts)
            || self.started_at_ms <= 0
            || self.timestamp_ms < self.started_at_ms
        {
            return false;
        }
        true
    }

    fn valid_capture(&self) -> bool {
        if !self.valid_request() {
            return false;
        }
        let Some(RequestEvidenceUsage {
            input_tokens: Some(input),
            output_tokens: Some(output),
            cached_tokens,
            cached_tokens_known,
        }) = self.usage
        else {
            return false;
        };
        self.actual_model().is_some()
            && self
                .final_response_id
                .as_ref()
                .is_none_or(|id| !id.is_empty() && id.len() <= 256)
            && cached_tokens.is_none_or(|cached| cached <= input)
            && !(cached_tokens_known == Some(true) && cached_tokens.is_none())
            && !(cached_tokens_known == Some(false) && cached_tokens.is_some_and(|n| n > 0))
            && input.saturating_add(output) > 0
    }

    fn valid_request_counts(&self) -> bool {
        self.valid_capture() && self.known_cached().is_some()
    }

    fn keys(&self) -> Vec<RequestMatchKey> {
        self.keys_with_cached(self.known_cached())
    }

    fn keys_with_cached(&self, cached: Option<u64>) -> Vec<RequestMatchKey> {
        let Some(cached) = cached.filter(|_| self.valid_capture()) else {
            return Vec::new();
        };
        let RequestEvidenceUsage {
            input_tokens: Some(input),
            output_tokens: Some(output),
            ..
        } = self.usage.unwrap()
        else {
            unreachable!("validated request counts")
        };
        let mut models: Vec<_> = [&self.model, &self.upstream_model]
            .into_iter()
            .filter(|model| !unknown_model(model))
            .map(String::as_str)
            .collect();
        models.dedup();
        models.push("");
        models
            .into_iter()
            .map(|model| RequestMatchKey {
                session: self.thread_id.clone(),
                model: model.to_owned(),
                input,
                output,
                cached,
            })
            .collect()
    }

    fn actual_model(&self) -> Option<&str> {
        let model = self.upstream_model.as_str();
        (!unknown_model(model) && !unknown_provider(&self.relay_id)).then_some(model)
    }
}

#[derive(Clone, serde::Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct VerifiedAttribution {
    key: RequestMatchKey,
    at: i64,
    request: RequestEvidence,
    rejected: bool,
    #[serde(default, skip_serializing_if = "is_legacy_request_proof")]
    version: u8,
}

fn is_legacy_request_proof(version: &u8) -> bool {
    *version == 0
}

#[derive(Default)]
struct RequestEvidenceCache {
    collision_families: HashSet<[u8; 32]>,
    path: PathBuf,
    stamp: Option<(u64, Option<SystemTime>, Option<(u64, u64)>)>,
    records: Vec<RequestEvidence>,
    captured_start: Option<usize>,
    captured_cursor: usize,
    captured_complete: Option<HashSet<[u8; 32]>>,
    captured_rejected: HashSet<String>,
    captured_id_check_failed: bool,
    verified: HashMap<[u8; 32], VerifiedAttribution>,
    reads: u64,
    bytes_read: u64,
}

impl RequestEvidenceCache {
    pub(super) fn restore_verified(
        &mut self,
        retained: impl IntoIterator<Item = ([u8; 32], VerifiedAttribution)>,
    ) {
        self.verified.retain(|_, saved| saved.rejected);
        for (id, saved) in retained {
            if let Some(previous) = self.verified.get_mut(&id) {
                if *previous != saved {
                    previous.rejected = true;
                }
            } else if self.verified.len() < MAX_EVENTS {
                self.verified.insert(id, saved);
            }
        }
    }

    fn reject_conflicting_event(&mut self, event: &UsageEvent) {
        if let Some(saved) = self.verified.get_mut(&event.id) {
            saved.rejected = true;
        } else if self.verified.len() < MAX_EVENTS {
            if let Ok(at) = DateTime::parse_from_rfc3339(&event.timestamp) {
                self.verified.insert(
                    event.id,
                    VerifiedAttribution {
                        key: RequestMatchKey {
                            session: String::new(),
                            model: String::new(),
                            input: 0,
                            output: 0,
                            cached: 0,
                        },
                        at: at.timestamp_millis(),
                        request: RequestEvidence::default(),
                        rejected: true,
                        version: 1,
                    },
                );
            }
        }
    }

    fn supplement_missing_session_models(
        &self,
        sessions: &mut BTreeMap<String, Value>,
        unavailable: &HashSet<String>,
        aliases: &RequestSessionAliases,
        now: i64,
    ) {
        let mut models: BTreeMap<&str, Option<&str>> = BTreeMap::new();
        let mut valid = BTreeMap::new();
        let mut ids = HashSet::new();
        let mut blocked = HashSet::new();
        for record in &self.records {
            if !ids.insert(record.id.as_str()) || record.keys().is_empty() {
                blocked.insert(record.id.as_str());
            }
        }
        for record in &self.records {
            if super::normalize_session_id(&record.thread_id).is_none()
                || aliases
                    .resolve(&record.thread_id)
                    .is_none_or(|session| unavailable.contains(&session))
                || sessions
                    .get(&record.thread_id)
                    .and_then(|info| info["model"].as_str())
                    .is_some_and(|model| !unknown_model(model))
            {
                continue;
            }
            let model = record.model.as_str();
            let entry = models.entry(&record.thread_id).or_insert(Some(model));
            if unknown_model(model) || *entry != Some(model) {
                *entry = None;
            }
            if !blocked.contains(record.id.as_str())
                && record.timestamp_ms <= now
                && now.saturating_sub(record.timestamp_ms) <= 10 * 60 * 1000
            {
                let latest = valid.entry(record.thread_id.as_str()).or_insert(0);
                *latest = (*latest).max(record.timestamp_ms);
            }
        }
        let mut recent: Vec<_> = valid
            .into_iter()
            .filter(|(session, _)| models.get(session).copied().flatten().is_some())
            .collect();
        recent.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        for (session, _) in recent.into_iter().take(16) {
            let Some(model) = models.get(session).copied().flatten() else {
                continue;
            };
            let model = request_base_model(model);
            if unknown_model(model) {
                continue;
            }
            // 仅补缺失模型，保留已落盘的角色/供应商；不产生用量或供应商推断。
            let info = sessions
                .entry(session.to_owned())
                .or_insert_with(|| json!({}));
            info["model"] = json!(model);
            info["modelSource"] = json!("request-log-single-model");
        }
    }

    fn refresh(&mut self, path: &Path) {
        if let Some(start) = self.captured_start.take() {
            self.records.truncate(start);
        }
        if self.path != path {
            // 首次绑定日志路径时保留刚恢复的事务快照；切换日志来源仍清空旧缓存。
            let verified = if self.path.as_os_str().is_empty() {
                std::mem::take(&mut self.verified)
            } else {
                HashMap::new()
            };
            *self = Self {
                path: path.to_owned(),
                verified,
                ..Self::default()
            };
        }
        self.bytes_read = 0;
        let Ok(guard) = codex_plus_core::request_log::lock_request_log() else {
            self.records.clear();
            self.stamp = None;
            return;
        };
        let metadata = fs::symlink_metadata(path).ok().filter(|m| m.is_file());
        let stamp = metadata
            .as_ref()
            .map(|m| (m.len(), m.modified().ok(), file_identity(m)));
        if self.stamp == stamp {
            return;
        }
        self.stamp = stamp;
        self.records.clear();
        let Some(metadata) = metadata.filter(|m| m.len() <= MAX_REQUEST_LOG_BYTES) else {
            return;
        };
        let Ok(mut file) = File::open(path) else {
            self.stamp = None;
            return;
        };
        if file
            .metadata()
            .ok()
            .is_none_or(|m| file_identity(&m) != file_identity(&metadata))
        {
            self.stamp = None;
            return;
        }
        self.reads += 1;
        let mut contents = Vec::with_capacity(metadata.len() as usize);
        let read_result = (&mut file).take(metadata.len()).read_to_end(&mut contents);
        drop(guard);
        if read_result.is_err() {
            self.stamp = None;
            return;
        }
        // 总量和单行均有界；serde 跳过未声明字段，不分配 error/正文字符串。
        let mut reader = BufReader::new(contents.as_slice());
        let mut line = Vec::new();
        let mut oversized = false;
        let result = (|| -> io::Result<()> {
            loop {
                let buffer = reader.fill_buf()?;
                if buffer.is_empty() {
                    break;
                }
                let newline = buffer.iter().position(|b| *b == b'\n');
                let length = newline.map_or(buffer.len(), |i| i + 1);
                if !oversized {
                    if line.len() + length > MAX_REQUEST_LOG_LINE {
                        line.clear();
                        oversized = true;
                    } else {
                        line.extend_from_slice(&buffer[..length]);
                    }
                }
                reader.consume(length);
                self.bytes_read += length as u64;
                if newline.is_some() {
                    if !oversized {
                        #[derive(Deserialize)]
                        struct Identity {
                            id: String,
                        }
                        let record = serde_json::from_slice::<RequestEvidence>(&line)
                            .ok()
                            .or_else(|| {
                                // 字段类型损坏但身份仍明确时，撤销该 ID 的旧补证，不能当作轮转缺失。
                                serde_json::from_slice::<Identity>(&line)
                                    .ok()
                                    .map(|identity| RequestEvidence {
                                        id: identity.id,
                                        ..Default::default()
                                    })
                            });
                        if let Some(record) = record {
                            if self.records.len() == MAX_REQUEST_EVIDENCE {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "request evidence limit",
                                ));
                            }
                            self.records.push(record);
                        }
                    }
                    line.clear();
                    oversized = false;
                }
            }
            Ok(())
        })();
        // 未换行的尾记录不接受；读取期间被替换或追加时，下轮重新投影。
        let after = fs::symlink_metadata(path)
            .ok()
            .map(|m| (m.len(), m.modified().ok(), file_identity(&m)));
        if let Err(error) = result {
            self.records.clear();
            if error.kind() != io::ErrorKind::InvalidData {
                self.stamp = None;
            }
        }
        if after != stamp {
            self.records.clear();
            self.stamp = None;
        }
    }

    fn supplement_captured(
        &mut self,
        events: &[UsageEvent],
        aliases: &BTreeMap<String, String>,
        cache: &provider_cache::ProviderCache,
    ) -> (usize, bool) {
        if let Some(start) = self.captured_start.take() {
            self.records.truncate(start);
        }
        self.captured_complete = Some(HashSet::new());
        let aliases = RequestSessionAliases::new(aliases);
        let mut source_aliases: HashMap<String, Vec<String>> = HashMap::new();
        for alias in aliases.canonical.keys() {
            if let Some(target) = aliases.resolve(alias) {
                if &target != alias {
                    source_aliases
                        .entry(target)
                        .or_default()
                        .push(alias.clone());
                }
            }
        }
        let mut targets: BTreeMap<String, Vec<(i64, [u8; 32])>> = BTreeMap::new();
        for event in events.iter().filter(|e| {
            (unknown_provider(&e.provider)
                || unknown_model(&e.model)
                || e.native_response_id
                    .as_ref()
                    .is_some_and(|proof| !proof.full_counts)
                || codex_plus_core::aggregate_catalog::is_aggregate_model(&e.model))
                && !e.context_conflicts.any()
                && !e.count_uncertain
                && !e.count_incomplete
                && (e.delta.cached_known || e.native_response_id.is_some())
        }) {
            let Some(session) = aliases.resolve(&event.session) else {
                continue;
            };
            let Ok(at) = DateTime::parse_from_rfc3339(&event.timestamp) else {
                continue;
            };
            let at = at.timestamp_millis();
            targets.entry(session).or_default().push((at, event.id));
        }
        if targets.is_empty() {
            return (0, false);
        }
        // refresh 每轮丢弃旧补充；文件 stamp 不承担完成缓存的更新通知。
        self.captured_start = Some(self.records.len());
        let mut observed: HashMap<_, _> = self
            .records
            .iter()
            .enumerate()
            .map(|(index, r)| (r.id.clone(), index))
            .collect();
        let mut recovered = 0;
        let mut limited = targets.len() > 256;
        let targets: Vec<_> = targets.into_iter().collect();
        let offset = self.captured_cursor % targets.len();
        let count = targets.len().min(256);
        self.captured_cursor = offset + count;
        for (session, events) in targets.iter().cycle().skip(offset).take(count) {
            let since = events
                .iter()
                .map(|(at, _)| *at)
                .min()
                .unwrap()
                .saturating_sub(DELAYED_REQUEST_MATCH_WINDOW_MS);
            let until = events
                .iter()
                .map(|(at, _)| *at)
                .max()
                .unwrap()
                .saturating_add(DELAYED_REQUEST_MATCH_WINDOW_MS);
            let mut sources = source_aliases.remove(session).unwrap_or_default();
            sources.push(session.clone());
            sources.sort();
            if sources.len() > 16 {
                limited = true;
                continue;
            }
            let mut rows = Vec::new();
            let mut complete = true;
            let mut floor = 0;
            for source in sources {
                match cache.captured_proxy(&source, since, until) {
                    Ok(batch) => {
                        complete &= batch.complete;
                        floor = floor.max(batch.floor);
                        rows.extend(batch.rows);
                    }
                    Err(_) => {
                        complete = false;
                        break;
                    }
                }
            }
            // 一个别名的截断边界同样约束整组；不让更早的部分窗口推断唯一性。
            rows.retain(|row| row.timestamp_ms >= floor);
            let additional = rows
                .iter()
                .filter(|row| {
                    !observed
                        .get(&row.id)
                        .is_some_and(|&index| self.records[index].merge_observation(row).is_some())
                })
                .count();
            if self.records.len().saturating_add(additional) > MAX_ROWS_CAPTURED_PROXY {
                limited = true;
                continue;
            }
            for row in rows {
                if let Some(&index) = observed.get(&row.id)
                    && let Some(merged) = self.records[index].merge_observation(&row)
                {
                    if let std::borrow::Cow::Owned(merged) = merged {
                        self.records[index] = merged;
                    }
                    continue;
                }
                observed.insert(row.id.clone(), self.records.len());
                self.records.push(row);
                recovered += 1;
            }
            for (at, id) in events {
                if complete && at.saturating_sub(DELAYED_REQUEST_MATCH_WINDOW_MS) >= floor {
                    self.captured_complete.as_mut().unwrap().insert(*id);
                } else {
                    limited = true;
                }
            }
        }
        (recovered, limited)
    }

    fn attribute_with_cache(
        &mut self,
        events: &mut [UsageEvent],
        names: &mut BTreeMap<String, String>,
        aliases: &BTreeMap<String, String>,
        cache: &provider_cache::ProviderCache,
    ) -> Value {
        let checked = cache.rejected_proxy_ids(
            self.records
                .iter()
                .chain(self.verified.values().map(|saved| &saved.request)),
        );
        self.captured_id_check_failed = checked.is_err();
        self.captured_rejected = checked.unwrap_or_default();
        self.attribute(events, names, aliases)
    }

    fn attribute(
        &mut self,
        events: &mut [UsageEvent],
        names: &mut BTreeMap<String, String>,
        aliases: &BTreeMap<String, String>,
    ) -> Value {
        let aliases = RequestSessionAliases::new(aliases);
        let cutoff = provider_cache::ProviderCache::since(
            Local::now().date_naive() - Days::days(RETENTION_DAYS - 1),
        );
        // 暂时不可读的原文件恢复后仍需补证；不会用这些证据生成任何消费记录。
        self.verified.retain(|_, saved| saved.at >= cutoff);
        let mut requests = BTreeMap::new();
        let mut blocked = self.captured_rejected.clone();
        for record in &self.records {
            if record.id.is_empty() || record.id.len() > 256 {
                continue;
            }
            if !record.valid_capture()
                || requests.insert(record.id.clone(), record.clone()).is_some()
            {
                blocked.insert(record.id.clone());
            }
        }
        let current: HashSet<_> = requests.keys().cloned().collect();
        let mut legacy_zero_proofs = HashMap::new();
        for (event_id, saved) in &self.verified {
            // 旧版已验过的零只用于原事件；未知零不能为其他事件建立证明。
            if !saved.rejected
                && saved.key.cached == 0
                && saved.request.usage.is_some_and(|usage| {
                    usage.cached_tokens == Some(0) && usage.cached_tokens_known.is_none()
                })
                && saved.request.known_cached().is_none()
            {
                legacy_zero_proofs.insert(saved.request.id.clone(), *event_id);
            }
            if let Some(record) = requests.get_mut(&saved.request.id) {
                match record.merge_observation(&saved.request) {
                    None => {
                        blocked.insert(saved.request.id.clone());
                    }
                    Some(std::borrow::Cow::Owned(merged)) => *record = merged,
                    Some(std::borrow::Cow::Borrowed(_)) => {}
                }
            } else {
                requests.insert(saved.request.id.clone(), saved.request.clone());
            }
        }
        let cached_for_matching = |record: &RequestEvidence| {
            record
                .known_cached()
                .or_else(|| legacy_zero_proofs.contains_key(&record.id).then_some(0))
        };
        let mut uncertain_requests: HashMap<String, Vec<RequestEvidence>> = HashMap::new();
        for record in &self.records {
            if record.could_be_success()
                && (blocked.contains(&record.id)
                    || requests
                        .get(&record.id)
                        .is_none_or(|r| cached_for_matching(r).is_none()))
            {
                if let Some(session) = aliases.resolve(&record.thread_id) {
                    uncertain_requests
                        .entry(session)
                        .or_default()
                        .push(record.clone());
                }
            }
        }
        for records in uncertain_requests.values_mut() {
            records.sort_unstable_by_key(|record| record.timestamp_ms);
        }
        requests.retain(|id, _| !blocked.contains(id));
        let requests: Vec<_> = requests.into_values().collect();
        let request_keys: Vec<Vec<_>> = requests
            .iter()
            .map(|request| {
                request
                    .keys_with_cached(cached_for_matching(request))
                    .into_iter()
                    .filter_map(|mut key| {
                        key.session = aliases.resolve(&key.session)?;
                        Some(key)
                    })
                    .collect()
            })
            .collect();
        let mut request_index: HashMap<RequestMatchKey, Vec<(i64, usize)>> = HashMap::new();
        for (i, request) in requests.iter().enumerate() {
            if let Some(key) = request_keys[i].first() {
                request_index
                    .entry(key.counts_key())
                    .or_default()
                    .push((request.timestamp_ms, i));
            }
        }
        for entries in request_index.values_mut() {
            entries.sort_unstable();
        }
        let event_keys: Vec<_> = events
            .iter()
            .map(|event| {
                let at = DateTime::parse_from_rfc3339(&event.timestamp)
                    .ok()?
                    .timestamp_millis();
                if event.session.is_empty() || event.count_incomplete || !event.delta.cached_known {
                    return None;
                }
                Some((
                    RequestMatchKey {
                        session: aliases.resolve(&event.session)?,
                        model: if unknown_model(&event.model) {
                            String::new()
                        } else if self
                            .verified
                            .get(&event.id)
                            .is_some_and(|saved| saved.version == 0)
                        {
                            request_base_model(&event.model).to_owned()
                        } else {
                            event.model.clone()
                        },
                        input: event.delta.input_tokens,
                        output: event.delta.output_tokens,
                        cached: event.delta.cached_input_tokens,
                    },
                    at,
                ))
            })
            .collect();
        // 型号尚未确认、已知归属及子会话都参与竞争；一个请求只计一次。
        let mut event_index: HashMap<RequestMatchKey, Vec<i64>> = HashMap::new();
        for (key, at) in event_keys.iter().flatten() {
            event_index.entry(key.counts_key()).or_default().push(*at);
        }
        for times in event_index.values_mut() {
            times.sort_unstable();
        }
        let reverse_counts = |window: i64| {
            requests
                .iter()
                .zip(&request_keys)
                .map(|(request, keys)| {
                    keys.first()
                        .and_then(|key| event_index.get(&key.counts_key()))
                        .map(|times| {
                            times.partition_point(|at| {
                                *at <= request.timestamp_ms.saturating_add(window)
                            }) - times.partition_point(|at| {
                                *at < request.timestamp_ms.saturating_sub(window)
                            })
                        })
                        .unwrap_or(0)
                })
                .collect::<Vec<usize>>()
        };
        let delayed_reverse = reverse_counts(DELAYED_REQUEST_MATCH_WINDOW_MS);
        // 终态 ID 只能绑定一条原生用量与一条网关请求；拒绝的候选也参与查重。
        let mut repeated_response_ids = HashSet::new();
        let mut supplier_requests = HashMap::new();
        if events
            .iter()
            .any(|event| event.native_response_id.is_some())
        {
            let mut seen = HashSet::new();
            for id in events
                .iter()
                .filter_map(|event| event.native_response_id.as_ref().map(|proof| &*proof.id))
            {
                if !seen.insert(id) {
                    repeated_response_ids.insert(id.to_owned());
                }
            }
            let mut owners = HashMap::new();
            for request in self
                .records
                .iter()
                .chain(&requests)
                .chain(self.verified.values().map(|saved| &saved.request))
            {
                if let Some(id) = request.final_response_id.as_deref()
                    && seen.contains(id)
                    && owners
                        .insert(id, request.id.as_str())
                        .is_some_and(|old| old != request.id)
                {
                    repeated_response_ids.insert(id.to_owned());
                }
            }
            for request in &requests {
                if request.final_response_evidence.as_deref()
                    == Some(codex_plus_core::request_log::FINAL_RESPONSE_SUPPLIER_EVIDENCE_V1)
                    && request.known_cached().is_none()
                    && let Some(id) = request.final_response_id.as_deref()
                    && seen.contains(id)
                    && !repeated_response_ids.contains(id)
                {
                    supplier_requests.insert(id, request);
                }
            }
        }
        // 来源计数缺口可能藏有未知时间的事件；可定位的不确定事件只参与附近请求的竞争。
        let mut incomplete_sessions = HashSet::new();
        let mut uncertain_times: HashMap<String, Vec<i64>> = HashMap::new();
        for event in events.iter().filter(|event| {
            event.count_uncertain
                || event.count_incomplete
                || !event.delta.cached_known
                || event.context_conflicts.any()
        }) {
            let Some(session) = aliases.resolve(&event.session) else {
                continue;
            };
            let at = DateTime::parse_from_rfc3339(&event.timestamp)
                .ok()
                .map(|time| time.timestamp_millis());
            if event.count_incomplete || at.is_none() {
                incomplete_sessions.insert(session);
            } else if let Some(at) = at {
                uncertain_times.entry(session).or_default().push(at);
            }
        }
        for times in uncertain_times.values_mut() {
            times.sort_unstable();
        }
        let (mut recovered, mut tokens, mut retained) = (0usize, 0u64, 0usize);
        let (mut delayed_events, mut delayed_tokens) = (0usize, 0u64);
        let (mut model_events, mut direct_conflicts) = (0usize, 0usize);
        let mut supplier_only = 0usize;
        let mut missing_names = HashSet::new();
        for (event, evidence) in events.iter_mut().zip(&event_keys) {
            // 请求日志没有轮次身份，不能把旧证明复用于拆分后的碰撞事件。
            if self.collision_families.contains(&event.original_id) {
                continue;
            }
            if event.context_conflicts.any() {
                self.reject_conflicting_event(event);
                continue;
            }
            if let Some(saved) = self.verified.get_mut(&event.id) {
                if !unknown_provider(&event.provider) && event.provider != saved.request.relay_id {
                    saved.rejected = true;
                }
            }
            if !self.captured_id_check_failed
                && !event.count_uncertain
                && !event.count_incomplete
                && !self
                    .verified
                    .get(&event.id)
                    .is_some_and(|saved| saved.rejected)
                && self
                    .captured_complete
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&event.id))
                && let Some(native) = event.native_response_id.as_ref()
                && let Some(request) = supplier_requests.get(&*native.id)
                && request.thread_id == event.session
                && request.usage.is_some_and(|usage| {
                    usage.input_tokens == Some(event.delta.input_tokens)
                        && usage.output_tokens == Some(event.delta.output_tokens)
                })
                && DateTime::parse_from_rfc3339(&event.timestamp)
                    .is_ok_and(|at| request.started_at_ms <= at.timestamp_millis())
            {
                // 双端终态身份只确认实际供应商，不补造缓存计数或改写原生模型。
                if !unknown_provider(&event.provider) && event.provider != request.relay_id {
                    event.provider.clear();
                    event.context_conflicts.provider = true;
                    self.reject_conflicting_event(event);
                    direct_conflicts += 1;
                } else {
                    event.provider.clone_from(&request.relay_id);
                    event.provider_source = Some(REQUEST_SUPPLIER_PROOF.to_owned());
                    let name = names.entry(event.provider.clone()).or_default();
                    if name.trim().is_empty() {
                        *name = request.provider_name().to_owned();
                    }
                    supplier_only += 1;
                }
                continue;
            }
            if self.captured_id_check_failed
                || event.count_uncertain
                || event.count_incomplete
                || !event.delta.cached_known
                || evidence
                    .as_ref()
                    .is_some_and(|(key, _)| incomplete_sessions.contains(&key.session))
            {
                continue;
            }
            let mut uncertain_competition = false;
            let mut paired_receipt = false;
            let candidate = evidence.as_ref().and_then(|(key, at)| {
                let entries = request_index.get(&key.counts_key())?;
                let start = entries.partition_point(|(time, _)| {
                    *time < at.saturating_sub(DELAYED_REQUEST_MATCH_WINDOW_MS)
                });
                let end = entries.partition_point(|(time, _)| {
                    *time <= at.saturating_add(DELAYED_REQUEST_MATCH_WINDOW_MS)
                });
                if end - start != 1 {
                    return None;
                }
                let i = entries[start].1;
                let request = &requests[i];
                let native_id = event
                    .native_response_id
                    .as_ref()
                    .filter(|proof| proof.full_counts)
                    .map(|proof| &*proof.id);
                let final_id = request.final_response_id.as_deref();
                let terminal =
                    request.final_response_evidence.as_deref() == Some("final-response-v1");
                if terminal
                    && native_id.zip(final_id).is_some_and(|(native, final_id)| {
                        native != final_id || repeated_response_ids.contains(native)
                    })
                {
                    return None;
                }
                let paired = !event.inherited
                    && request.thread_id == event.session
                    && terminal
                    && native_id.is_some()
                    && native_id == final_id
                    && request.valid_request_counts()
                    && request.started_at_ms <= *at
                    && request.timestamp_ms.abs_diff(*at) <= REQUEST_MATCH_WINDOW_MS as u64
                    && event
                        .last
                        .is_some_and(|last| last.cached_known && last.same_counts(event.delta));
                if let Some(records) = uncertain_requests.get(&key.session) {
                    let start = records.partition_point(|record| {
                        record.timestamp_ms < at.saturating_sub(DELAYED_REQUEST_MATCH_WINDOW_MS)
                    });
                    let end = records.partition_point(|record| {
                        record.timestamp_ms <= at.saturating_add(DELAYED_REQUEST_MATCH_WINDOW_MS)
                    });
                    if records[start..end].iter().any(|record| {
                        record.could_fit_counts(key.input, key.output, key.cached)
                            && !(paired
                                && record.usage.is_none()
                                && record.final_response_id.is_none()
                                && record.valid_request()
                                && record.relay_id == request.relay_id
                                && record.actual_model() == request.actual_model()
                                && record.model == request.model)
                    }) {
                        return None;
                    }
                }
                if request.known_cached().is_none()
                    && legacy_zero_proofs.get(&request.id) != Some(&event.id)
                {
                    return None;
                }
                // 以请求时间为中心，与反向唯一性使用同一窗口。
                if let Some(times) = uncertain_times.get(&key.session) {
                    let first = times.partition_point(|at| {
                        *at < request
                            .timestamp_ms
                            .saturating_sub(DELAYED_REQUEST_MATCH_WINDOW_MS)
                    });
                    if times.get(first).is_some_and(|at| {
                        *at <= request
                            .timestamp_ms
                            .saturating_add(DELAYED_REQUEST_MATCH_WINDOW_MS)
                    }) {
                        uncertain_competition = true;
                        return None;
                    }
                }
                if delayed_reverse[i] != 1 {
                    return None;
                }
                if request.timestamp_ms.abs_diff(*at) <= REQUEST_MATCH_WINDOW_MS as u64 {
                    paired_receipt = paired;
                    return Some((i, false));
                }
                // 延迟补认只接受请求结束之后且没有重试的记录。
                (request.timestamp_ms < *at
                    && request.attempts == Some(1)
                    && request.auto_retries == Some(0))
                .then_some((i, true))
            });
            if let Some(saved) = self.verified.get_mut(&event.id) {
                let mut saved_key = saved.key.clone();
                if unknown_model(&saved_key.model) {
                    saved_key.model.clear();
                } else if saved.version == 0 {
                    saved_key.model = request_base_model(&saved_key.model).to_owned();
                }
                let saved_session = aliases.resolve(&saved_key.session);
                if let Some(session) = &saved_session {
                    saved_key.session.clone_from(session);
                }
                if saved_session.is_none()
                    || evidence
                        .as_ref()
                        .is_none_or(|(key, at)| *key != saved_key || *at != saved.at)
                    || blocked.contains(&saved.request.id)
                    || !uncertain_competition
                        && candidate.is_none_or(|(i, _)| {
                            requests[i].merge_observation(&saved.request).is_none()
                        })
                {
                    saved.rejected = true;
                } else if let Some((i, _)) = candidate
                    && let Some(std::borrow::Cow::Owned(merged)) =
                        saved.request.merge_observation(&requests[i])
                {
                    saved.request = merged;
                }
            }
            if uncertain_competition {
                continue;
            }
            let Some((i, delayed)) = candidate else {
                continue;
            };
            let request = &requests[i];
            let legacy = self
                .verified
                .get(&event.id)
                .is_some_and(|saved| saved.version == 0);
            if !legacy
                && !paired_receipt
                && !unknown_model(&event.model)
                && (codex_plus_core::aggregate_catalog::is_aggregate_model(&request.model)
                    || codex_plus_core::aggregate_catalog::is_aggregate_model(&event.model))
                && event.model != request.model
            {
                if let Some(saved) = self.verified.get_mut(&event.id) {
                    saved.rejected = true;
                }
                continue;
            }
            let Some(actual_model) = request
                .actual_model()
                .map(|model| {
                    if legacy {
                        request_base_model(model)
                    } else {
                        model
                    }
                })
                .filter(|model| !unknown_model(model))
            else {
                continue;
            };
            let routed_model = unknown_model(&event.model)
                || (codex_plus_core::aggregate_catalog::is_aggregate_model(&event.model)
                    && (legacy || request.model == event.model && event.model != actual_model));
            if !routed_model
                && !paired_receipt
                && !request_keys[i].iter().any(|key| {
                    if legacy {
                        request_base_model(&key.model) == request_base_model(&event.model)
                    } else {
                        key.model == event.model
                    }
                })
            {
                continue;
            }
            // 明确的直连身份冲突只撤销分类，不能静默改派另一家。
            if !unknown_provider(&event.provider) && event.provider != request.relay_id {
                event.provider.clear();
                event.context_conflicts.provider = true;
                self.reject_conflicting_event(event);
                direct_conflicts += 1;
                continue;
            }
            if self
                .verified
                .get(&event.id)
                .is_some_and(|saved| saved.rejected)
            {
                continue;
            }
            if !self.verified.contains_key(&event.id)
                && self
                    .captured_complete
                    .as_ref()
                    .is_some_and(|complete| !complete.contains(&event.id))
            {
                continue;
            }
            if !unknown_provider(&event.provider) && !routed_model && !paired_receipt {
                continue;
            }
            if !self.verified.contains_key(&event.id) && self.verified.len() < MAX_EVENTS {
                let (key, at) = evidence.as_ref().unwrap();
                self.verified.insert(
                    event.id,
                    VerifiedAttribution {
                        key: key.clone(),
                        at: *at,
                        request: request.clone(),
                        rejected: false,
                        version: 1,
                    },
                );
            }
            event.provider = request.relay_id.clone();
            event.provider_source = Some("request-log-exact".to_owned());
            if (routed_model || paired_receipt) && event.model != actual_model {
                event.model = actual_model.to_owned();
                model_events += 1;
            }
            if request.relay_name.trim().is_empty() {
                missing_names.insert(request.relay_id.clone());
            } else {
                let name = names.entry(request.relay_id.clone()).or_default();
                if name.trim().is_empty() {
                    name.clone_from(&request.relay_name);
                }
            }
            recovered += 1;
            tokens = tokens.saturating_add(event.delta.total_tokens);
            if delayed {
                delayed_events += 1;
                delayed_tokens = delayed_tokens.saturating_add(event.delta.total_tokens);
            }
            retained += usize::from(!current.contains(&request.id));
        }
        for id in missing_names {
            let name = names.entry(id.clone()).or_default();
            if name.trim().is_empty() {
                *name = id;
            }
        }
        json!({"source":"request-log-exact","scope":"index","recoveredEvents":recovered,"recoveredTokens":tokens,
            "recoveredModelEvents":model_events,"directProviderConflicts":direct_conflicts,
            "supplierOnlyEvents":supplier_only,
            "delayedEvents":delayed_events,"delayedTokens":delayed_tokens,
            "strictWindowMs":REQUEST_MATCH_WINDOW_MS,"delayedWindowMs":DELAYED_REQUEST_MATCH_WINDOW_MS,
            "retainedEvents":retained,"conflictedEvents":self.verified.values().filter(|saved| saved.rejected).count(),
            "logReads":self.reads,"logBytesRead":self.bytes_read})
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
struct ContextConflicts {
    model: bool,
    provider: bool,
    role: bool,
    turn: bool,
}

impl ContextConflicts {
    fn any(self) -> bool {
        self.model || self.provider || self.role || self.turn
    }
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
struct LegacyIdentity {
    id: [u8; 32],
    session: String,
    role: String,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct NativeResponseIdentity {
    id: Box<str>,
    full_counts: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct UsageEvent {
    native_response_id: Option<NativeResponseIdentity>,
    terminal_response: Option<Box<str>>,
    id: [u8; 32],
    original_id: [u8; 32],
    session: String,
    turn: String,
    model: String,
    timestamp: String,
    date: NaiveDate,
    total: Tokens,
    delta: Tokens,
    has_file_baseline: bool,
    last: Option<Tokens>,
    count_uncertain: bool,
    count_incomplete: bool,
    count_pending: bool,
    rollout_position: Option<(u64, u64, u64)>,
    provider: String,
    provider_source: Option<String>,
    role: String,
    inherited: bool,
    legacy_identity: Option<Box<LegacyIdentity>>,
    context_conflicts: ContextConflicts,
}

fn context_fields(raw: &str) -> serde_json::Result<[(&'static str, Option<&RawValue>); 14]> {
    #[derive(Deserialize)]
    struct Fields<'a> {
        #[serde(borrow)]
        id: Option<&'a RawValue>,
        #[serde(borrow)]
        session_id: Option<&'a RawValue>,
        #[serde(borrow)]
        history_base: Option<&'a RawValue>,
        #[serde(borrow)]
        source: Option<&'a RawValue>,
        #[serde(borrow)]
        model_provider: Option<&'a RawValue>,
        #[serde(borrow, rename = "modelProvider")]
        model_provider_camel: Option<&'a RawValue>,
        #[serde(borrow)]
        model: Option<&'a RawValue>,
        #[serde(borrow)]
        turn_id: Option<&'a RawValue>,
        #[serde(borrow)]
        forked_from_id: Option<&'a RawValue>,
        #[serde(borrow)]
        parent_thread_id: Option<&'a RawValue>,
        #[serde(borrow)]
        subagent_history_start_ordinal: Option<&'a RawValue>,
        #[serde(borrow)]
        thread_settings: Option<&'a RawValue>,
        #[serde(borrow, rename = "type")]
        kind: Option<&'a RawValue>,
        #[serde(borrow)]
        info: Option<&'a RawValue>,
    }
    let fields: Fields<'_> = serde_json::from_str(raw)?;
    Ok([
        ("id", fields.id),
        ("session_id", fields.session_id),
        ("history_base", fields.history_base),
        ("source", fields.source),
        ("model_provider", fields.model_provider),
        ("modelProvider", fields.model_provider_camel),
        ("model", fields.model),
        ("turn_id", fields.turn_id),
        ("forked_from_id", fields.forked_from_id),
        ("parent_thread_id", fields.parent_thread_id),
        (
            "subagent_history_start_ordinal",
            fields.subagent_history_start_ordinal,
        ),
        ("thread_settings", fields.thread_settings),
        ("type", fields.kind),
        ("info", fields.info),
    ])
}

fn merge_context_value(
    current: &mut String,
    incoming: &str,
    unknown: impl Fn(&str) -> bool,
    conflicted: &mut bool,
) {
    if *conflicted {
        return;
    }
    match (unknown(current), unknown(incoming)) {
        (true, true) => current.clear(),
        (true, false) => *current = incoming.to_owned(),
        (false, false) if current != incoming => {
            current.clear();
            *conflicted = true;
        }
        _ => {}
    }
}

impl UsageEvent {
    fn update_membership_digest(&self, digest: &mut Sha256) {
        digest.update(self.id);
        for count in [
            self.delta.input_tokens,
            self.delta.cached_input_tokens,
            self.delta.output_tokens,
            self.delta.total_tokens,
        ] {
            digest.update(count.to_le_bytes());
        }
        digest.update([u8::from(self.delta.cached_known)]);
        digest.update([u8::from(self.delta.reasoning_output_tokens.is_some())]);
        digest.update(
            self.delta
                .reasoning_output_tokens
                .unwrap_or_default()
                .to_le_bytes(),
        );
    }

    fn session_delta(&self, previous: Option<Tokens>) -> Tokens {
        match previous {
            Some(previous)
                if self.total.input_tokens == previous.input_tokens
                    && self.total.output_tokens == previous.output_tokens =>
            {
                Tokens::default()
            }
            Some(previous) if self.total.is_monotonic_after(previous) => {
                let merged = self.total.usage_delta(Some(previous), self.last);
                if !self.has_file_baseline || merged.total_tokens <= self.delta.total_tokens {
                    merged
                } else {
                    self.delta
                }
            }
            _ => self.delta,
        }
    }

    fn merge_duplicate_context(&mut self, other: &Self, names: &BTreeMap<String, String>) {
        if self.native_response_id != other.native_response_id {
            self.native_response_id = None;
        }
        // 先按各自模型解析供应商，避免合并模型后丢失原始候选的归属。
        if self.context_conflicts.provider {
            self.provider.clear();
        } else {
            let provider = usage_provider(&self.model, &self.provider, names).to_owned();
            let other_provider = usage_provider(&other.model, &other.provider, names).to_owned();
            self.provider = provider;
            merge_context_value(
                &mut self.provider,
                &other_provider,
                unknown_provider,
                &mut self.context_conflicts.provider,
            );
        }
        if self.context_conflicts.model || other.context_conflicts.model {
            self.model.clear();
            self.context_conflicts.model = true;
        } else {
            merge_context_value(
                &mut self.model,
                &other.model,
                unknown_model,
                &mut self.context_conflicts.model,
            );
        }
        merge_context_value(
            &mut self.role,
            &other.role,
            str::is_empty,
            &mut self.context_conflicts.role,
        );
        merge_context_value(
            &mut self.turn,
            &other.turn,
            str::is_empty,
            &mut self.context_conflicts.turn,
        );
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ClassificationEvidence {
    #[default]
    None,
    UpstreamExact,
    RequestExact,
    SupplierOnly,
    RecoveredRequestSupplier,
}

impl ClassificationEvidence {
    fn of(event: &UsageEvent) -> Self {
        if event.context_conflicts.any() {
            return Self::None;
        }
        match event.provider_source.as_deref() {
            Some("upstream-exact-bidirectional") => Self::UpstreamExact,
            Some("request-log-exact") => Self::RequestExact,
            Some("upstream-counter-bidirectional-supplier-only") => Self::SupplierOnly,
            Some(REQUEST_SUPPLIER_PROOF) => Self::RecoveredRequestSupplier,
            _ => Self::None,
        }
    }

    fn intersect(self, next: Self) -> Self {
        match (self, next) {
            (Self::UpstreamExact, Self::RequestExact)
            | (Self::RequestExact, Self::UpstreamExact) => Self::RequestExact,
            (left, right) if left == right => left,
            _ => Self::None,
        }
    }

    fn write_to(self, row: &mut Value) {
        match self {
            Self::None => {}
            Self::UpstreamExact => {
                row["classificationEvidence"] = json!("upstream-exact-bidirectional");
            }
            Self::RequestExact => {
                row["classificationEvidence"] = json!("request-log-exact");
            }
            Self::SupplierOnly => {
                row["supplierClassificationEvidence"] =
                    json!("upstream-counter-bidirectional-supplier-only");
            }
            Self::RecoveredRequestSupplier => {
                row["supplierClassificationEvidence"] = json!(REQUEST_SUPPLIER_PROOF);
            }
        }
    }
}

#[derive(serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyProviderEvidence {
    event_id: String,
    session: String,
    timestamp: String,
    model: String,
    role: String,
    input: u64,
    output: u64,
    cached: u64,
    provider: String,
    provider_name: String,
    source: String,
}

impl LegacyProviderEvidence {
    fn matches(&self, event: &UsageEvent, legacy: Option<&LegacyIdentity>) -> bool {
        self.session == legacy.map_or(event.session.as_str(), |id| id.session.as_str())
            && self.role == legacy.map_or(event.role.as_str(), |id| id.role.as_str())
            && DateTime::parse_from_rfc3339(&self.timestamp).ok()
                == DateTime::parse_from_rfc3339(&event.timestamp).ok()
            && self.input == event.delta.input_tokens
            && self.output == event.delta.output_tokens
            && self.cached == event.delta.cached_input_tokens
            && (self.model == event.model
                || (unknown_model(&event.model) && self.source == "upstream-exact-bidirectional"))
    }
}

#[derive(Clone, serde::Serialize, Deserialize)]
struct Call {
    timestamp: String,
    session: String,
    payload: Value,
}

#[derive(Clone)]
struct ToolRecord {
    id: String,
    call: Call,
    output: Value,
    date: NaiveDate,
}

#[derive(Clone, serde::Serialize, Deserialize)]
struct ExecSkill {
    call: Call,
    evidence: String,
}

#[derive(Clone, serde::Serialize, Deserialize)]
struct SessionModelObservation {
    at: i64,
    model: String,
    provider: String,
    role: String,
    unavailable: bool,
    had_unavailable: bool,
    unordered: bool,
}

#[derive(Clone, Default, serde::Serialize, Deserialize)]
struct InheritedUsageContext {
    session: String,
    model: String,
    model_unavailable: bool,
    turn: String,
    provider: String,
    role: String,
    end: u64,
    prefix: Option<(String, u64)>,
    ambiguous: bool,
}

enum PendingUsageReceipt {
    Unusable,
    One {
        id: Box<str>,
        at: i64,
        usage: Tokens,
        total: Tokens,
    },
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct TokenDaySpan {
    first: Option<i32>,
    last: Option<i32>,
}

#[derive(Default)]
struct FileState {
    pending_response: Option<PendingUsageReceipt>,
    stamp: Option<(u64, Option<SystemTime>, Option<(u64, u64)>)>,
    offset: u64,
    resume_pending: bool,
    replay_remaining: u64,
    reader: Option<index_budget::ReadCursor>,
    retained_charge: usize,
    pending: index_budget::PendingLine,
    skipping: bool,
    skip_activity: bool,
    activity_truncated: bool,
    session: String,
    model: String,
    model_unavailable: bool,
    turn: String,
    previous: Option<Tokens>,
    previous_cache_accounting: Option<CacheAccounting>,
    invalid_counts: bool,
    malformed_token_counts: usize,
    identity_conflict: bool,
    continuation: bool,
    history_base: Option<segments::HistoryBase>,
    rollout_extent: Option<(u64, u64, u64)>,
    rollout_out_of_order: bool,
    expired_count: Option<((u64, u64, u64), Tokens)>,
    token_day_span: Option<TokenDaySpan>,
    events: Vec<UsageEvent>,
    calls: HashMap<String, Call>,
    exec_skills: HashMap<String, ExecSkill>,
    exec_polls: HashMap<String, String>,
    tools: Vec<ToolRecord>,
    issues: usize,
    bytes_read: u64,
    provider: String,
    role: String,
    history_boundary: super::UsageHistoryBoundary,
    inherited_contexts: Vec<InheritedUsageContext>,
    model_observations: HashMap<String, SessionModelObservation>,
    legacy_header: Option<(String, String)>,
    scope_parent_candidates: BTreeMap<String, HashSet<Option<String>>>,
}

#[derive(Default)]
struct Index {
    supplier_evidence: supplier_evidence::Cache,
    collision_families: HashSet<[u8; 32]>,
    settings_path: Option<PathBuf>,
    storage_dir: Option<PathBuf>,
    files: BTreeMap<PathBuf, FileState>,
    provider_stamp: Option<native_endpoints::ConfigStamp>,
    provider_names: BTreeMap<String, String>,
    provider_endpoints: BTreeMap<String, Option<String>>,
    provider_endpoint_since: i64,
    request_evidence: RequestEvidenceCache,
    native_evidence: native_endpoints::Cache,
    native_cache_loaded: bool,
    native_cache_version: Option<i64>,
    provider_cache: provider_cache::ProviderCache,
    legacy_evidence: LegacyEvidenceCache,
}

#[derive(Default)]
struct Snapshot {
    supplier_evidence: Value,
    live_requests: Vec<RequestEvidence>,
    events: Vec<UsageEvent>,
    tools: Vec<ToolRecord>,
    sessions: HashSet<String>,
    aliases: BTreeMap<String, String>,
    cursors: BTreeMap<String, (String, Tokens)>,
    files: usize,
    ignored: usize,
    compressed: usize,
    indexed: usize,
    errors: usize,
    issues: usize,
    bytes_read: u64,
    elapsed_ms: u128,
    ready: bool,
    activity_truncated: bool,
    count_cache_available: bool,
    unobserved_count_conflicts: usize,
    updated_at: i64,
    provider_names: BTreeMap<String, String>,
    session_info: BTreeMap<String, Value>,
    scope_parents: BTreeMap<String, String>,
    provider_recovery: Value,
    provider_evidence: Value,
}

struct Cache {
    settings_path: PathBuf,
    storage: PathBuf,
    runtime: Option<token_checkpoint::Runtime>,
    snapshot: Arc<token_checkpoint::ReadState>,
    active: bool,
    refreshed: Option<Instant>,
    touched: Instant,
    _collector: Option<provider_capture::ProviderCollector>,
    worker: Option<std::thread::JoinHandle<()>>,
    archive: ArchiveSlot,
}

#[derive(Default)]
struct ArchiveSlot {
    cancelled: Arc<AtomicBool>,
    worker: Option<(
        token_checkpoint::ArchiveKey,
        std::thread::JoinHandle<token_checkpoint::ArchiveAnswer>,
    )>,
    result: Option<(Instant, Arc<token_checkpoint::ArchiveAnswer>)>,
}

impl ArchiveSlot {
    fn result(&mut self) -> Option<Arc<token_checkpoint::ArchiveAnswer>> {
        if self
            .worker
            .as_ref()
            .is_some_and(|(_, worker)| worker.is_finished())
        {
            let (key, worker) = self.worker.take().expect("finished archive worker");
            let answer = worker
                .join()
                .unwrap_or_else(|_| (key, Err("archive worker panicked".into())));
            self.result = Some((Instant::now(), Arc::new(answer)));
        }
        if self.result.as_ref().is_some_and(|(at, answer)| {
            at.elapsed() >= Duration::from_secs(if answer.1.is_ok() { 300 } else { 15 })
        }) {
            self.result = None;
        }
        self.result.as_ref().map(|(_, answer)| Arc::clone(answer))
    }
}

type HistoryCaches = Mutex<HashMap<PathBuf, Arc<Mutex<Cache>>>>;

fn homes() -> &'static HistoryCaches {
    static HOMES: OnceLock<HistoryCaches> = OnceLock::new();
    HOMES.get_or_init(Default::default)
}

fn stop_cache(cache: Arc<Mutex<Cache>>) {
    let (collector, worker, archive) = {
        let mut state = cache.lock().unwrap_or_else(|error| error.into_inner());
        state.archive.cancelled.store(true, Ordering::Relaxed);
        (
            state._collector.take(),
            state.worker.take(),
            state.archive.worker.take(),
        )
    };
    // 不持有索引锁等待退出；其他强引用不能延迟后台任务的收尾。
    drop(collector);
    if let Some(worker) = worker {
        let _ = worker.join();
    }
    if let Some((_, worker)) = archive {
        if worker.join().is_err() {
            eprintln!("usage archive worker panicked while stopping");
        }
    }
}

pub(super) fn clear(home: &Path) {
    let Ok(mut homes) = homes().lock() else {
        return;
    };
    if let Some(cache) = homes.remove(home) {
        stop_cache(cache);
    }
    // 并发 clear 也须等待已移除缓存的后台任务退出。
    drop(homes);
}

fn history_enabled(settings: &Path) -> bool {
    #[derive(Deserialize)]
    struct Settings {
        #[serde(rename = "usageHistoryEnabled", default)]
        enabled: bool,
    }
    let Ok(file) = File::open(settings) else {
        return false;
    };
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= 8 * 1024 * 1024)
    {
        return false;
    }
    serde_json::from_reader::<_, Settings>(file.take(8 * 1024 * 1024 + 1))
        .is_ok_and(|settings| settings.enabled)
}

fn evict_idle(home: &Path, cache: &Arc<Mutex<Cache>>) -> bool {
    let Ok(mut homes) = homes().lock() else {
        return true;
    };
    if !homes
        .get(home)
        .is_some_and(|current| Arc::ptr_eq(current, cache))
    {
        return true;
    }
    if !cache
        .lock()
        .is_ok_and(|state| !state.active && state.touched.elapsed() >= IDLE_TIMEOUT)
    {
        return false;
    }
    if let Some(removed) = homes.remove(home) {
        // 注册表锁覆盖退出过程，并发 clear 不能漏等已移除的任务。
        stop_cache(removed);
    }
    true
}

fn expire_idle(home: PathBuf, cache: std::sync::Weak<Mutex<Cache>>) {
    loop {
        let Some(shared) = cache.upgrade() else {
            return;
        };
        if evict_idle(&home, &shared) {
            return;
        }
        let delay = shared
            .lock()
            .map(|state| {
                IDLE_TIMEOUT
                    .saturating_sub(state.touched.elapsed())
                    .max(Duration::from_secs(1))
            })
            .unwrap_or(IDLE_TIMEOUT);
        // 等待期间只保留弱引用，关闭功能可立即释放最后一个强引用。
        drop(shared);
        std::thread::sleep(delay);
    }
}

pub(super) fn query(home: &Path, options: &Query, request: &Value) -> Value {
    query_with_settings(
        home,
        options,
        request,
        codex_plus_core::paths::default_settings_path(),
    )
}

fn query_with_settings(home: &Path, options: &Query, request: &Value, settings: PathBuf) -> Value {
    let scope_session = match request.get("scopeSessionId") {
        None => None,
        Some(value) => {
            let Some(id) = value.as_str().and_then(super::normalize_session_id) else {
                return json!({"status":"unavailable","complete":false,"message":"会话范围 ID 无效"});
            };
            Some(id)
        }
    };
    if !request["startAt"].is_null() || !request["endAt"].is_null() {
        if request["startAt"]
            .as_i64()
            .zip(request["endAt"].as_i64())
            .is_none_or(|(start, end)| start >= end)
        {
            return json!({"status":"unavailable","complete":false,"message":"用量时间范围无效"});
        }
    }
    let Ok(mut homes) = homes().lock() else {
        return json!({"status":"unavailable","complete":false});
    };
    // 与缓存创建共用锁，拦住已通过上层开关检查、关闭后才抵达的请求。
    if !history_enabled(&settings) {
        return json!({"status":"unavailable","enabled":false,"complete":false});
    }
    if !homes.contains_key(home) && homes.len() >= 2 {
        // 只回收空闲索引；运行中的扫描不能因另一个 home 查询而被反复创建。
        let idle = homes
            .iter()
            .find(|(_, cache)| cache.lock().is_ok_and(|c| !c.active))
            .map(|(path, _)| path.clone());
        if let Some(idle) = idle {
            if let Some(removed) = homes.remove(&idle) {
                stop_cache(removed);
            }
        } else {
            return json!({"status":"unavailable","complete":false});
        }
    }
    let cache = if let Some(cache) = homes.get(home) {
        cache.clone()
    } else {
        let storage_dir = provider_cache::storage_dir(home, &settings);
        let runtime = token_checkpoint::Runtime::new(storage_dir.clone(), settings.clone());
        let snapshot = Arc::new(runtime.initial_state());
        let cache = Arc::new(Mutex::new(Cache {
            settings_path: settings.clone(),
            runtime: Some(runtime),
            storage: storage_dir,
            snapshot,
            active: false,
            refreshed: None,
            touched: Instant::now(),
            _collector: None,
            worker: None,
            archive: ArchiveSlot::default(),
        }));
        let weak = Arc::downgrade(&cache);
        let identity = home.to_owned();
        if std::thread::Builder::new()
            .name("usage-history-expiry".into())
            .spawn(move || expire_idle(identity, weak))
            .is_err()
        {
            return json!({"status":"unavailable","complete":false});
        }
        homes.insert(home.to_owned(), cache.clone());
        cache
    };
    let Ok(mut state) = cache.lock() else {
        return json!({"status":"unavailable","complete":false});
    };
    if state.settings_path != settings {
        drop(state);
        drop(homes);
        clear(home);
        return query_with_settings(home, options, request, settings);
    }
    state.touched = Instant::now();
    if state._collector.is_none() {
        let sqlite_home = std::env::var_os("CODEX_SQLITE_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.to_owned());
        // 启动失败不影响只读查询；下一次请求会重试，而非永久静默停采。
        state._collector = provider_capture::start_provider_collector(
            home.to_owned(),
            sqlite_home,
            state.settings_path.clone(),
        )
        .ok();
    }
    if !state.active
        && state
            .refreshed
            .is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL)
    {
        let mut runtime = state
            .runtime
            .take()
            .expect("idle usage worker owns its runtime");
        state.active = true;
        let shared = cache.clone();
        let home = home.to_owned();
        match std::thread::Builder::new()
            .name("rollout-history".into())
            .spawn(move || {
                let snapshot = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runtime.update(&home)
                }));
                if let Ok(mut state) = shared.lock() {
                    if let Ok(snapshot) = snapshot {
                        state.snapshot = Arc::new(snapshot);
                    }
                    state.runtime = Some(runtime);
                    state.active = false;
                    state.refreshed = Some(Instant::now());
                }
            }) {
            Ok(worker) => state.worker = Some(worker),
            Err(error) => {
                state.active = false;
                state.refreshed = Some(Instant::now());
                state.runtime = Some(token_checkpoint::Runtime::new(
                    state.storage.clone(),
                    state.settings_path.clone(),
                ));
                return json!({"status":"unavailable","complete":false,"message":error.to_string()});
            }
        }
    }
    let snapshot = state.snapshot.clone();
    let storage = state.storage.clone();
    let processing = state.active;
    let archived = state.archive.result();
    drop(state);
    // clear 必须等本次启动登记完成，才能移除缓存并收齐后台任务。
    drop(homes);
    let (mut response, schedule) = snapshot.read_with_archive(
        &storage,
        options,
        request,
        scope_session.as_deref(),
        processing,
        archived.as_deref(),
    );
    if let Some(key) = schedule
        && let Ok(mut state) = cache.lock()
        && !state.archive.cancelled.load(Ordering::Relaxed)
        && state.archive.worker.is_none()
        && !state.archive.result.as_ref().is_some_and(|(_, ready)| {
            ready.0 == key
                && archived
                    .as_ref()
                    .is_none_or(|seen| !Arc::ptr_eq(seen, ready))
        })
    {
        let job = key.clone();
        let home = home.to_owned();
        let settings = state.settings_path.clone();
        let cancelled = state.archive.cancelled.clone();
        // 同一缓存仅允许一个只读回读；不阻塞前台查询或近期索引写者。
        match std::thread::Builder::new()
            .name("rollout-history-archive".into())
            .spawn(move || {
                token_checkpoint::recover_archive(
                    job, &home, &storage, &settings, &snapshot, &cancelled,
                )
            }) {
            Ok(worker) => state.archive.worker = Some((key, worker)),
            Err(error) => {
                let message = error.to_string();
                response["archiveRecovery"] = json!({"status":"partial","message":message});
                state.archive.result = Some((Instant::now(), Arc::new((key, Err(message)))));
            }
        }
    }
    response
}

fn log_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with("rollout-")
                && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"))
        })
}

fn discover(home: &Path, cold_start: bool, errors: &mut usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![home.join("sessions"), home.join("archived_sessions")];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                *errors += 1;
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else {
                *errors += 1;
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                *errors += 1;
                continue;
            };
            if kind.is_dir() {
                // 不跟随符号链接；年份/月/日目录是预期的最深层级。
                if entry
                    .path()
                    .strip_prefix(home)
                    .is_ok_and(|p| p.components().count() <= 4)
                {
                    pending.push(entry.path());
                }
            } else if kind.is_file() && log_file(&entry.path()) {
                if files.len() >= MAX_FILES {
                    *errors += 1;
                    continue;
                }
                match fs::canonicalize(entry.path()) {
                    Ok(path) if path.starts_with(home) => {
                        // 修改时间只用于冷启动排序；读取时仍校验路径和文件身份。
                        let modified = if cold_start {
                            entry
                                .metadata()
                                .and_then(|metadata| metadata.modified())
                                .ok()
                        } else {
                            None
                        };
                        files.push((std::cmp::Reverse(modified), path));
                    }
                    _ => *errors += 1,
                }
            }
        }
    }
    files.sort_unstable();
    files.into_iter().map(|(_, path)| path).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyEvidenceDocument {
    version: u32,
    records: Vec<LegacyProviderEvidence>,
}

impl LegacyEvidenceDocument {
    fn read(settings: &Path) -> anyhow::Result<Option<Self>> {
        const LIMIT: u64 = 32 * 1024 * 1024;
        let path = settings.with_file_name("usage-provider-evidence.json");
        let metadata = match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            other => other?,
        };
        anyhow::ensure!(
            metadata.is_file() && metadata.len() <= LIMIT,
            "evidence size/type"
        );
        let stamp = |m: &fs::Metadata| (m.len(), m.modified().ok(), file_identity(m));
        let file = File::open(&path)?;
        anyhow::ensure!(
            stamp(&file.metadata()?) == stamp(&metadata),
            "evidence replaced"
        );
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 == metadata.len()
                && stamp(&fs::symlink_metadata(&path)?) == stamp(&metadata),
            "evidence changed"
        );
        let document: Self = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            document.version == 1 && document.records.len() <= MAX_EVENTS,
            "evidence version/count"
        );
        Ok(Some(document))
    }

    fn timestamps(&self, times: &mut BTreeSet<i64>) {
        times.extend(self.records.iter().filter_map(|record| {
            DateTime::parse_from_rfc3339(&record.timestamp)
                .ok()
                .map(|at| at.timestamp_millis())
        }));
    }
}

impl Index {
    fn supplement_legacy_evidence(&mut self, settings: &Path, snapshot: &mut Snapshot) {
        self.legacy_evidence
            .apply(LegacyEvidenceDocument::read(settings), snapshot);
    }
}

#[derive(Default)]
struct LegacyEvidenceCache {
    signatures: HashMap<String, Option<[u8; 32]>>,
}

impl LegacyEvidenceCache {
    fn apply(
        &mut self,
        document: anyhow::Result<Option<LegacyEvidenceDocument>>,
        snapshot: &mut Snapshot,
    ) {
        // 不写入请求缓存，删除文件即可撤销，改写身份在本轮索引内不再信任。
        let result = (|| -> anyhow::Result<Value> {
            let Some(document) = document? else {
                return Ok(json!({"status":"absent"}));
            };
            let mut records = HashMap::new();
            let mut invalid = 0usize;
            for record in document.records {
                let id = record.event_id.to_ascii_lowercase();
                if id.len() != 64
                    || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
                    || (!self.signatures.contains_key(&id) && self.signatures.len() >= MAX_EVENTS)
                {
                    invalid += 1;
                    continue;
                }
                let signature: [u8; 32] = Sha256::digest(serde_json::to_vec(&record)?).into();
                let valid = [
                    &record.session,
                    &record.timestamp,
                    &record.model,
                    &record.provider,
                ]
                .iter()
                .all(|s| {
                    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
                }) && record.provider_name.len() <= 256
                    && !record.provider_name.chars().any(char::is_control)
                    && record.provider.trim() == record.provider
                    && record.model.trim() == record.model
                    && !unknown_provider(&record.provider)
                    && !unknown_model(&record.model)
                    && !unknown_model(request_base_model(&record.model))
                    && matches!(record.role.as_str(), "main" | "subagent")
                    && matches!(
                        record.source.as_str(),
                        "upstream-exact-bidirectional"
                            | "user-classified"
                            | "user-provider-classification"
                    )
                    && DateTime::parse_from_rfc3339(&record.timestamp).is_ok()
                    && record.cached <= record.input
                    && record
                        .input
                        .checked_add(record.output)
                        .is_some_and(|n| n > 0)
                    && !records.contains_key(&id)
                    && self
                        .signatures
                        .get(&id)
                        .is_none_or(|previous| *previous == Some(signature));
                self.signatures
                    .insert(id.clone(), valid.then_some(signature));
                invalid += usize::from(!valid);
                records.insert(id, valid.then_some(record));
            }
            let mut unique = HashSet::new();
            let mut duplicates = HashSet::new();
            let mut legacy_counts = HashMap::new();
            for event in &snapshot.events {
                if !unique.insert(event.id) {
                    duplicates.insert(event.id);
                }
                if let Some(legacy) = &event.legacy_identity {
                    *legacy_counts.entry(legacy.id).or_insert(0usize) += 1;
                }
            }
            let (mut applied, mut conflicts, mut mismatches, mut migrated) =
                (0usize, 0usize, 0usize, 0usize);
            let mut sources = BTreeMap::<String, usize>::new();
            for event in &mut snapshot.events {
                let id: String = event.id.iter().map(|byte| format!("{byte:02x}")).collect();
                let direct = records.get(&id).and_then(Option::as_ref);
                // 只兼容明确复制头形成的旧身份，两端唯一且完整签名一致；不借父供应商推断。
                let legacy = event.legacy_identity.as_ref().and_then(|identity| {
                    if unique.contains(&identity.id) || legacy_counts.get(&identity.id) != Some(&1)
                    {
                        return None;
                    }
                    let id: String = identity
                        .id
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    records
                        .get(&id)
                        .and_then(Option::as_ref)
                        .filter(|record| record.matches(event, Some(identity)))
                });
                if records.get(&id).is_some_and(Option::is_none) {
                    continue;
                }
                if direct
                    .zip(legacy)
                    .is_some_and(|(a, b)| a.provider != b.provider)
                {
                    conflicts += 1;
                    continue;
                }
                let Some(record) = direct.or(legacy) else {
                    continue;
                };
                if event.context_conflicts.any()
                    || event.count_uncertain
                    || event.count_incomplete
                    || !event.delta.cached_known
                {
                    conflicts += 1;
                    continue;
                }
                let missing_model = unknown_model(&event.model);
                if duplicates.contains(&event.id)
                    || direct.is_some_and(|record| !record.matches(event, None))
                {
                    mismatches += 1;
                    continue;
                }
                if !unknown_provider(&event.provider) && event.provider != record.provider {
                    conflicts += 1;
                    continue;
                }
                if unknown_provider(&event.provider) {
                    event.provider.clone_from(&record.provider);
                    event.provider_source = Some(record.source.clone());
                    applied += 1;
                    migrated += usize::from(direct.is_none());
                    *sources.entry(record.source.clone()).or_default() += 1;
                }
                if missing_model {
                    event.model.clone_from(&record.model);
                }
                if !record.provider_name.trim().is_empty() {
                    let name = snapshot
                        .provider_names
                        .entry(record.provider.clone())
                        .or_default();
                    if name.trim().is_empty() {
                        name.clone_from(&record.provider_name);
                    }
                }
            }
            Ok(
                json!({"status":"ok","source":"legacy-provider-evidence","appliedEvents":applied,
                "sources":sources,"invalid":invalid,"conflictedEvents":conflicts,"signatureMismatches":mismatches,
                "migratedEvents":migrated}),
            )
        })();
        snapshot.provider_evidence = result.unwrap_or_else(|_| json!({"status":"rejected"}));
    }
}

impl Index {
    fn sync_native_evidence(&mut self, cutoff: NaiveDate) -> anyhow::Result<()> {
        for _ in 0..2 {
            let before = self.provider_cache.data_version()?;
            if self.native_cache_loaded && self.native_cache_version == Some(before) {
                return Ok(());
            }
            let records = self.provider_cache.load_native(cutoff)?;
            let after = self.provider_cache.data_version()?;
            if before == after {
                self.native_evidence.restore_after_failed_load(records);
                self.native_cache_loaded = true;
                self.native_cache_version = Some(after);
                return Ok(());
            }
        }
        self.native_cache_loaded = false;
        self.native_cache_version = None;
        anyhow::bail!("native evidence changed during reload")
    }

    fn native_evidence_available(
        &mut self,
        home: &Path,
        cutoff: NaiveDate,
    ) -> anyhow::Result<bool> {
        self.sync_native_evidence(cutoff)?;
        let available = native_endpoints::cache_below_limit(
            self.storage_dir.as_deref().unwrap_or(home),
            cutoff,
        )?;
        // 容量和全局屏障必须与重载的事件屏障属于同一未变更版本。
        Ok(available && self.native_cache_version == Some(self.provider_cache.data_version()?))
    }

    fn observed_collisions(&self) -> HashMap<[u8; 32], i64> {
        // 原有事件 ID 保持不变；仅给有明确不同轮次的碰撞分配独立身份。
        let mut turns = HashMap::<[u8; 32], (bool, HashSet<String>, i64)>::new();
        for event in self
            .files
            .values()
            .filter(|state| !state.identity_conflict && !state.session.is_empty())
            .flat_map(|state| &state.events)
        {
            let family = turns
                .entry(event.original_id)
                .or_insert_with(|| (event.inherited, HashSet::new(), 0));
            if !family.0 && event.inherited {
                continue;
            }
            if family.0 && !event.inherited {
                family.0 = false;
                family.1.clear();
            }
            if !event.turn.is_empty() {
                family.1.insert(event.turn.clone());
            }
            family.2 = family.2.max(
                DateTime::parse_from_rfc3339(&event.timestamp)
                    .map_or(0, |time| time.timestamp_millis()),
            );
        }
        turns
            .into_iter()
            .filter_map(|(id, (_, turns, at))| (turns.len() > 1).then_some((id, at)))
            .collect()
    }

    fn apply_collision_ids(&mut self) {
        for event in self.files.values_mut().flat_map(|state| &mut state.events) {
            let original = event.original_id;
            event.id = if self.collision_families.contains(&original) && !event.turn.is_empty() {
                let mut digest = Sha256::new();
                digest.update(b"usage-turn-collision-v1\0");
                digest.update(original);
                digest.update(event.turn.as_bytes());
                digest.finalize().into()
            } else {
                if self.collision_families.contains(&original) {
                    event.context_conflicts.turn = true;
                    event.count_uncertain = true;
                }
                original
            };
        }
    }

    fn select_observations(
        &self,
        cutoff: NaiveDate,
        snapshot: &mut Snapshot,
        current_aliases: &mut BTreeMap<String, String>,
        event_limit: usize,
    ) {
        let mut seen = HashMap::new();
        let mut tools = BTreeMap::new();
        for (path, state) in &self.files {
            if state.identity_conflict || state.session.is_empty() && state.stamp.is_some() {
                snapshot.ignored += 1;
                continue;
            }
            if !state.session.is_empty() {
                snapshot.sessions.insert(state.session.clone());
                let info = snapshot
                    .session_info
                    .entry(state.session.clone())
                    .or_insert_with(|| {
                        json!({
                            "model":"","provider":"","role":state.role
                        })
                    });
                if !state.model.is_empty() || info["model"].as_str().unwrap_or("").is_empty() {
                    *info = json!({
                    "model":state.model,"provider":usage_provider(&state.model, &state.provider, &self.provider_names),
                    "role":state.role
                    });
                }
                if let Some(id) = segments::rollout_id(path) {
                    let id = id.to_string();
                    snapshot
                        .aliases
                        .entry(id.clone())
                        .and_modify(|previous| {
                            if *previous != state.session {
                                *previous = id.clone();
                            }
                        })
                        .or_insert_with(|| state.session.clone());
                    current_aliases.insert(id.clone(), snapshot.aliases[&id].clone());
                }
            }
            for event in &state.events {
                if event.date < cutoff || event.terminal_response.is_some() {
                    continue;
                }
                if let Some(&position) = seen.get(&event.id) {
                    let previous: &mut UsageEvent = &mut snapshot.events[position];
                    if previous.inherited && !event.inherited {
                        *previous = event.clone();
                    } else if previous.inherited == event.inherited {
                        previous.count_uncertain |= event.count_uncertain;
                        previous.total.cached_known &= event.total.cached_known;
                        previous.delta.cached_known &= event.delta.cached_known;
                        if let (Some(last), Some(other)) = (&mut previous.last, event.last) {
                            last.cached_known &= other.cached_known;
                        }
                        previous.merge_duplicate_context(event, &self.provider_names);
                    }
                    continue;
                }
                if snapshot.events.len() == event_limit {
                    snapshot.issues += 1;
                    break;
                }
                let cursor = snapshot
                    .cursors
                    .entry(event.session.clone())
                    .or_insert_with(|| (event.timestamp.clone(), event.total));
                if event.timestamp >= cursor.0 {
                    *cursor = (event.timestamp.clone(), event.total);
                }
                seen.insert(event.id, snapshot.events.len());
                snapshot.events.push(event.clone());
            }
            for tool in &state.tools {
                if tool.date >= cutoff {
                    // 同一调用在复制/迁移日志里只保留一个完成回执。
                    tools.insert((tool.call.session.clone(), tool.id.clone()), tool.clone());
                }
            }
        }
        // 真正有独立头部的会话不是另一个会话的别名。
        for state in self
            .files
            .values()
            .filter(|state| !state.identity_conflict && !state.session.is_empty())
        {
            snapshot
                .aliases
                .insert(state.session.clone(), state.session.clone());
            current_aliases.insert(state.session.clone(), state.session.clone());
        }
        snapshot
            .collect_scope_parents(self.files.values().filter(|state| !state.identity_conflict));
        snapshot.tools = tools.into_values().collect();
    }

    fn resolve_counts(&self, snapshot: &mut Snapshot) -> HashMap<[u8; 32], usize> {
        // 无法读取累计输入或输出时，整个会话的增量边界暂时不可核验。
        let incomplete_sessions: HashSet<_> = self
            .files
            .values()
            .filter(|state| state.invalid_counts)
            .flat_map(|state| {
                std::iter::once(&state.session)
                    .chain(state.events.iter().map(|event| &event.session))
                    .chain(
                        state
                            .inherited_contexts
                            .iter()
                            .map(|context| &context.session),
                    )
            })
            .cloned()
            .collect();
        for event in &mut snapshot.events {
            event.count_incomplete = incomplete_sessions.contains(&event.session);
        }
        let prefixes = segments::referenced_prefixes(&self.files);
        let (components, mut predecessors, contradictory_order) =
            segments::causal_order(&mut snapshot.events, &self.files, &prefixes);
        snapshot.issues += usize::from(contradictory_order);
        let unresolved =
            segments::prune_transitive_predecessors(&mut predecessors, &components, 1_000_000);
        let ordered_totals: Vec<_> = snapshot.events.iter().map(|event| event.total).collect();
        let mut seen: HashMap<_, _> = snapshot
            .events
            .iter()
            .enumerate()
            .map(|(position, event)| (event.id, position))
            .collect();
        let mut continuation_starts = vec![false; snapshot.events.len()];
        let mut independent_starts = vec![false; snapshot.events.len()];
        let mut external_baselines = HashMap::new();
        for (path, state) in self
            .files
            .iter()
            .filter(|(_, state)| !state.identity_conflict)
        {
            if state.session.is_empty() && state.stamp.is_some() {
                continue;
            }
            for event in &state.events {
                let Some(&position) = seen.get(&event.id) else {
                    continue;
                };
                if snapshot.events[position].inherited != event.inherited {
                    continue;
                }
                if !state.continuation || event.has_file_baseline {
                    independent_starts[position] = true;
                } else if let Some(prefix) = prefixes.get(path) {
                    if let segments::PrefixCount::Baseline(total) = prefix {
                        external_baselines.entry(position).or_insert(*total);
                    }
                } else {
                    continuation_starts[position] = true;
                }
            }
        }
        let mut count_evidence = HashMap::new();
        let mut conflicting_evidence = Vec::new();
        let mut alternate_totals = HashMap::<_, Vec<Tokens>>::new();
        for event in &snapshot.events {
            alternate_totals
                .entry((
                    event.session.clone(),
                    event.timestamp.clone(),
                    event.turn.clone(),
                    event.total.input_tokens,
                    event.total.output_tokens,
                ))
                .or_default()
                .push(event.total);
        }
        let mut preceding_totals = Vec::with_capacity(snapshot.events.len());
        let mut previous_baselines = Vec::with_capacity(snapshot.events.len());
        let merge_delta = |event: &mut UsageEvent, other: Tokens, temporary: bool| {
            if event.delta.same_counts(other) && event.delta.cached_known == other.cached_known {
                return;
            }
            if event.delta.input_tokens != other.input_tokens
                || event.delta.output_tokens != other.output_tokens
            {
                if !event.count_incomplete {
                    event.count_pending |= temporary;
                    event.count_uncertain = true;
                }
            }
            if !event.delta.cached_known
                || !other.cached_known
                || event.delta.cached_input_tokens != other.cached_input_tokens
            {
                event.delta.cached_input_tokens = 0;
                event.delta.cached_known = false;
            }
            if event.delta.reasoning_output_tokens != other.reasoning_output_tokens {
                event.delta.reasoning_output_tokens = None;
            }
        };
        seen.clear();
        // 模型仅取解析时已有的上下文或精确补证；会话摘要及其他事件不能证明连续性。
        for (position, event) in snapshot.events.iter_mut().enumerate() {
            event.provider =
                usage_provider(&event.model, &event.provider, &self.provider_names).to_owned();
            if event.context_conflicts.provider {
                event.provider.clear();
            }
            let previous_position = predecessors[position]
                .iter()
                .copied()
                .filter(|&before| before < position)
                .max();
            let previous = previous_position
                .map(|before| ordered_totals[before])
                .or(external_baselines.get(&position).copied());
            previous_baselines.push(previous);
            let unanchored = previous.is_none()
                && continuation_starts[position]
                && !independent_starts[position];
            event.count_pending |= unresolved[position]
                || unanchored
                || previous.is_none() && !event.has_file_baseline && event.last.is_none();
            event.count_uncertain |= event.count_pending;
            if unresolved[position] {
                event.delta = Tokens::default();
            } else {
                event.count_uncertain |= event.last.is_none()
                    && previous.is_some_and(|before| event.total.requires_last_after(before));
                event.delta = event.session_delta(previous);
            }
            // 竞争基线会影响本流下一次增量；其他分段插入不能解除这种不确定性。
            if !unresolved[position] {
                for &before in &predecessors[position] {
                    if let Some(Some(totals)) = preceding_totals.get(before) {
                        for &total in *totals {
                            merge_delta(
                                event,
                                event.session_delta(Some(total)),
                                predecessors[position].len() > 1,
                            );
                        }
                    }
                }
            }
            // 缓存/推理分量也可能被补全或修正，不能让它们掩盖相同累计输入输出的竞争记录。
            let evidence_key = (
                event.session.clone(),
                event.timestamp.clone(),
                event.turn.clone(),
                event.total.input_tokens,
                event.total.output_tokens,
            );
            preceding_totals.push(alternate_totals.get(&evidence_key));
            if let Some(first) = count_evidence.insert(evidence_key, position) {
                conflicting_evidence.push(first);
                conflicting_evidence.push(position);
            }
            seen.insert(event.id, position);
        }
        for position in conflicting_evidence {
            snapshot.events[position].count_uncertain = true;
        }
        // 使用同一个会话基线比较所有同优先级副本，不能用首次出现的文件决定计数。
        for (path, state) in self
            .files
            .iter()
            .filter(|(_, state)| !state.identity_conflict)
        {
            let mut source_previous = HashMap::new();
            for candidate in &state.events {
                let Some(&position) = seen.get(&candidate.id) else {
                    continue;
                };
                let selected = &mut snapshot.events[position];
                if candidate.inherited != selected.inherited {
                    continue;
                }
                let previous = source_previous
                    .insert(candidate.session.as_str(), position)
                    .filter(|&before| {
                        before != position && predecessors[position].binary_search(&before).is_ok()
                    })
                    .map(|before| ordered_totals[before])
                    .or_else(|| {
                        if !candidate.has_file_baseline {
                            match prefixes.get(path) {
                                Some(segments::PrefixCount::Baseline(total)) => Some(*total),
                                Some(segments::PrefixCount::Event(id, total))
                                    if seen.get(id).is_none_or(|before| {
                                        predecessors[position].binary_search(before).is_ok()
                                    }) =>
                                {
                                    Some(*total)
                                }
                                _ => None,
                            }
                        } else {
                            None
                        }
                    })
                    .or(previous_baselines[position]);
                if !unresolved[position] {
                    merge_delta(
                        selected,
                        candidate.session_delta(previous),
                        predecessors[position].len() > 1
                            || external_baselines.contains_key(&position)
                                && prefixes.get(path).is_some_and(|prefix| {
                                    matches!(prefix, segments::PrefixCount::Baseline(_))
                                }),
                    );
                }
            }
        }
        let component_correction = |before: Tokens, after: Tokens| {
            if before.input_tokens != after.input_tokens
                || before.output_tokens != after.output_tokens
            {
                return (false, false);
            }
            (
                before.cached_known
                    && after.cached_known
                    && before.cached_input_tokens != after.cached_input_tokens,
                before.reasoning_output_tokens.is_some()
                    && after.reasoning_output_tokens.is_some()
                    && before.reasoning_output_tokens != after.reasoning_output_tokens,
            )
        };
        let mut corrections = Vec::new();
        for (after, before_positions) in predecessors.iter().enumerate() {
            if unresolved[after] {
                continue;
            }
            for &before in before_positions {
                if snapshot.events[before].session != snapshot.events[after].session {
                    continue;
                }
                let flags = component_correction(ordered_totals[before], ordered_totals[after]);
                if flags.0 || flags.1 {
                    corrections.push((before, flags));
                }
            }
        }
        if !corrections.is_empty() {
            // 修正只影响同一累计链中此前的事件，不传播到兄弟分支或修正后的增量。
            let mut affected = vec![(false, false); snapshot.events.len()];
            while let Some((position, (cached, reasoning))) = corrections.pop() {
                let flags = (
                    cached && !affected[position].0,
                    reasoning && !affected[position].1,
                );
                if !flags.0 && !flags.1 {
                    continue;
                }
                affected[position].0 |= flags.0;
                affected[position].1 |= flags.1;
                if unresolved[position] {
                    continue;
                }
                for &before in &predecessors[position] {
                    if snapshot.events[position].session == snapshot.events[before].session
                        && ordered_totals[position].is_monotonic_after(ordered_totals[before])
                    {
                        let boundary =
                            component_correction(ordered_totals[before], ordered_totals[position]);
                        corrections
                            .push((before, (flags.0 && !boundary.0, flags.1 && !boundary.1)));
                    }
                }
            }
            for (event, (cached, reasoning)) in snapshot.events.iter_mut().zip(affected) {
                if cached {
                    event.delta.cached_input_tokens = 0;
                    event.delta.cached_known = false;
                }
                if reasoning {
                    event.delta.reasoning_output_tokens = None;
                }
            }
        }
        seen
    }

    fn load_evidence(
        &mut self,
        home: &Path,
        cutoff: NaiveDate,
        request_log: &Path,
    ) -> (bool, bool) {
        let mut cache_available = true;
        let mut cache_opened = false;
        match self
            .provider_cache
            .open(self.storage_dir.as_deref().unwrap_or(home), cutoff)
        {
            Ok(true) => {
                cache_opened = true;
                self.native_cache_loaded = false;
                self.native_cache_version = None;
                // refresh 会在路径变化时清空请求缓存，必须先建立路径再恢复落盘证据。
                self.request_evidence.refresh(request_log);
                match self.provider_cache.load_verified(cutoff) {
                    Ok(records) => self.request_evidence.restore_verified(records),
                    Err(_) => cache_available = false,
                }
            }
            Ok(false) => cache_opened = true,
            Err(_) => cache_available = false,
        }
        if cache_opened && self.sync_native_evidence(cutoff).is_err() {
            self.native_cache_loaded = false;
            self.native_cache_version = None;
            cache_available = false;
        }
        // 加载器会跳过无效条目；仅比较加载后的数量无法证明 LIMIT 未截断。
        if cache_available {
            cache_available = native_endpoints::cache_below_limit(
                self.storage_dir.as_deref().unwrap_or(home),
                cutoff,
            )
            .unwrap_or(false);
        }
        (cache_opened, cache_available)
    }

    fn refresh_provider_profiles(&mut self, settings: &Path) {
        let stamp = native_endpoints::config_stamp(settings);
        if self.provider_stamp.as_ref() != Some(&stamp) {
            self.provider_endpoints.clear();
            self.provider_names.clear();
            self.provider_endpoint_since = 0;
            self.provider_stamp = None;
            if let Ok(profiles) = native_endpoints::profiles(settings)
                && native_endpoints::config_stamp(settings) == stamp
            {
                self.provider_endpoints = native_endpoints::endpoints(&profiles);
                self.provider_endpoint_since = self
                    .provider_cache
                    .endpoint_binding_since(&self.provider_endpoints)
                    .unwrap_or_else(|_| Local::now().timestamp_millis());
                self.provider_names = profiles
                    .into_iter()
                    .map(|name| (name.id, name.name))
                    .collect();
                self.provider_stamp = Some(stamp);
            }
        }
    }

    #[cfg(test)]
    fn update_with_request_log(&mut self, home: &Path, request_log: &Path) -> Snapshot {
        let started = Instant::now();
        let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
        let mut snapshot = Snapshot::default();
        let (cache_opened, mut cache_available) = self.load_evidence(home, cutoff, request_log);
        snapshot.aliases = self.provider_cache.aliases.clone();
        let mut current_aliases = BTreeMap::new();
        let settings = self
            .settings_path
            .clone()
            .unwrap_or_else(codex_plus_core::paths::default_settings_path);
        self.refresh_provider_profiles(&settings);
        snapshot.provider_names = self.provider_names.clone();
        let paths = discover(home, self.files.is_empty(), &mut snapshot.errors);
        let existing: HashSet<_> = paths.iter().cloned().collect();
        self.files.retain(|path, _| existing.contains(path));
        let mut budget = index_budget::Budget::new(self.files.values());
        for path in paths {
            snapshot.files += 1;
            snapshot.compressed += usize::from(path.extension().is_some_and(|ext| ext == "zst"));
            let state = self.files.entry(path.clone()).or_default();
            state.bytes_read = 0;
            let previous_usage = index_budget::Usage::of(state);
            budget.replace(previous_usage, index_budget::Usage::default());
            match state.read_with_budget(&path, cutoff, &mut budget) {
                Err(_) => {
                    snapshot.errors += 1;
                    // 下次从有效文件头重试，不能让失败的解压流留下错误累计基线。
                    *state = FileState::default();
                }
                Ok(true) => snapshot.indexed += 1,
                Ok(false) => snapshot.errors += 1,
            }
            budget.replace(
                index_budget::Usage::default(),
                index_budget::Usage::of(state),
            );
            snapshot.issues += state.issues;
            snapshot.activity_truncated |= state.activity_truncated;
            snapshot.bytes_read += state.bytes_read;
        }
        let observed_collisions = self.observed_collisions();
        let collision_identity_available = match self
            .provider_cache
            .event_collisions(&observed_collisions, cutoff)
        {
            Ok(collisions) => {
                self.collision_families = collisions;
                true
            }
            Err(_) => {
                cache_available = false;
                self.collision_families
                    .extend(observed_collisions.keys().copied());
                false
            }
        };
        self.apply_collision_ids();
        self.select_observations(cutoff, &mut snapshot, &mut current_aliases, MAX_EVENTS);
        let seen = self.resolve_counts(&mut snapshot);
        let scan_complete = snapshot.errors == 0 && snapshot.issues == 0;
        let count_cache_has_room = self.native_evidence.observe_count_conflicts(
            &mut snapshot.events,
            scan_complete && collision_identity_available,
        );
        if !collision_identity_available {
            for event in &mut snapshot.events {
                event.count_incomplete = true;
            }
        }
        snapshot.count_cache_available = cache_available && count_cache_has_room;
        snapshot.unobserved_count_conflicts =
            self.native_evidence.unobserved_count_conflicts(&seen);
        snapshot.events.retain(|event| {
            event.delta.total_tokens > 0
                || event.count_uncertain
                || event.count_incomplete
                || event.context_conflicts.any()
        });
        self.request_evidence.refresh(request_log);
        let captured = self.request_evidence.supplement_captured(
            &snapshot.events,
            &snapshot.aliases,
            &self.provider_cache,
        );
        let aliases = RequestSessionAliases::new(&snapshot.aliases);
        let mut observations =
            HashMap::<String, (&SessionModelObservation, bool, bool, bool)>::new();
        let mut unavailable_session_models = HashSet::new();
        for state in self
            .files
            .values()
            .filter(|state| !state.identity_conflict && !state.session.is_empty())
        {
            for (session, observation) in &state.model_observations {
                let Some(session) = aliases.resolve(session) else {
                    if observation.had_unavailable {
                        unavailable_session_models.insert(session.clone());
                    }
                    continue;
                };
                let entry =
                    observations
                        .entry(session)
                        .or_insert((observation, false, false, false));
                entry.1 |= observation.had_unavailable;
                entry.3 |= observation.unordered;
                if observation.at > entry.0.at {
                    entry.0 = observation;
                    entry.2 = false;
                } else if observation.at == entry.0.at
                    && (observation.model != entry.0.model
                        || observation.provider != entry.0.provider
                        || observation.role != entry.0.role
                        || observation.unavailable != entry.0.unavailable)
                {
                    entry.2 = true;
                }
            }
        }
        for (session, (latest, had_unavailable, tied, unordered)) in observations {
            if !had_unavailable {
                continue;
            }
            if latest.unavailable || unknown_model(&latest.model) || tied || unordered {
                if let Some(info) = snapshot.session_info.get_mut(&session) {
                    info["model"] = json!("");
                    if tied || unordered {
                        info["provider"] = json!("");
                        info["role"] = json!("unknown");
                    } else {
                        info["provider"] = json!(usage_provider(
                            &latest.model,
                            &latest.provider,
                            &self.provider_names
                        ));
                        info["role"] = json!(latest.role);
                    }
                }
                unavailable_session_models.insert(session);
            } else if let Some(info) = snapshot.session_info.get_mut(&session) {
                info["model"] = json!(latest.model);
                info["provider"] = json!(usage_provider(
                    &latest.model,
                    &latest.provider,
                    &self.provider_names
                ));
                info["role"] = json!(latest.role);
            }
        }
        self.request_evidence.supplement_missing_session_models(
            &mut snapshot.session_info,
            &unavailable_session_models,
            &aliases,
            Local::now().timestamp_millis(),
        );
        self.request_evidence
            .collision_families
            .clone_from(&self.collision_families);
        snapshot.provider_recovery = self.request_evidence.attribute_with_cache(
            &mut snapshot.events,
            &mut snapshot.provider_names,
            &snapshot.aliases,
            &self.provider_cache,
        );
        snapshot.live_requests = self.request_evidence.records.clone();
        for record in &mut snapshot.live_requests {
            if self.request_evidence.captured_id_check_failed
                || self.request_evidence.captured_rejected.contains(&record.id)
            {
                // 拒绝记录仍作为合并屏障和竞争证据，不能过滤后拼接两侧请求。
                record.success = false;
            }
        }
        snapshot.provider_recovery["capturedProxyRequestsRead"] = json!(captured.0);
        snapshot.provider_recovery["capturedProxyLimited"] = json!(captured.1);
        snapshot.provider_recovery["nativeEndpoint"] = self.native_evidence.attribute(
            home,
            &mut snapshot.events,
            &self.provider_endpoints,
            self.provider_endpoint_since,
            &snapshot.aliases,
            &self.provider_cache,
            scan_complete && snapshot.count_cache_available,
        );
        self.supplement_legacy_evidence(&settings, &mut snapshot);
        self.supplier_evidence.supplement(&settings, &mut snapshot);
        let writes = self
            .provider_cache
            .save(&self.request_evidence.verified, &current_aliases);
        cache_available &= writes.is_ok();
        let native_writes = if cache_opened && self.native_cache_loaded {
            self.native_evidence.save_bounded(
                self.storage_dir.as_deref().unwrap_or(home),
                cutoff,
                &snapshot.events,
            )
        } else {
            Err(anyhow::anyhow!("native evidence cache unavailable"))
        };
        cache_available &= native_writes.is_ok();
        if cache_opened && self.native_cache_loaded {
            match self.native_evidence_available(home, cutoff) {
                Ok(available) => cache_available &= available,
                Err(_) => {
                    self.native_cache_loaded = false;
                    self.native_cache_version = None;
                    cache_available = false;
                }
            }
        }
        self.native_evidence
            .apply_count_conflicts(&mut snapshot.events);
        let observed = snapshot
            .events
            .iter()
            .enumerate()
            .map(|(position, event)| (event.id, position))
            .collect();
        snapshot.unobserved_count_conflicts =
            self.native_evidence.unobserved_count_conflicts(&observed);
        snapshot.count_cache_available &= cache_available
            && native_writes
                .as_ref()
                .is_ok_and(|(_, available)| *available)
            && self.native_evidence.verified.len() < MAX_EVENTS;
        // 精确事件证据供尚未合并的实时回执使用；不把最后供应商推广到整段会话。
        let now = Local::now().timestamp_millis();
        let mut recent: Vec<_> = snapshot
            .events
            .iter()
            .filter(|event| {
                !event.context_conflicts.any()
                    && !event.count_uncertain
                    && !event.count_incomplete
                    && event.total.cached_known
                    && event.delta.cached_known
                    && snapshot.count_cache_available
                    && !unknown_provider(&event.provider)
                    && event.provider_source.as_deref().is_none_or(|source| {
                        matches!(source, "upstream-exact-bidirectional" | "request-log-exact")
                    })
                    && DateTime::parse_from_rfc3339(&event.timestamp).is_ok_and(|at| {
                        (0..=600_000).contains(&now.saturating_sub(at.timestamp_millis()))
                    })
            })
            .collect();
        recent.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        let mut per_session = HashMap::new();
        for event in recent.into_iter().take(128) {
            let count = per_session.entry(&event.session).or_insert(0usize);
            if *count >= 16 {
                continue;
            }
            *count += 1;
            let info = snapshot
                .session_info
                .entry(event.session.clone())
                .or_insert_with(|| json!({}));
            if !info["verifiedUsage"].is_array() {
                info["verifiedUsage"] = json!([]);
            }
            info["verifiedUsage"].as_array_mut().unwrap().push(json!({
                "sourceEventId":event.id.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
                "timestamp":event.timestamp,"model":event.model,"provider":event.provider,
                "providerSource":event.provider_source,"role":event.role,"total":event.total.value(),"delta":event.delta.value()
            }));
        }
        snapshot.provider_recovery["nativeCacheWrittenRecords"] =
            json!(native_writes.as_ref().map_or(0, |(written, _)| *written));
        snapshot.provider_recovery["cacheAvailable"] = json!(cache_available);
        snapshot.provider_recovery["cacheWrittenRecords"] = json!(writes.unwrap_or(0));
        snapshot.tools.sort_by(|a, b| {
            a.call
                .timestamp
                .cmp(&b.call.timestamp)
                .then(a.id.cmp(&b.id))
        });
        if snapshot.tools.len() > MAX_ACTIVITY {
            snapshot.activity_truncated = true;
            snapshot.tools.drain(..snapshot.tools.len() - MAX_ACTIVITY);
        }
        snapshot.ready = true;
        snapshot.updated_at = Local::now().timestamp_millis();
        snapshot.elapsed_ms = started.elapsed().as_millis();
        snapshot
    }
}

impl FileState {
    fn expire_observations(&mut self, cutoff: NaiveDate) {
        let events_before = self.events.len();
        self.events.retain(|event| {
            if event.date >= cutoff {
                return true;
            }
            if event.terminal_response.is_none()
                && let Some(position) = event.rollout_position
                && self
                    .expired_count
                    .is_none_or(|(previous, _)| position > previous)
            {
                self.expired_count = Some((position, event.total));
            }
            false
        });
        if self.events.len() < events_before
            && self.events.capacity() > self.events.len().saturating_mul(2).max(16)
        {
            self.events.shrink_to_fit();
        }
    }

    fn inherited_context(&mut self, ordinal: u64) -> &mut InheritedUsageContext {
        if self.inherited_contexts.is_empty() {
            self.inherited_contexts.push(InheritedUsageContext {
                session: self.history_boundary.parent.clone().unwrap(),
                end: self.history_boundary.start.unwrap(),
                ..InheritedUsageContext::default()
            });
        }
        while self.inherited_contexts.len() > 1
            && ordinal >= self.inherited_contexts.last().unwrap().end
        {
            self.inherited_contexts.pop();
        }
        loop {
            let context = self.inherited_contexts.last().unwrap();
            let Some((parent, end)) = context.prefix.as_ref().filter(|(_, end)| ordinal < *end)
            else {
                break;
            };
            if context.ambiguous
                || self.inherited_contexts.len() >= 32
                || *end > context.end
                || parent == &self.session
                || self
                    .inherited_contexts
                    .iter()
                    .any(|context| &context.session == parent)
            {
                self.inherited_contexts.last_mut().unwrap().ambiguous = true;
                break;
            }
            self.inherited_contexts.push(InheritedUsageContext {
                session: parent.clone(),
                end: *end,
                ..InheritedUsageContext::default()
            });
        }
        self.inherited_contexts.last_mut().unwrap()
    }

    fn read_with_budget(
        &mut self,
        path: &Path,
        cutoff: NaiveDate,
        budget: &mut index_budget::Budget,
    ) -> io::Result<bool> {
        self.read_source_batch(path, cutoff, budget)
            .map(|(complete, _)| complete)
    }

    fn read_source_batch(
        &mut self,
        path: &Path,
        cutoff: NaiveDate,
        budget: &mut index_budget::Budget,
    ) -> io::Result<(bool, bool)> {
        let observed = Self::source_stamp(path)?;
        let unchanged =
            self.stamp == Some(observed) && self.reader.is_none() && !self.resume_pending;
        let mut file = if unchanged {
            None
        } else {
            if fs::canonicalize(path)? != path {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "rollout path changed",
                ));
            }
            Some(File::open(path)?)
        };
        let stamp = if let Some(file) = &file {
            let metadata = file.metadata()?;
            let identity = Some(codex_plus_core::codex_sqlite::opened_file_identity(file)?);
            if identity != observed.2 || fs::canonicalize(path)? != path {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "rollout identity changed",
                ));
            }
            (metadata.len(), metadata.modified().ok(), identity)
        } else {
            observed
        };
        let compressed = path.extension().is_some_and(|ext| ext == "zst");
        self.expire_observations(cutoff);
        let tools_before = self.tools.len();
        self.tools.retain(|tool| tool.date >= cutoff);
        if self.tools.len() < tools_before
            && self.tools.capacity() > self.tools.len().saturating_mul(2).max(16)
        {
            self.tools.shrink_to_fit();
        }
        self.calls.retain(|_, call| {
            DateTime::parse_from_rfc3339(&call.timestamp)
                .is_ok_and(|time| time.with_timezone(&Local).date_naive() >= cutoff)
        });
        if self.calls.capacity() > self.calls.len().saturating_mul(4).max(64) {
            self.calls.shrink_to_fit();
        }
        self.exec_skills.retain(|_, pending| {
            DateTime::parse_from_rfc3339(&pending.call.timestamp)
                .is_ok_and(|time| time.with_timezone(&Local).date_naive() >= cutoff)
        });
        self.exec_polls
            .retain(|_, session| self.exec_skills.contains_key(session));
        if self.exec_skills.capacity() > self.exec_skills.len().saturating_mul(4).max(64) {
            self.exec_skills.shrink_to_fit();
        }
        if self.exec_polls.capacity() > self.exec_polls.len().saturating_mul(4).max(64) {
            self.exec_polls.shrink_to_fit();
        }
        let mut meter = index_budget::Meter::new(self);
        let cursor_bytes = if self.reader.is_some() {
            index_budget::CURSOR_BYTES
        } else {
            0
        };
        meter.refresh(self, cursor_bytes);
        if self.stamp == Some(stamp) && self.reader.is_none() && !self.resume_pending {
            return Ok((self.pending.is_empty() && !self.skipping, false));
        }
        let replaced = self.source_replaced(stamp, compressed);
        if replaced {
            *self = Self::default();
            meter = index_budget::Meter::new(self);
            meter.refresh(self, 0);
        }
        if self.stamp.is_none() && self.offset == 0 {
            self.token_day_span = Some(TokenDaySpan::default());
        }
        if self.reader.is_none() {
            if !budget.can_open(self) {
                return Ok((false, replaced));
            }
            let mut file = file
                .take()
                .ok_or_else(|| io::Error::other("missing rollout reader"))?;
            if !compressed {
                file.seek(SeekFrom::Start(self.offset))?;
            } else if self.resume_pending {
                // 重开压缩流须跳过已提交的解压字节，但不重放计数。
                self.replay_remaining = self.offset;
            }
            self.reader = Some(index_budget::ReadCursor::new(file, compressed)?);
            self.resume_pending = false;
            self.retained_charge += index_budget::CURSOR_BYTES;
        }
        self.stamp = Some(stamp);
        let mut cursor = self.reader.take().unwrap();
        cursor.set_allowance(budget.source_left);
        let mut waiting_for_append = false;
        let result = (|| -> io::Result<bool> {
            let input = &mut cursor.input;
            loop {
                if self.replay_remaining != 0 {
                    let limit = budget.read_limit(self);
                    if limit == 0 {
                        return Ok(false);
                    }
                    let bytes = input.fill_buf()?;
                    if bytes.is_empty() {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "compressed rollout ended before its committed cursor",
                        ));
                    }
                    let length = self.replay_remaining.min(bytes.len().min(limit) as u64);
                    input.consume(length as usize);
                    self.replay_remaining -= length;
                    self.bytes_read += length;
                    budget.charge_read(length as usize);
                    continue;
                }
                if self.pending.last() == Some(&b'\n') {
                    if !self.accept_pending_line(cutoff, budget, &mut meter) {
                        return Ok(false);
                    }
                }
                if !budget.can_read(self) {
                    return Ok(false);
                }
                let bytes = input.fill_buf()?;
                if bytes.is_empty() {
                    if compressed && !self.pending.is_empty() {
                        if !self.accept_pending_line(cutoff, budget, &mut meter) {
                            return Ok(false);
                        }
                    }
                    waiting_for_append = !compressed && (!self.pending.is_empty() || self.skipping);
                    return Ok(self.pending.is_empty() && !self.skipping);
                }
                let length = bytes
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(bytes.len(), |pos| pos + 1)
                    .min(budget.read_limit(self));
                if length == 0 {
                    return Ok(false);
                }
                let part = &bytes[..length];
                let complete = part.last() == Some(&b'\n');
                let pending_capacity = self.pending.capacity();
                if !self.skipping {
                    if self.pending.len() + part.len() <= MAX_LINE {
                        if !self.pending.try_extend(part) {
                            return Ok(false);
                        }
                    } else {
                        let affects_usage = oversized::may_affect_usage(&self.pending);
                        self.issues += usize::from(affects_usage);
                        self.activity_truncated |= !affects_usage;
                        self.pending_response = None;
                        self.pending.clear();
                        self.skipping = true;
                    }
                }
                self.offset += length as u64;
                self.bytes_read += length as u64;
                budget.charge_read(length);
                self.retained_charge =
                    self.retained_charge.saturating_sub(pending_capacity) + self.pending.capacity();
                if complete && self.skipping {
                    self.skipping = false;
                }
                input.consume(length);
            }
        })();
        budget.source_left = cursor.allowance();
        match result {
            // WouldBlock 是输入预算耗尽，不是损坏流；保留解压器内部状态供下轮续读。
            Err(ref error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.reader = Some(cursor);
                Ok((false, replaced))
            }
            Ok(false) => {
                if waiting_for_append {
                    meter.refresh(self, 0);
                } else {
                    self.reader = Some(cursor);
                }
                Ok((false, replaced))
            }
            other => {
                if self.pending.is_empty() {
                    self.pending.clear();
                }
                {
                    // 文件读完后释放扩容余量，给后续日期的计量记录留出配额。
                    self.events.shrink_to_fit();
                    self.tools.shrink_to_fit();
                    self.calls.shrink_to_fit();
                    self.exec_skills.shrink_to_fit();
                    self.exec_polls.shrink_to_fit();
                    self.inherited_contexts.shrink_to_fit();
                }
                meter.refresh(self, 0);
                other.map(|complete| (complete, replaced))
            }
        }
    }

    fn source_stamp(path: &Path) -> io::Result<(u64, Option<SystemTime>, Option<(u64, u64)>)> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "rollout path changed",
            ));
        }
        Ok((
            metadata.len(),
            metadata.modified().ok(),
            observed_file_identity(path, &metadata)?,
        ))
    }

    fn source_replaced(
        &self,
        stamp: (u64, Option<SystemTime>, Option<(u64, u64)>),
        compressed: bool,
    ) -> bool {
        self.stamp.is_some_and(|old| {
            old != stamp
                && (compressed
                    || self.invalid_counts
                    || old.0 > stamp.0
                    || old.2 != stamp.2
                    || old.0 == stamp.0 && old.1 != stamp.1)
        })
    }

    fn accept_pending_line(
        &mut self,
        cutoff: NaiveDate,
        budget: &index_budget::Budget,
        meter: &mut index_budget::Meter,
    ) -> bool {
        let accepted = budget.accept_line(self);
        let skip_for_memory = !accepted && budget.can_skip_activity_line(self);
        if !accepted && !skip_for_memory {
            if !budget.line_exceeds_budget(self) {
                return false;
            }
            // 即使空索引也无法容纳的行不能无限重试；标记不完整并继续后续记录。
            self.pending_response = None;
            self.pending.clear();
            self.issues += 1;
        } else {
            let line = std::mem::take(&mut self.pending);
            self.skip_activity = skip_for_memory || !budget.can_accept_activity(self);
            self.accept(&line, cutoff);
            if line.capacity() < 256 * 1024 {
                self.pending = line;
                self.pending.clear();
            }
        }
        self.skipping = false;
        meter.refresh(self, index_budget::CURSOR_BYTES);
        true
    }

    fn accept(&mut self, line: &[u8], cutoff: NaiveDate) {
        if self.identity_conflict {
            return;
        }
        #[derive(Deserialize)]
        struct Envelope<'a> {
            #[serde(rename = "type")]
            kind: &'a str,
            timestamp: Option<&'a str>,
            ordinal: Option<i64>,
            #[serde(borrow)]
            payload: &'a RawValue,
        }
        let Ok(envelope) = serde_json::from_slice::<Envelope>(line) else {
            if !line.iter().all(u8::is_ascii_whitespace) {
                self.pending_response = None;
                self.issues += 1;
            }
            return;
        };
        let ordinal = envelope.ordinal.and_then(|n| u64::try_from(n).ok());
        if let Some(ordinal) = ordinal {
            let extent = self.rollout_extent.get_or_insert((ordinal, ordinal, 0));
            self.rollout_out_of_order |= extent.2 != 0 && ordinal != extent.1 + 1;
            extent.1 = ordinal;
            extent.2 = self.offset;
        } else if self.rollout_extent.is_some() {
            self.rollout_out_of_order = true;
        }
        let inherited = self.history_boundary.contains(ordinal);
        if envelope.kind == "token_usage_record" {
            if inherited || line.len() > MAX_NATIVE_RECEIPT_LINE {
                self.pending_response = Some(PendingUsageReceipt::Unusable);
                return;
            }
            let (Ok(payload), Some(time)) = (
                serde_json::from_str::<Value>(envelope.payload.get()),
                envelope.timestamp.and_then(|at| DateTime::parse_from_rfc3339(at).ok()),
            ) else {
                self.pending_response = Some(PendingUsageReceipt::Unusable);
                return;
            };
            // 保留相邻累计记录的供应商证据；持久索引独立配对回执与迟到的计数。
            let parsed = (|| {
                if self.pending_response.is_some()
                    || payload["thread_id"] != self.session
                    || self.turn.is_empty()
                    || payload["turn_id"] != self.turn
                {
                    return None;
                }
                let response = payload["response_id"].as_str().filter(|id| {
                    !id.is_empty() && id.len() <= 256 && id.bytes().all(|byte| byte.is_ascii_graphic())
                })?;
                let usage = Tokens::parse(&payload["usage"])?;
                let usage = usage.normalize_cache_input(
                    CacheAccounting::parse(&payload["usage"], usage),
                    None,
                    None,
                );
                Some(PendingUsageReceipt::One {
                    id: response.into(),
                    at: time.timestamp_millis(),
                    usage,
                    total: Tokens::parse(&payload["thread_token_usage"])?,
                })
            })();
            self.pending_response = Some(parsed.unwrap_or(PendingUsageReceipt::Unusable));
            let date = time.with_timezone(&Local).date_naive();
            if (self.history_boundary.parent.is_none() || ordinal.is_some())
                && let Some(mut receipt) = terminal_receipts::parse(
                    self,
                    &payload,
                    time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    date,
                )
            {
                self.record_token_day(date);
                if date >= cutoff {
                    if self.events.len() >= MAX_EVENTS {
                        self.issues += 1;
                    } else {
                        receipt.rollout_position = ordinal.map(|ordinal| {
                            (
                                ordinal,
                                self.offset.saturating_sub(line.len() as u64),
                                self.offset,
                            )
                        });
                        self.events.push(receipt);
                    }
                }
            }
            return;
        }
        if matches!(envelope.kind, "session_meta" | "turn_context") {
            self.pending_response = None;
        }
        if !matches!(
            envelope.kind,
            "session_meta" | "turn_context" | "event_msg" | "response_item"
        ) {
            return;
        }
        // 忽略消息/推理正文，而不是把多 MB 的文本分配成 Value。
        if envelope.kind == "response_item" {
            #[derive(Deserialize)]
            struct Item<'a> {
                #[serde(rename = "type")]
                kind: &'a str,
                call_id: Option<&'a str>,
            }
            let Ok(item) = serde_json::from_str::<Item>(envelope.payload.get()) else {
                return;
            };
            if !matches!(
                item.kind,
                "function_call"
                    | "custom_tool_call"
                    | "function_call_output"
                    | "custom_tool_call_output"
            ) {
                return;
            }
            // 活动配额用尽只截断工具记录，仍读取后续 token 和累计水位。
            if self.skip_activity {
                self.activity_truncated = true;
                return;
            }
            if item.kind.ends_with("_output")
                && item.call_id.is_none_or(|id| {
                    !self.calls.contains_key(id) && !self.exec_polls.contains_key(id)
                })
            {
                return;
            }
        } else if envelope.kind == "event_msg" {
            #[derive(Deserialize)]
            struct Event<'a> {
                #[serde(rename = "type")]
                kind: &'a str,
            }
            let Ok(event) = serde_json::from_str::<Event>(envelope.payload.get()) else {
                self.pending_response = None;
                return;
            };
            if event.kind.starts_with("thread_settings")
                || matches!(
                    event.kind,
                    "task_started"
                        | "turn_started"
                        | "turn_aborted"
                        | "task_aborted"
                        | "task_complete"
                        | "turn_completed"
                        | "task_failed"
                )
            {
                self.pending_response = None;
            }
            if event.kind != "token_count"
                && !event.kind.starts_with("thread_settings")
                && !matches!(event.kind, "task_started" | "turn_started")
            {
                return;
            }
        }
        let parsed = if matches!(envelope.kind, "session_meta" | "turn_context") {
            context_fields(envelope.payload.get()).and_then(|fields| {
                fields
                    .into_iter()
                    .filter_map(|(key, value)| value.map(|value| (key, value)))
                    .map(|(key, value)| {
                        serde_json::from_str(value.get()).map(|value| (key.to_owned(), value))
                    })
                    .collect::<serde_json::Result<serde_json::Map<String, Value>>>()
                    .map(Value::Object)
            })
        } else {
            serde_json::from_str::<Value>(envelope.payload.get())
        };
        let Ok(payload) = parsed else {
            self.pending_response = None;
            self.issues += 1;
            return;
        };
        if envelope.kind == "session_meta" {
            let id = payload["id"]
                .as_str()
                .and_then(super::normalize_session_id)
                .or_else(|| {
                    payload["session_id"]
                        .as_str()
                        .and_then(super::normalize_session_id)
                });
            let role = if payload["source"].get("subagent").is_some()
                || payload["source"]
                    .as_str()
                    .is_some_and(|s| s.to_ascii_lowercase().contains("subagent"))
            {
                "subagent"
            } else {
                "main"
            };
            if inherited {
                self.legacy_header = None;
                let context = self.inherited_context(ordinal.unwrap());
                if id.as_ref() != Some(&context.session) {
                    context.ambiguous = true;
                    return;
                }
                context.provider = payload["model_provider"].as_str().unwrap_or("").to_owned();
                context.role = role.into();
                let boundary = super::UsageHistoryBoundary::from_meta(&payload, &context.session);
                // 复制头的原始 ordinal 为 0；其局部边界平移到当前文件的 ordinal。
                context.prefix = boundary
                    .parent
                    .zip(boundary.start)
                    .and_then(|(parent, start)| {
                        ordinal.unwrap().checked_add(start).map(|end| (parent, end))
                    });
                if context
                    .prefix
                    .as_ref()
                    .is_some_and(|(_, end)| *end <= ordinal.unwrap() || *end > context.end)
                    || ["forked_from_id", "parent_thread_id"]
                        .iter()
                        .any(|key| payload[*key].as_str().is_some_and(|id| !id.is_empty()))
                        && context.prefix.is_none()
                {
                    context.ambiguous = true;
                }
                if !context.ambiguous {
                    self.legacy_header = id.map(|session| (session, role.into()));
                }
                return;
            }
            if let Some(id) = id {
                if !self.session.is_empty() && self.session != id {
                    self.identity_conflict = true;
                    self.invalid_counts = true;
                    self.issues += 1;
                    return;
                }
                self.session = id;
                let parent = (role == "subagent")
                    .then(|| {
                        payload["source"]["subagent"]["thread_spawn"]["parent_thread_id"]
                            .as_str()
                            .or_else(|| payload["parent_thread_id"].as_str())
                    })
                    .flatten()
                    .and_then(super::normalize_session_id);
                self.scope_parent_candidates
                    .entry(self.session.clone())
                    .or_default()
                    .insert(parent);
                self.role = role.into();
            }
            self.history_boundary = super::UsageHistoryBoundary::from_meta(&payload, &self.session);
            self.continuation = payload["history_base"]["end_ordinal_exclusive"]
                .as_u64()
                .is_some_and(|ordinal| ordinal > 0);
            self.history_base = segments::HistoryBase::parse(&payload["history_base"], ordinal);
            self.inherited_contexts.clear();
            self.legacy_header = None;
            self.provider = payload["model_provider"].as_str().unwrap_or("").to_owned();
            return;
        }
        let event_type = payload["type"].as_str().unwrap_or("");
        let receipt = if event_type == "token_count" {
            if payload.get("info") == Some(&Value::Null) && payload["rate_limits"].is_object() {
                return;
            }
            self.pending_response.take()
        } else {
            None
        };
        if self.session.is_empty() {
            if event_type == "token_count" {
                self.issues += 1;
                self.invalid_counts = true;
            }
            return;
        }
        let Some(time) = envelope
            .timestamp
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        else {
            if event_type == "token_count"
                || envelope.kind == "turn_context"
                || event_type.starts_with("thread_settings")
            {
                self.issues += 1;
                self.invalid_counts = true;
            }
            return;
        };
        let timestamp = time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let date = time.with_timezone(&Local).date_naive();
        if envelope.kind == "turn_context" || event_type.starts_with("thread_settings") {
            let observation = {
                let (session, role, provider, model, model_unavailable, turn, ambiguous) =
                    if inherited {
                        let context = self.inherited_context(ordinal.unwrap());
                        (
                            &context.session,
                            &context.role,
                            &mut context.provider,
                            &mut context.model,
                            &mut context.model_unavailable,
                            &mut context.turn,
                            context.ambiguous,
                        )
                    } else {
                        (
                            &self.session,
                            &self.role,
                            &mut self.provider,
                            &mut self.model,
                            &mut self.model_unavailable,
                            &mut self.turn,
                            false,
                        )
                    };
                let updated_provider = payload["model_provider"]
                    .as_str()
                    .or_else(|| payload["modelProvider"].as_str())
                    .or_else(|| payload["thread_settings"]["model_provider"].as_str())
                    .or_else(|| payload["thread_settings"]["modelProvider"].as_str())
                    .filter(|provider| !provider.is_empty() && provider.len() <= 200);
                if let Some(value) = updated_provider {
                    *provider = value.to_owned();
                }
                let updated_model = payload["model"]
                    .as_str()
                    .or_else(|| payload["thread_settings"]["model"].as_str());
                if let Some(value) = updated_model {
                    if value.len() <= 256 {
                        *model = value.to_owned();
                        *model_unavailable = false;
                    } else {
                        model.clear();
                        *model_unavailable = true;
                    }
                }
                if let Some(value) = payload["turn_id"].as_str() {
                    *turn = value.to_owned();
                }
                ((updated_model.is_some() || updated_provider.is_some()) && !ambiguous).then(|| {
                    (
                        session.clone(),
                        model.clone(),
                        provider.clone(),
                        role.clone(),
                        *model_unavailable,
                    )
                })
            };
            if let Some((session, model, provider, role, model_unavailable)) = observation {
                let at = time.timestamp_millis();
                let observation = self.model_observations.entry(session).or_insert_with(|| {
                    SessionModelObservation {
                        at,
                        model: String::new(),
                        provider: String::new(),
                        role: String::new(),
                        unavailable: false,
                        had_unavailable: false,
                        unordered: false,
                    }
                });
                observation.unordered |= at < observation.at;
                observation.at = at;
                observation.model = model;
                observation.provider = provider;
                observation.role = role;
                observation.unavailable = model_unavailable;
                observation.had_unavailable |= model_unavailable;
            }
        }
        if matches!(event_type, "task_started" | "turn_started") {
            if let Some(turn) = payload["turn_id"].as_str() {
                if inherited {
                    self.inherited_context(ordinal.unwrap()).turn = turn.to_owned();
                } else {
                    self.turn = turn.to_owned();
                }
            }
        }
        if envelope.kind == "response_item" && date >= cutoff {
            if !inherited {
                self.accept_tool(payload, timestamp, date);
            }
            return;
        }
        if event_type != "token_count" {
            return;
        }
        let info = &payload["info"];
        let raw_cumulative = &info["total_token_usage"];
        let raw_request = &info["last_token_usage"];
        // 先记录范围，再按保留期丢弃明细；冷索引也能找到历史原会话。
        if !raw_cumulative.is_null() || !raw_request.is_null() {
            self.record_token_day(date);
        }
        let Some(raw_total) = Tokens::parse(raw_cumulative) else {
            if info.is_object()
                && (!info["total_token_usage"].is_null() || !info["last_token_usage"].is_null())
            {
                self.issues += 1;
                self.malformed_token_counts += usize::from(!inherited);
                self.invalid_counts = true;
            }
            return;
        };
        let raw_last = Tokens::parse(raw_request);
        let cumulative = CacheAccounting::parse(raw_cumulative, raw_total);
        let last_usage = raw_last.and_then(|last| CacheAccounting::parse(raw_request, last));
        let total = raw_total.normalize_cache_input(
            cumulative,
            self.previous_cache_accounting.zip(self.previous),
            last_usage,
        );
        let last = raw_last.map(|last| last.normalize_cache_input(last_usage, None, None));
        let has_file_baseline = self.previous.is_some();
        let count_uncertain = last.is_none()
            && self
                .previous
                .is_some_and(|before| total.requires_last_after(before));
        let delta = total.usage_delta(self.previous, last);
        self.previous = Some(total);
        self.previous_cache_accounting = cumulative;
        let rollout_position = ordinal.map(|ordinal| {
            (
                ordinal,
                self.offset.saturating_sub(line.len() as u64),
                self.offset,
            )
        });
        if date < cutoff {
            if let Some(position) = rollout_position {
                self.expired_count = Some((position, total));
            }
            return;
        }
        if self.events.len() >= MAX_EVENTS {
            self.issues += 1;
            return;
        }
        let (owner, model, model_unavailable, turn, provider, role) = if inherited {
            let context = self.inherited_context(ordinal.unwrap());
            if context.ambiguous {
                self.issues += 1;
                return;
            }
            (
                context.session.clone(),
                context.model.clone(),
                context.model_unavailable,
                context.turn.clone(),
                context.provider.clone(),
                context.role.clone(),
            )
        } else {
            (
                self.session.clone(),
                self.model.clone(),
                self.model_unavailable,
                self.turn.clone(),
                self.provider.clone(),
                self.role.clone(),
            )
        };
        // 补证与冲突缓存用旧口径生成的 ID；归一后的数量只参与记账。
        let identity = serde_json::to_vec(&json!([raw_total, raw_last])).unwrap_or_default();
        let id_for = |session: &str| -> [u8; 32] {
            let mut digest = Sha256::new();
            digest.update(session.as_bytes());
            digest.update(timestamp.as_bytes());
            digest.update(&identity);
            digest.finalize().into()
        };
        let id = id_for(&owner);
        let legacy_identity =
            self.legacy_header
                .as_ref()
                .filter(|_| !inherited)
                .map(|(session, role)| {
                    Box::new(LegacyIdentity {
                        id: id_for(session),
                        session: session.clone(),
                        role: role.clone(),
                    })
                });
        self.events.push(UsageEvent {
            native_response_id: receipt.and_then(|receipt| {
                let PendingUsageReceipt::One {
                    id,
                    at,
                    usage,
                    total: receipt_total,
                } = receipt
                else {
                    return None;
                };
                let compatible = |a: Tokens, b: Tokens| {
                    a.input_tokens == b.input_tokens
                        && a.output_tokens == b.output_tokens
                        && (!a.cached_known
                            || !b.cached_known
                            || a.cached_input_tokens == b.cached_input_tokens)
                        && a.reasoning_output_tokens
                            .zip(b.reasoning_output_tokens)
                            .is_none_or(|(a, b)| a == b)
                };
                let full_counts = total.cached_known
                    && receipt_total.cached_known
                    && total.same_counts(receipt_total)
                    && delta.cached_known
                    && usage.cached_known
                    && delta.same_counts(usage)
                    && last.is_some_and(|tokens| tokens.cached_known && tokens.same_counts(usage));
                (!inherited
                    && !count_uncertain
                    && time.timestamp_millis() >= at
                    && time.timestamp_millis().saturating_sub(at) <= REQUEST_MATCH_WINDOW_MS
                    && compatible(total, receipt_total)
                    && compatible(delta, usage)
                    && last.is_some_and(|tokens| compatible(tokens, usage)))
                .then_some(NativeResponseIdentity { id, full_counts })
            }),
            terminal_response: None,
            id,
            original_id: id,
            session: owner,
            turn,
            model,
            timestamp,
            date,
            total,
            delta,
            has_file_baseline,
            last,
            count_uncertain,
            count_incomplete: false,
            count_pending: false,
            rollout_position,
            provider,
            provider_source: None,
            role,
            inherited,
            legacy_identity,
            context_conflicts: ContextConflicts {
                model: model_unavailable,
                ..ContextConflicts::default()
            },
        });
    }

    fn record_token_day(&mut self, date: NaiveDate) {
        if let Some(span) = &mut self.token_day_span {
            let date = date.num_days_from_ce();
            span.first = Some(span.first.map_or(date, |day| day.min(date)));
            span.last = Some(span.last.map_or(date, |day| day.max(date)));
        }
    }

    fn accept_tool(&mut self, mut payload: Value, timestamp: String, date: NaiveDate) {
        let Some(id) = payload["call_id"]
            .as_str()
            .or_else(|| payload["id"].as_str())
            .filter(|id| id.len() <= 300)
            .map(str::to_owned)
        else {
            return;
        };
        let kind = payload["type"].as_str().unwrap_or("").to_owned();
        if matches!(kind.as_str(), "function_call" | "custom_tool_call") {
            let name = payload["name"].as_str().unwrap_or("");
            let namespace = payload["namespace"].as_str().unwrap_or("");
            let arguments = payload["arguments"]
                .as_str()
                .or_else(|| payload["input"].as_str())
                .unwrap_or("");
            if name.rsplit('.').next() == Some("write_stdin") {
                #[derive(Deserialize)]
                struct PollArgs {
                    session_id: Option<u64>,
                }
                if let Ok(args) = serde_json::from_str::<PollArgs>(arguments) {
                    let session = args.session_id.map(|id| id.to_string());
                    if let Some(session) = session.filter(|id| self.exec_skills.contains_key(id)) {
                        if self.exec_polls.len() >= 512 {
                            self.exec_polls.clear();
                        }
                        self.exec_polls.insert(id, session);
                    }
                }
                return;
            }
            if !name.starts_with("mcp__")
                && !namespace.starts_with("mcp__")
                && !arguments.to_lowercase().contains("skill.md")
            {
                return;
            }
            if arguments.len() > 64_000 {
                self.issues += 1;
                return;
            }
            payload = tool_receipts::call(&payload);
            if self.calls.len() >= 512 {
                if let Some(oldest) = self
                    .calls
                    .iter()
                    .min_by(|a, b| a.1.timestamp.cmp(&b.1.timestamp))
                    .map(|(id, _)| id.clone())
                {
                    self.calls.remove(&oldest);
                }
            }
            self.calls.insert(
                id,
                Call {
                    timestamp,
                    session: self.session.clone(),
                    payload,
                },
            );
        } else if matches!(
            kind.as_str(),
            "function_call_output" | "custom_tool_call_output"
        ) {
            let session = self.exec_polls.remove(&id);
            let asynchronous = session
                .as_ref()
                .and_then(|key| self.exec_skills.remove(key));
            let (call, evidence) = if let Some(pending) = asynchronous {
                (Some(pending.call), pending.evidence)
            } else {
                (self.calls.remove(&id), String::new())
            };
            let Some(call) = call else {
                return;
            };
            let skill_read = call.payload["arguments"]["skillNames"]
                .as_array()
                .is_some_and(|names| !names.is_empty());
            let shell_skill = skill_read
                && call.payload["name"].as_str().is_some_and(|name| {
                    name.contains("exec") || name.contains("shell") || name.contains("terminal")
                });
            let receipt = exec_receipt(&payload["output"]);
            let failed = payload["isError"] == true
                || payload["error"].is_object()
                || payload["error"].is_string()
                || payload["output"]["success"] == false
                || payload["output"]["isError"] == true
                || payload["output"]["error"].is_object()
                || payload["output"]["error"].is_string()
                || matches!(
                    payload["output"]["status"].as_str(),
                    Some("failed" | "error" | "cancelled" | "canceled" | "interrupted")
                )
                || ["exit_code", "exitCode"].iter().any(|key| {
                    payload["output"][key]
                        .as_i64()
                        .is_some_and(|code| code != 0)
                })
                || payload["output"].as_str().is_some_and(|text| {
                    text.trim_start().starts_with("Error")
                        || text.trim_start().starts_with("Tool call error:")
                });
            let exited = receipt.as_ref().is_some_and(|(running, _)| !running)
                || payload["output"]["exit_code"].is_number()
                || payload["output"]["exitCode"].is_number()
                || payload["output"]["success"] == true;
            if shell_skill
                && !failed
                && (receipt.as_ref().is_some_and(|r| r.0) || session.is_some() && !exited)
            {
                let session = session.or_else(|| receipt.map(|(_, id)| id)).unwrap();
                let evidence = compact_evidence(&evidence, &payload["output"]);
                if self.exec_skills.len() >= 256 {
                    if let Some(oldest) = self
                        .exec_skills
                        .iter()
                        .min_by(|a, b| a.1.call.timestamp.cmp(&b.1.call.timestamp))
                        .map(|(id, _)| id.clone())
                    {
                        self.exec_skills.remove(&oldest);
                    }
                }
                let mut call = call;
                // 后台完成时间包含轮询等待，不能作为同步技能读取耗时。
                call.payload["arguments"]["singleRead"] = json!(false);
                self.exec_skills
                    .insert(session, ExecSkill { call, evidence });
                return;
            }
            payload["output"] = tool_receipts::compact(&payload["output"], 0, skill_read);
            let id = call.payload["call_id"]
                .as_str()
                .or_else(|| call.payload["id"].as_str())
                .unwrap_or(&id)
                .to_owned();
            // 不返回完整工具输入/输出正文，保持调用身份与失败判定所需字段。
            let mut output = json!({"type":kind,"call_id":id,"timestamp":timestamp,"output":payload["output"],
                "isError":payload["isError"],"error":payload["error"].is_object() || payload["error"].is_string()});
            if !evidence.is_empty() {
                output["skillReadEvidence"] = json!(evidence);
            }
            let date = DateTime::parse_from_rfc3339(&call.timestamp)
                .map(|time| time.with_timezone(&Local).date_naive())
                .unwrap_or(date);
            if self.tools.len() >= MAX_ACTIVITY {
                self.tools.remove(0);
                self.issues += 1;
            }
            self.tools.push(ToolRecord {
                id,
                call,
                output,
                date,
            });
        }
    }
}

// 只识别执行器回执头，正文里的示例不能建立后台会话关联。
fn exec_receipt(value: &Value) -> Option<(bool, String)> {
    let text = value.as_str()?.trim();
    for line in text.lines() {
        let line = line.trim_end();
        if [
            "Chunk ID:",
            "Wall time:",
            "Original token count:",
            "Final output:",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        {
            continue;
        }
        if let Some(id) = line.strip_prefix("Process running with session ID ") {
            let id = id.split_whitespace().next()?;
            return (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| (true, id.to_owned()));
        }
        if line
            .strip_prefix("Process exited with code ")
            .and_then(|code| code.split_whitespace().next())
            .and_then(|code| code.parse::<i32>().ok())
            .is_some()
        {
            return Some((false, String::new()));
        }
        return None;
    }
    None
}

fn compact_evidence(previous: &str, output: &Value) -> String {
    let output = compact_output(output, 0);
    let text = format!("{previous}\n{}", output.as_str().unwrap_or(""));
    if text.chars().count() <= 6000 {
        return text;
    }
    let prefix: String = text.chars().take(4000).collect();
    let suffix: String = text
        .chars()
        .rev()
        .take(2000)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}\n{suffix}")
}

fn compact_output(value: &Value, depth: usize) -> Value {
    tool_receipts::compact(value, depth, true)
}

fn usage_provider<'a>(
    _model: &str,
    recorded: &'a str,
    providers: &BTreeMap<String, String>,
) -> &'a str {
    let recorded = recorded.trim();
    // 原始明确供应商优先；未配置的原生标签和内部标记不能充当供应商证据。
    let generic_openai =
        recorded.eq_ignore_ascii_case("openai") && !providers.contains_key(recorded);
    if !unknown_provider(recorded) && !generic_openai {
        return recorded;
    }
    // 聚合模型只代表路由意图，实际供应商可能因故障转移而不同。
    // 外部代理可沿用原生标签；未配置的标签不能阻止实际请求证据归属。
    if generic_openai {
        return "";
    }
    recorded
}

impl Snapshot {
    fn collect_scope_parents<'a>(&mut self, files: impl Iterator<Item = &'a FileState>) {
        self.collect_scope_candidates(files.filter_map(|file| {
            file.scope_parent_candidates
                .get(&file.session)
                .map(|parents| (file.session.as_str(), parents))
        }));
    }

    fn collect_scope_candidates<'a>(
        &mut self,
        candidates_by_source: impl Iterator<Item = (&'a str, &'a HashSet<Option<String>>)>,
    ) {
        let mut candidates: BTreeMap<String, HashSet<Option<String>>> = BTreeMap::new();
        for (session, parents) in candidates_by_source {
            let entries = candidates.entry(session.to_owned()).or_default();
            for parent in parents {
                entries.insert(
                    parent
                        .as_ref()
                        .map(|id| self.aliases.get(id).unwrap_or(id).clone()),
                );
            }
        }
        let parents: BTreeMap<String, String> = candidates
            .into_iter()
            .filter_map(|(child, parents)| {
                if parents.len() != 1 {
                    return None;
                }
                parents
                    .into_iter()
                    .next()
                    .flatten()
                    .map(|parent| (child, parent))
            })
            .collect();
        // 只关闭不可信归属边；原有用量事件、继承历史去重与全局状态均不受影响。
        self.scope_parents = parents
            .iter()
            .filter(|(child, _)| {
                let mut seen = HashSet::new();
                let mut node = child.as_str();
                while let Some(parent) = parents.get(node) {
                    if !seen.insert(node) {
                        return false;
                    }
                    node = parent;
                }
                true
            })
            .map(|(child, parent)| (child.clone(), parent.clone()))
            .collect();
        for (child, parent) in &self.scope_parents {
            if let Some(info) = self.session_info.get_mut(child) {
                info["parentSessionId"] = json!(parent);
            }
        }
    }

    fn scope_members<'a>(&'a self, requested: &'a str) -> (&'a str, HashSet<String>) {
        let session = self
            .aliases
            .get(requested)
            .map_or(requested, String::as_str);
        let mut sessions = HashSet::new();
        if self.sessions.contains(session) {
            sessions.insert(session.to_owned());
            let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for (child, parent) in &self.scope_parents {
                children.entry(parent).or_default().push(child);
            }
            let mut pending = vec![session];
            while let Some(parent) = pending.pop() {
                for &child in children.get(parent).into_iter().flatten() {
                    if sessions.insert(child.to_owned()) {
                        pending.push(child);
                    }
                }
            }
        }
        (session, sessions)
    }

    fn scoped_usage(&self, requested: &str, query: &Query, bounds: Option<(i64, i64)>) -> Value {
        let (session, sessions) = self.scope_members(requested);
        let ids = ordered_scope_ids(session, &sessions);
        json!({"sessionId":session,"sessionIds":ids,"days":self.usage_days(query, bounds, Some(&sessions))})
    }

    fn usage_days(
        &self,
        query: &Query,
        bounds: Option<(i64, i64)>,
        sessions: Option<&HashSet<String>>,
    ) -> Vec<Value> {
        if !self.count_cache_available {
            return Vec::new();
        }
        let mut days: BTreeMap<NaiveDate, Aggregate> = BTreeMap::new();
        let mut models: BTreeMap<(NaiveDate, String), Aggregate> = BTreeMap::new();
        let mut breakdowns: BTreeMap<
            (NaiveDate, String, String, String, bool, bool),
            (Aggregate, Tokens),
        > = BTreeMap::new();
        let mut day_proofs = BTreeMap::<NaiveDate, Sha256>::new();
        let mut breakdown_proofs = BTreeMap::<
            (NaiveDate, String, String, String, bool, bool),
            (Sha256, ClassificationEvidence),
        >::new();
        let mut ordered_events: Vec<_> = self.events.iter().collect();
        ordered_events.sort_unstable_by_key(|event| event.id);
        for event in ordered_events {
            if event.count_uncertain || event.count_incomplete || event.delta.total_tokens == 0 {
                continue;
            }
            if sessions.is_some_and(|ids| !ids.contains(&event.session)) {
                continue;
            }
            let at = DateTime::parse_from_rfc3339(&event.timestamp)
                .map(|time| time.timestamp_millis())
                .unwrap_or(0);
            if bounds.is_some_and(|(start, end)| start >= end || at < start || at >= end) {
                continue;
            }
            let record = Record {
                session_id: event.session.clone(),
                turn_id: event.turn.clone(),
                model: Some(event.model.clone()).filter(|m| !m.is_empty()),
                date: Some(event.date),
                usage: Some(event.delta),
            };
            if !query.accepts(&record) {
                continue;
            }
            days.entry(event.date).or_default().add(&record);
            models
                .entry((event.date, event.model.clone()))
                .or_default()
                .add(&record);
            let key = (
                event.date,
                event.model.clone(),
                event.provider.clone(),
                if event.context_conflicts.role {
                    "unknown".to_owned()
                } else {
                    event.role.clone()
                },
                event.context_conflicts.provider,
                event.provider_source.as_deref() == Some(REQUEST_SUPPLIER_PROOF),
            );
            let (aggregate, priceable) = breakdowns.entry(key.clone()).or_default();
            aggregate.add(&record);
            if event.delta.cached_known {
                priceable.add(event.delta);
            }
            event.update_membership_digest(day_proofs.entry(event.date).or_default());
            let evidence = ClassificationEvidence::of(event);
            let (digest, _) = breakdown_proofs
                .entry(key)
                .and_modify(|(_, shared)| *shared = shared.intersect(evidence))
                .or_insert_with(|| (Sha256::new(), evidence));
            event.update_membership_digest(digest);
        }
        days.into_iter()
            .map(|(date, aggregate)| {
                let mut row = aggregate.value();
                row["date"] = json!(date.to_string());
                if let Some(digest) = day_proofs.get(&date) {
                    row["accountingFingerprint"] =
                        json!(format!("{:x}", digest.clone().finalize()));
                }
                row["models"] = json!(
                    models
                        .range((date, String::new())..=(date, char::MAX.to_string()))
                        .map(|((_, model), aggregate)| {
                            let mut row = aggregate.value();
                            row["model"] = json!(model);
                            row
                        })
                        .collect::<Vec<_>>()
                );
                row["breakdowns"] = json!(
                    breakdowns
                        .iter()
                        .filter(|((day, _, _, _, _, _), _)| day == &date)
                        .map(
                            |(
                                key @ (_, model, provider, role, provider_conflict, _),
                                (aggregate, priceable),
                            )| {
                                let mut row = aggregate.value();
                                row["model"] = json!(model);
                                row["provider"] = json!(provider);
                                row["providerName"] = json!(self.provider_names.get(provider));
                                row["role"] = json!(role);
                                if !aggregate.usage.cached_known && priceable.total_tokens != 0 {
                                    row["priceableUsage"] = json!({
                                        "inputTokens":priceable.input_tokens,
                                        "cachedInputTokens":priceable.cached_input_tokens,
                                        "outputTokens":priceable.output_tokens,
                                    });
                                }
                                if *provider_conflict {
                                    row["providerConflict"] = json!(true);
                                }
                                if let Some((digest, evidence)) = breakdown_proofs.get(key) {
                                    row["membershipFingerprint"] =
                                        json!(format!("{:x}", digest.clone().finalize()));
                                    evidence.write_to(&mut row);
                                }
                                row
                            }
                        )
                        .collect::<Vec<_>>()
                );
                row
            })
            .collect()
    }

    fn response_page(
        &self,
        query: &Query,
        processing: bool,
        activity_page: usize,
        revision: Option<i64>,
        bounds: Option<(i64, i64)>,
    ) -> Value {
        let day_rows = self.usage_days(query, bounds, None);
        let selected_tools: Vec<_> = self
            .tools
            .iter()
            .filter(|_| query.include_activity)
            .filter(|tool| {
                query
                    .session_id
                    .as_ref()
                    .is_none_or(|id| id == &tool.call.session)
                    && query.from.is_none_or(|from| tool.date >= from)
                    && query.to.is_none_or(|to| tool.date <= to)
                    && bounds.is_none_or(|(start, end)| {
                        DateTime::parse_from_rfc3339(&tool.call.timestamp).is_ok_and(|time| {
                            time.timestamp_millis() >= start && time.timestamp_millis() < end
                        })
                    })
            })
            .collect();
        let unchanged = revision == Some(self.updated_at);
        let page_size = 512;
        let tools: Vec<_> = selected_tools
            .iter()
            .skip((activity_page - 1) * page_size)
            .take(if unchanged { 0 } else { page_size })
            .map(|tool| {
                json!({"conversationKey":tool.call.session,"timestamp":tool.call.timestamp,
            "call":tool.call.payload,"output":tool.output})
            })
            .collect();
        let uncertain_count_events = self
            .events
            .iter()
            .filter(|event| event.count_uncertain || event.count_incomplete)
            .count()
            + self.unobserved_count_conflicts;
        let complete = self.ready
            && self.errors == 0
            && self.issues == 0
            && self.count_cache_available
            && uncertain_count_events == 0;
        let mut response = json!({"status":if complete {"ok"} else {"partial"},"complete":complete,
            "updatedAt":self.updated_at,"source":"structured-local-rollout","days":day_rows,
            "accountingVersion":2,"rangeApplied":bounds.is_some(),
            "sessionInfo":self.session_info,"providerNames":self.provider_names,
            "providerRecovery":self.provider_recovery,
            "providerEvidence":self.provider_evidence,
            "supplierEvidence":self.supplier_evidence,
            "sessionIds":self.sessions.iter().collect::<Vec<_>>(),"sessionAliases":self.aliases,
            "activityRecords":tools,"activityTruncated":self.activity_truncated,
            "activityPagination":{"page":activity_page,"pages":selected_tools.len().div_ceil(page_size).max(1),
                "total":selected_tools.len(),"unchanged":unchanged},
            "scan":{"discoveredFiles":self.files,"indexedFiles":self.indexed,
                "ready":self.ready,
                "compressedFiles":self.compressed,"pendingFiles":self.files.saturating_sub(self.indexed),
                "ignoredFiles":self.ignored,
                "errors":self.errors,"malformedRecords":self.issues,"processing":processing,
                "contextConflicts":self.events.iter().filter(|event| event.context_conflicts.any()).count(),
                 "uncertainCountEvents":uncertain_count_events,
                 "unobservedCountConflicts":self.unobserved_count_conflicts,
                 "countCacheAvailable":self.count_cache_available,
                "bytesRead":self.bytes_read,"elapsedMs":self.elapsed_ms,
                "sessions":self.sessions.len(),"retentionDays":RETENTION_DAYS},
            "activityComplete":complete && !self.activity_truncated});
        if query.include_session_cursors {
            response["sessionCursors"] = json!(
                self.cursors
                    .iter()
                    .filter(|(id, _)| query.session_id.as_ref().is_none_or(|filter| filter == *id))
                    .map(|(id, (timestamp, total))| json!({
                        "sessionId": id,
                        "timestamp": timestamp,
                        "total": total.value()
                    }))
                    .collect::<Vec<_>>()
            );
        }
        response
    }
}

fn ordered_scope_ids<'a>(session: &str, sessions: &'a HashSet<String>) -> Vec<&'a str> {
    let mut ids: Vec<_> = sessions.iter().map(String::as_str).collect();
    ids.sort_unstable_by(|a, b| (a != &session, a).cmp(&(b != &session, b)));
    ids
}

#[cfg(test)]
mod regression_tests;

#[cfg(test)]
mod provider_review_tests;

#[cfg(test)]
mod backend_recovery_tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod segments_integration_tests;
