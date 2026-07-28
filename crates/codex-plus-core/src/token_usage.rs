use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCounts {
    pub input: u64,
    pub cached_input: u64,
    pub cache_write: u64,
    pub output: u64,
    pub reasoning: u64,
    pub total: u64,
}

impl UsageCounts {
    pub fn has_tokens(&self) -> bool {
        self.input > 0
            || self.cached_input > 0
            || self.cache_write > 0
            || self.output > 0
            || self.reasoning > 0
            || self.total > 0
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedResponseUsage {
    pub response_id: String,
    pub model: String,
    pub status: String,
    pub usage: UsageCounts,
    pub usage_missing: bool,
}

pub struct ResponsesSseUsageTracker {
    buffer: String,
    utf8_remainder: Vec<u8>,
    fallback: CapturedResponseUsage,
    terminal: Option<CapturedResponseUsage>,
}

impl ResponsesSseUsageTracker {
    pub fn with_request(request: &Value) -> Self {
        Self {
            buffer: String::new(),
            utf8_remainder: Vec::new(),
            fallback: CapturedResponseUsage {
                model: request
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .to_string(),
                status: "incomplete".to_string(),
                usage_missing: true,
                ..CapturedResponseUsage::default()
            },
            terminal: None,
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        append_utf8_safe(&mut self.buffer, &mut self.utf8_remainder, bytes);
        while let Some(block) = take_sse_block(&mut self.buffer) {
            self.handle_block(&block);
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.terminal.is_some()
    }

    pub fn finish(&mut self) -> CapturedResponseUsage {
        if !self.utf8_remainder.is_empty() {
            self.buffer
                .push_str(&String::from_utf8_lossy(&self.utf8_remainder));
            self.utf8_remainder.clear();
        }
        if !self.buffer.trim().is_empty() {
            let block = std::mem::take(&mut self.buffer);
            self.handle_block(&block);
        }
        self.terminal
            .take()
            .unwrap_or_else(|| self.fallback.clone())
    }

    fn handle_block(&mut self, block: &str) {
        let mut event_name = String::new();
        let mut data_parts = Vec::new();
        for line in block.lines() {
            if let Some(event) = strip_sse_field(line, "event") {
                event_name = event.trim().to_string();
            }
            if let Some(data) = strip_sse_field(line, "data") {
                data_parts.push(data.to_string());
            }
        }
        if data_parts.is_empty() {
            return;
        }
        let data = data_parts.join("\n");
        if data.trim() == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        self.remember_response_metadata(&value);
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event_name.as_str());
        let status = if event_type.ends_with(".completed") {
            "completed"
        } else if event_type.ends_with(".failed") || event_type == "error" {
            "failed"
        } else if event_type.ends_with(".incomplete") {
            "incomplete"
        } else {
            return;
        };
        self.terminal = Some(captured_response_usage_from_value(
            &value,
            &self.fallback.model,
            status,
        ));
    }

    fn remember_response_metadata(&mut self, value: &Value) {
        let response = value.get("response").unwrap_or(value);
        if let Some(id) = response
            .get("id")
            .or_else(|| value.get("response_id"))
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
        {
            self.fallback.response_id = id.to_string();
        }
        if let Some(model) = response
            .get("model")
            .or_else(|| value.get("model"))
            .and_then(Value::as_str)
            .filter(|model| !model.trim().is_empty())
        {
            self.fallback.model = model.to_string();
        }
        if let Some(status) = response
            .get("status")
            .or_else(|| value.get("status"))
            .and_then(Value::as_str)
            .filter(|status| !status.trim().is_empty())
        {
            self.fallback.status = status.to_string();
        }
    }
}

fn take_sse_block(buffer: &mut String) -> Option<String> {
    let lf = buffer.find("\n\n").map(|index| (index, 2));
    let crlf = buffer.find("\r\n\r\n").map(|index| (index, 4));
    let (index, delimiter_len) = match (lf, crlf) {
        (Some(left), Some(right)) => left.min(right),
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => return None,
    };
    let block = buffer[..index].to_string();
    buffer.drain(..index + delimiter_len);
    Some(block)
}

fn append_utf8_safe(buffer: &mut String, remainder: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let mut combined = std::mem::take(remainder);
    combined.extend_from_slice(bytes);
    match std::str::from_utf8(&combined) {
        Ok(text) => buffer.push_str(text),
        Err(error) => {
            let valid = error.valid_up_to();
            buffer.push_str(std::str::from_utf8(&combined[..valid]).unwrap_or_default());
            if error.error_len().is_none() {
                remainder.extend_from_slice(&combined[valid..]);
            } else {
                buffer.push_str(&String::from_utf8_lossy(&combined[valid..]));
            }
        }
    }
}

fn strip_sse_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RolloutEvent {
    pub id: String,
    pub session_id: String,
    pub timestamp: String,
    pub model: String,
    pub usage: UsageCounts,
    pub totals: UsageCounts,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub usage_missing: bool,
    #[serde(default)]
    pub timestamp_ms: u64,
    #[serde(default)]
    pub response_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenUsageQuery {
    pub since: String,
    pub days: u64,
    pub limit: usize,
    pub proxy_since_ms: u64,
    pub proxy_offset: u64,
    pub proxy_generation: String,
    pub include_rollout: bool,
    pub rollout_incremental: bool,
}

impl Default for TokenUsageQuery {
    fn default() -> Self {
        Self {
            since: String::new(),
            days: 7,
            limit: 100_000,
            proxy_since_ms: 0,
            proxy_offset: 0,
            proxy_generation: String::new(),
            include_rollout: true,
            rollout_incremental: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageResult {
    pub events: Vec<RolloutEvent>,
    pub warnings: Vec<String>,
    pub next_since: String,
    pub proxy_next_since_ms: u64,
    pub proxy_enabled_at_ms: u64,
    pub missing_usage: usize,
    pub proxy_next_offset: u64,
    pub proxy_generation: String,
    pub proxy_reset: bool,
}

#[derive(Default)]
pub struct RolloutTailer {
    files: HashMap<PathBuf, RolloutFileCursor>,
}

#[derive(Default)]
struct RolloutFileCursor {
    offset: u64,
    observed_len: u64,
    modified_ms: u64,
    first_line_hash: String,
    session_id: String,
    model: String,
}

impl RolloutTailer {
    pub fn reset(&mut self) {
        self.files.clear();
    }

    pub fn read_from_roots(
        &mut self,
        roots: &[PathBuf],
        query: &TokenUsageQuery,
    ) -> TokenUsageResult {
        let days = query.days.clamp(1, 31);
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(days * 24 * 60 * 60))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut paths = Vec::new();
        for root in roots {
            collect_rollout_paths(root, cutoff, &mut paths);
        }
        paths.sort();
        let active_paths = paths.iter().cloned().collect::<HashSet<_>>();
        self.files.retain(|path, _| active_paths.contains(path));

        let mut events = Vec::new();
        let mut warnings = Vec::new();
        for path in paths {
            if let Err(error) = self.read_path(&path, &mut events) {
                let name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("rollout file");
                warnings.push(format!("{name}: {error}"));
            }
        }
        events.sort_by(|left, right| {
            left.timestamp_ms
                .cmp(&right.timestamp_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        let limit = query.limit.clamp(1, 100_000);
        let events = deduplicate_rollout_events(events)
            .into_iter()
            .filter(|event| query.since.is_empty() || event.timestamp > query.since)
            .take(limit)
            .collect::<Vec<_>>();
        let next_since = events
            .last()
            .map(|event| event.timestamp.clone())
            .unwrap_or_else(|| query.since.clone());
        TokenUsageResult {
            events,
            warnings,
            next_since,
            proxy_next_since_ms: query.proxy_since_ms,
            proxy_enabled_at_ms: 0,
            missing_usage: 0,
            proxy_next_offset: query.proxy_offset,
            proxy_generation: query.proxy_generation.clone(),
            proxy_reset: false,
        }
    }

    fn read_path(&mut self, path: &Path, events: &mut Vec<RolloutEvent>) -> std::io::Result<()> {
        let metadata = fs::metadata(path)?;
        let len = metadata.len();
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_millis() as u64)
            .unwrap_or_default();
        let cursor = self.files.entry(path.to_path_buf()).or_default();
        if cursor.observed_len == len && cursor.modified_ms == modified_ms {
            return Ok(());
        }

        let first_line_hash = rollout_first_line_hash(path)?;
        if cursor.offset > len
            || (!cursor.first_line_hash.is_empty() && cursor.first_line_hash != first_line_hash)
        {
            *cursor = RolloutFileCursor::default();
        }
        cursor.first_line_hash = first_line_hash;

        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(cursor.offset))?;
        let mut reader = BufReader::new(file);
        let mut next_offset = cursor.offset;
        loop {
            let mut line = String::new();
            let bytes = reader.read_line(&mut line)?;
            if bytes == 0 || !line.ends_with('\n') {
                break;
            }
            next_offset = next_offset.saturating_add(bytes as u64);
            parse_rollout_line(&line, &mut cursor.session_id, &mut cursor.model, events);
        }
        cursor.offset = next_offset;
        cursor.observed_len = len;
        cursor.modified_ms = modified_ms;
        Ok(())
    }
}

fn rollout_first_line_hash(path: &Path) -> std::io::Result<String> {
    let file = File::open(path)?;
    let mut first_line = String::new();
    BufReader::new(file).read_line(&mut first_line)?;
    let mut digest = Sha256::new();
    digest.update(first_line.as_bytes());
    Ok(format!("{:x}", digest.finalize()))
}

const PROXY_ROLLOUT_MATCH_WINDOW_MS: u64 = 5 * 60 * 1_000;

#[derive(Hash, Eq, PartialEq)]
struct RequestUsageKey {
    model: String,
    input: u64,
    cached_input: u64,
    output: u64,
}

fn request_usage_key(event: &RolloutEvent) -> RequestUsageKey {
    RequestUsageKey {
        model: event.model.trim().to_ascii_lowercase(),
        input: event.usage.input,
        cached_input: event.usage.cached_input,
        output: event.usage.output,
    }
}

fn remove_rollouts_captured_by_proxy(
    rollout_events: &mut Vec<RolloutEvent>,
    proxy_events: &[RolloutEvent],
) {
    let mut rollout_by_usage = HashMap::<RequestUsageKey, Vec<usize>>::new();
    for (index, event) in rollout_events.iter().enumerate() {
        if event.timestamp_ms == 0 || !event.usage.has_tokens() {
            continue;
        }
        rollout_by_usage
            .entry(request_usage_key(event))
            .or_default()
            .push(index);
    }

    let mut matched_rollouts = HashSet::new();
    for proxy in proxy_events {
        if proxy.timestamp_ms == 0 || proxy.usage_missing || !proxy.usage.has_tokens() {
            continue;
        }
        let key = request_usage_key(proxy);
        let exact_candidates = rollout_by_usage
            .get(&key)
            .into_iter()
            .flatten()
            .copied()
            .filter(|index| !matched_rollouts.contains(index))
            .filter_map(|index| {
                let delta = rollout_events[index]
                    .timestamp_ms
                    .abs_diff(proxy.timestamp_ms);
                (delta <= PROXY_ROLLOUT_MATCH_WINDOW_MS).then_some((index, delta))
            })
            .collect::<Vec<_>>();
        let nearest = if exact_candidates.is_empty() {
            let renamed_candidates = rollout_events
                .iter()
                .enumerate()
                .filter(|(index, event)| {
                    !matched_rollouts.contains(index)
                        && event.usage.input == proxy.usage.input
                        && event.usage.cached_input == proxy.usage.cached_input
                        && event.usage.output == proxy.usage.output
                })
                .filter_map(|(index, event)| {
                    let delta = event.timestamp_ms.abs_diff(proxy.timestamp_ms);
                    (delta <= PROXY_ROLLOUT_MATCH_WINDOW_MS).then_some((index, delta))
                })
                .collect::<Vec<_>>();
            (renamed_candidates.len() == 1).then(|| renamed_candidates[0].0)
        } else {
            exact_candidates
                .into_iter()
                .min_by_key(|(_, delta)| *delta)
                .map(|(index, _)| index)
        };
        if let Some(index) = nearest {
            matched_rollouts.insert(index);
        }
    }

    let mut index = 0;
    rollout_events.retain(|_| {
        let keep = !matched_rollouts.contains(&index);
        index += 1;
        keep
    });
}

#[derive(Hash, Eq, PartialEq)]
enum EventKey {
    Watermark {
        session_id: String,
        input: u64,
        cached_input: u64,
        cache_write: u64,
        output: u64,
        reasoning: u64,
        total: u64,
    },
    EventId(String),
}

fn event_key(event: &RolloutEvent) -> EventKey {
    if !event.session_id.is_empty() && event.totals.has_tokens() {
        return EventKey::Watermark {
            session_id: event.session_id.clone(),
            input: event.totals.input,
            cached_input: event.totals.cached_input,
            cache_write: event.totals.cache_write,
            output: event.totals.output,
            reasoning: event.totals.reasoning,
            total: event.totals.total,
        };
    }
    EventKey::EventId(event.id.clone())
}

fn model_is_unknown(model: &str) -> bool {
    model.trim().is_empty() || model.eq_ignore_ascii_case("unknown")
}

pub fn deduplicate_rollout_events(events: Vec<RolloutEvent>) -> Vec<RolloutEvent> {
    let mut indexes = HashMap::new();
    let mut deduplicated = Vec::with_capacity(events.len());

    for event in events {
        let key = event_key(&event);
        if let Some(index) = indexes.get(&key).copied() {
            let existing: &mut RolloutEvent = &mut deduplicated[index];
            if model_is_unknown(&existing.model) && !model_is_unknown(&event.model) {
                *existing = event;
            }
            continue;
        }
        indexes.insert(key, deduplicated.len());
        deduplicated.push(event);
    }

    deduplicated
}

pub fn read_rollout_events(query: &TokenUsageQuery) -> TokenUsageResult {
    let codex_home = crate::codex_home::default_codex_home_dir();
    read_rollout_events_from_roots(
        &[
            codex_home.join("sessions"),
            codex_home.join("archived_sessions"),
        ],
        query,
    )
}

pub fn read_rollout_events_from_roots(
    roots: &[PathBuf],
    query: &TokenUsageQuery,
) -> TokenUsageResult {
    let days = query.days.clamp(1, 31);
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(days * 24 * 60 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut paths = Vec::new();
    for root in roots {
        collect_rollout_paths(root, cutoff, &mut paths);
    }
    paths.sort();

    let mut events = Vec::new();
    let mut warnings = Vec::new();
    for path in paths {
        if let Err(error) = parse_rollout(&path, &mut events) {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("rollout file");
            warnings.push(format!("{name}: {error}"));
        }
    }
    events.sort_by(|left, right| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left.id.cmp(&right.id))
    });
    let deduplicated = deduplicate_rollout_events(events);
    let limit = query.limit.clamp(1, 100_000);
    let events = deduplicated
        .into_iter()
        .filter(|event| query.since.is_empty() || event.timestamp > query.since)
        .take(limit)
        .collect::<Vec<_>>();
    let next_since = events
        .last()
        .map(|event| event.timestamp.clone())
        .unwrap_or_else(|| query.since.clone());

    TokenUsageResult {
        events,
        warnings,
        next_since,
        proxy_next_since_ms: query.proxy_since_ms,
        proxy_enabled_at_ms: 0,
        missing_usage: 0,
        proxy_next_offset: 0,
        proxy_generation: String::new(),
        proxy_reset: false,
    }
}

fn collect_rollout_paths(root: &Path, cutoff: SystemTime, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rollout_paths(&path, cutoff, paths);
            continue;
        }
        let is_rollout = path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"));
        if !is_rollout {
            continue;
        }
        let recent = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map(|modified| modified >= cutoff)
            .unwrap_or(true);
        if recent {
            paths.push(path);
        }
    }
}

fn parse_rollout(path: &Path, events: &mut Vec<RolloutEvent>) -> anyhow::Result<()> {
    let file = File::open(path)?;
    let mut session_id = String::new();
    let mut model = String::new();

    for line in BufReader::new(file).lines() {
        let line = line?;
        parse_rollout_line(&line, &mut session_id, &mut model, events);
    }
    Ok(())
}

fn parse_rollout_line(
    line: &str,
    session_id: &mut String,
    model: &mut String,
    events: &mut Vec<RolloutEvent>,
) {
    if !line.contains("token_count")
        && !line.contains("session_meta")
        && !line.contains("turn_context")
    {
        return;
    }
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let payload = record.get("payload").unwrap_or(&Value::Null);
    let record_type = payload
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| record.get("type").and_then(Value::as_str))
        .unwrap_or_default();
    match record_type {
        "session_meta" => *session_id = string_field(payload, &["session_id", "id"]),
        "turn_context" => {
            let next_model = string_field(payload, &["model"]);
            if !next_model.is_empty() {
                *model = next_model;
            }
        }
        "token_count" => {
            let info = payload.get("info").unwrap_or(&Value::Null);
            let usage = usage_counts(
                info.get("last_token_usage")
                    .or_else(|| info.get("lastTokenUsage"))
                    .unwrap_or(&Value::Null),
            );
            if !usage.has_tokens() {
                return;
            }
            let totals = usage_counts(
                info.get("total_token_usage")
                    .or_else(|| info.get("totalTokenUsage"))
                    .unwrap_or(&Value::Null),
            );
            let timestamp = record
                .get("timestamp")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let event_model = if model.is_empty() {
                "Unknown".to_string()
            } else {
                model.clone()
            };
            let id = stable_event_id(session_id, &timestamp, &usage, &totals);
            let timestamp_ms = timestamp_to_epoch_ms(&timestamp).unwrap_or_default();
            events.push(RolloutEvent {
                id,
                session_id: session_id.clone(),
                timestamp,
                model: event_model,
                usage,
                totals,
                source: "rollout".to_string(),
                status: "completed".to_string(),
                usage_missing: false,
                timestamp_ms,
                response_id: String::new(),
            });
        }
        _ => {}
    }
}

fn usage_counts(value: &Value) -> UsageCounts {
    let input = number_field(value, &["input_tokens", "inputTokens"]);
    let output = number_field(value, &["output_tokens", "outputTokens"]);
    UsageCounts {
        input,
        cached_input: number_field(value, &["cached_input_tokens", "cachedInputTokens"]),
        cache_write: number_field(
            value,
            &[
                "cache_write_input_tokens",
                "cacheWriteInputTokens",
                "cache_creation_input_tokens",
                "cacheCreationInputTokens",
            ],
        ),
        output,
        reasoning: number_field(value, &["reasoning_output_tokens", "reasoningOutputTokens"]),
        total: number_field(value, &["total_tokens", "totalTokens"])
            .max(input.saturating_add(output)),
    }
}

fn number_field(value: &Value, names: &[&str]) -> u64 {
    names
        .iter()
        .find_map(|name| value.get(name).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn string_field(value: &Value, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| value.get(name).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn stable_event_id(
    session_id: &str,
    timestamp: &str,
    usage: &UsageCounts,
    totals: &UsageCounts,
) -> String {
    let mut digest = Sha256::new();
    if !session_id.is_empty() && totals.has_tokens() {
        digest.update(b"watermark-v2\0");
        digest.update(session_id.as_bytes());
        for value in [
            totals.input,
            totals.cached_input,
            totals.cache_write,
            totals.output,
            totals.reasoning,
            totals.total,
        ] {
            digest.update(value.to_le_bytes());
        }
    } else {
        digest.update(b"event-v1\0");
        digest.update(session_id.as_bytes());
        digest.update(timestamp.as_bytes());
        for value in [
            usage.input,
            usage.cached_input,
            usage.cache_write,
            usage.output,
            usage.reasoning,
            usage.total,
        ] {
            digest.update(value.to_le_bytes());
        }
    }
    format!("{:x}", digest.finalize())
}

static PROXY_LEDGER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static ROLLOUT_TAILER: OnceLock<Mutex<RolloutTailer>> = OnceLock::new();
static LEDGER_GENERATION_COUNTER: AtomicU64 = AtomicU64::new(0);
const LEDGER_HEADER_TYPE: &str = "codex_plus_token_usage_ledger";
const MAX_PROXY_LEDGER_BYTES: u64 = 20 * 1024 * 1024;
const MAX_PROXY_LEDGER_ARCHIVES: usize = 3;

pub fn captured_response_usage_from_value(
    value: &Value,
    fallback_model: &str,
    fallback_status: &str,
) -> CapturedResponseUsage {
    let response = value.get("response").unwrap_or(value);
    let usage_value = response.get("usage").or_else(|| value.get("usage"));
    let usage = usage_value.map(usage_counts_from_api).unwrap_or_default();
    let response_id = string_field(response, &["id", "response_id", "responseId"]);
    let response_id = if response_id.is_empty() {
        string_field(value, &["response_id", "responseId", "id"])
    } else {
        response_id
    };
    let model = {
        let response_model = string_field(response, &["model"]);
        if response_model.is_empty() {
            let event_model = string_field(value, &["model"]);
            if event_model.is_empty() {
                fallback_model.to_string()
            } else {
                event_model
            }
        } else {
            response_model
        }
    };
    let status = {
        let response_status = string_field(response, &["status"]);
        if response_status.is_empty() {
            let event_status = string_field(value, &["status"]);
            if event_status.is_empty() {
                fallback_status.to_string()
            } else {
                event_status
            }
        } else {
            response_status
        }
    };
    let usage_missing = usage_value.is_none_or(|value| !has_usage_fields(value));

    CapturedResponseUsage {
        response_id,
        model,
        status,
        usage,
        usage_missing,
    }
}

fn usage_counts_from_api(value: &Value) -> UsageCounts {
    let input = number_field(
        value,
        &[
            "input_tokens",
            "inputTokens",
            "prompt_tokens",
            "promptTokens",
        ],
    );
    let output = number_field(
        value,
        &[
            "output_tokens",
            "outputTokens",
            "completion_tokens",
            "completionTokens",
        ],
    );
    let input_details = value
        .get("input_tokens_details")
        .or_else(|| value.get("inputTokensDetails"))
        .or_else(|| value.get("prompt_tokens_details"))
        .or_else(|| value.get("promptTokensDetails"))
        .unwrap_or(&Value::Null);
    let output_details = value
        .get("output_tokens_details")
        .or_else(|| value.get("outputTokensDetails"))
        .or_else(|| value.get("completion_tokens_details"))
        .or_else(|| value.get("completionTokensDetails"))
        .unwrap_or(&Value::Null);
    UsageCounts {
        input,
        cached_input: number_field(
            value,
            &[
                "cached_input_tokens",
                "cachedInputTokens",
                "cache_read_input_tokens",
                "cacheReadInputTokens",
            ],
        )
        .max(number_field(
            input_details,
            &[
                "cached_tokens",
                "cachedTokens",
                "cache_read_tokens",
                "cacheReadTokens",
            ],
        )),
        cache_write: number_field(
            value,
            &[
                "cache_write_input_tokens",
                "cacheWriteInputTokens",
                "cache_creation_input_tokens",
                "cacheCreationInputTokens",
            ],
        )
        .max(number_field(
            input_details,
            &[
                "cache_write_tokens",
                "cacheWriteTokens",
                "cache_creation_tokens",
                "cacheCreationTokens",
            ],
        )),
        output,
        reasoning: number_field(value, &["reasoning_output_tokens", "reasoningOutputTokens"]).max(
            number_field(output_details, &["reasoning_tokens", "reasoningTokens"]),
        ),
        total: number_field(value, &["total_tokens", "totalTokens"])
            .max(input.saturating_add(output)),
    }
}

fn has_usage_fields(value: &Value) -> bool {
    [
        "input_tokens",
        "inputTokens",
        "prompt_tokens",
        "promptTokens",
        "output_tokens",
        "outputTokens",
        "completion_tokens",
        "completionTokens",
        "total_tokens",
        "totalTokens",
    ]
    .iter()
    .any(|name| value.get(name).is_some())
}

pub fn append_proxy_usage_record(captured: &CapturedResponseUsage) -> std::io::Result<()> {
    append_proxy_usage_record_at(
        &crate::paths::default_token_usage_proxy_ledger_path(),
        captured,
        now_ms(),
    )
}

pub fn append_proxy_retry_attempt(request: &Value, attempt: usize) -> std::io::Result<()> {
    append_proxy_retry_attempt_at(
        &crate::paths::default_token_usage_proxy_ledger_path(),
        request,
        attempt,
        now_ms(),
    )
}

pub fn append_proxy_retry_attempt_at(
    path: &Path,
    request: &Value,
    attempt: usize,
    timestamp_ms: u64,
) -> std::io::Result<()> {
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("Unknown");
    let mut digest = Sha256::new();
    digest.update(b"proxy-retry-v1\0");
    digest.update(timestamp_ms.to_le_bytes());
    digest.update((attempt as u64).to_le_bytes());
    digest.update(model.as_bytes());
    digest.update(
        LEDGER_GENERATION_COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .to_le_bytes(),
    );
    append_proxy_usage_record_at(
        path,
        &CapturedResponseUsage {
            response_id: format!("retry:{:x}", digest.finalize()),
            model: model.to_string(),
            status: "retry".to_string(),
            usage_missing: true,
            ..CapturedResponseUsage::default()
        },
        timestamp_ms,
    )
}

pub fn append_proxy_usage_record_at(
    path: &Path,
    captured: &CapturedResponseUsage,
    timestamp_ms: u64,
) -> std::io::Result<()> {
    let _guard = PROXY_LEDGER_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| std::io::Error::other("token usage ledger lock poisoned"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    rotate_proxy_ledger_if_needed(path)?;
    let response_id = captured.response_id.trim().to_string();
    let id = stable_proxy_event_id(&response_id, captured, timestamp_ms);
    let event = RolloutEvent {
        id,
        session_id: String::new(),
        timestamp: timestamp_ms.to_string(),
        model: if captured.model.trim().is_empty() {
            "Unknown".to_string()
        } else {
            captured.model.clone()
        },
        usage: captured.usage.clone(),
        totals: UsageCounts::default(),
        source: "proxy".to_string(),
        status: captured.status.clone(),
        usage_missing: captured.usage_missing,
        timestamp_ms,
        response_id,
    };
    let needs_header = fs::metadata(path)
        .map(|metadata| metadata.len() == 0)
        .unwrap_or(true);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if needs_header {
        let header = serde_json::json!({
            "type": LEDGER_HEADER_TYPE,
            "generation": new_ledger_generation(timestamp_ms),
        });
        let mut line = serde_json::to_vec(&header)?;
        line.push(b'\n');
        file.write_all(&line)?;
    }
    let mut line = serde_json::to_vec(&event)?;
    line.push(b'\n');
    file.write_all(&line)?;
    Ok(())
}

fn rotate_proxy_ledger_if_needed(path: &Path) -> std::io::Result<()> {
    let should_rotate = fs::metadata(path)
        .map(|metadata| metadata.len() >= MAX_PROXY_LEDGER_BYTES)
        .unwrap_or(false);
    if !should_rotate {
        return Ok(());
    }
    for index in (1..=MAX_PROXY_LEDGER_ARCHIVES).rev() {
        let destination = proxy_ledger_archive_path(path, index);
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        let source = if index == 1 {
            path.to_path_buf()
        } else {
            proxy_ledger_archive_path(path, index - 1)
        };
        if source.exists() {
            fs::rename(source, destination)?;
        }
    }
    Ok(())
}

fn proxy_ledger_archive_path(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

pub fn read_proxy_usage_records_from_path(
    path: &Path,
    since_ms: u64,
    limit: usize,
) -> std::io::Result<Vec<RolloutEvent>> {
    let (events, _, _, _) = read_all_proxy_usage_records(path)?;
    let mut events = events
        .into_iter()
        .filter(|event| event.timestamp_ms > since_ms)
        .collect::<Vec<_>>();
    events.truncate(limit.max(1));
    Ok(events)
}

fn read_all_proxy_usage_records(
    path: &Path,
) -> std::io::Result<(Vec<RolloutEvent>, u64, bool, String)> {
    let mut events = Vec::new();
    for index in (1..=MAX_PROXY_LEDGER_ARCHIVES).rev() {
        let archive = proxy_ledger_archive_path(path, index);
        let (mut archived, _, _, _) =
            read_proxy_usage_records_from_offset(&archive, 0, "", usize::MAX)?;
        events.append(&mut archived);
    }
    let (mut current, next_offset, reset, generation) =
        read_proxy_usage_records_from_offset(path, 0, "", usize::MAX)?;
    events.append(&mut current);
    let mut seen = HashSet::new();
    events.retain(|event| {
        let key = if event.response_id.is_empty() {
            event.id.clone()
        } else {
            format!("response:{}", event.response_id)
        };
        seen.insert(key)
    });
    events.sort_by_key(|event| event.timestamp_ms);
    Ok((events, next_offset, reset, generation))
}

fn read_proxy_usage_records_from_offset(
    path: &Path,
    offset: u64,
    expected_generation: &str,
    limit: usize,
) -> std::io::Result<(Vec<RolloutEvent>, u64, bool, String)> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), 0, offset > 0, String::new()));
        }
        Err(error) => return Err(error),
    };
    let len = file.metadata()?.len();
    let (generation, data_start) = read_ledger_header(&mut file)?;
    let generation_changed = !expected_generation.is_empty() && expected_generation != generation;
    let reset = generation_changed || offset > len;
    let start = if reset || offset == 0 {
        data_start
    } else {
        offset
    };
    file.seek(SeekFrom::Start(start))?;

    let mut seen = HashSet::new();
    let mut events = Vec::new();
    let mut reader = BufReader::new(file);
    let mut next_offset = start;
    while events.len() < limit.max(1) {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        if !line.ends_with('\n') {
            break;
        }
        let complete_offset = next_offset.saturating_add(bytes as u64);
        let Ok(event) = serde_json::from_str::<RolloutEvent>(&line) else {
            next_offset = complete_offset;
            continue;
        };
        next_offset = complete_offset;
        let key = if event.response_id.is_empty() {
            event.id.clone()
        } else {
            format!("response:{}", event.response_id)
        };
        if !seen.insert(key) {
            continue;
        }
        events.push(event);
    }
    events.sort_by_key(|event| event.timestamp_ms);
    Ok((events, next_offset, reset, generation))
}

fn read_ledger_header(file: &mut File) -> std::io::Result<(String, u64)> {
    file.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::new(&mut *file);
    let mut first_line = String::new();
    let bytes = reader.read_line(&mut first_line)?;
    let header = serde_json::from_str::<Value>(&first_line).ok();
    let is_header = header
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        == Some(LEDGER_HEADER_TYPE);
    let generation = if is_header {
        header
            .as_ref()
            .and_then(|value| value.get("generation"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    } else {
        "legacy-v1".to_string()
    };
    Ok((generation, if is_header { bytes as u64 } else { 0 }))
}

fn new_ledger_generation(timestamp_ms: u64) -> String {
    let sequence = LEDGER_GENERATION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"ledger-v1\0");
    digest.update(timestamp_ms.to_le_bytes());
    digest.update(std::process::id().to_le_bytes());
    digest.update(sequence.to_le_bytes());
    format!("{:x}", digest.finalize())
}

pub fn read_token_usage_events_from_sources(
    roots: &[PathBuf],
    ledger_path: &Path,
    query: &TokenUsageQuery,
) -> TokenUsageResult {
    let cutoff_ms = now_ms().saturating_sub(query.days.clamp(1, 31) * 24 * 60 * 60 * 1_000);
    if !query.include_rollout {
        return match read_proxy_usage_records_from_offset(
            ledger_path,
            query.proxy_offset,
            &query.proxy_generation,
            query.limit.clamp(1, 1_000_000),
        ) {
            Ok((events, proxy_next_offset, proxy_reset, proxy_generation)) => {
                let events = events
                    .into_iter()
                    .filter(|event| event.timestamp_ms >= cutoff_ms)
                    .collect::<Vec<_>>();
                let missing_usage = events.iter().filter(|event| event.usage_missing).count();
                let proxy_next_since_ms = events
                    .last()
                    .map(|event| event.timestamp_ms)
                    .unwrap_or(query.proxy_since_ms);
                TokenUsageResult {
                    events,
                    warnings: Vec::new(),
                    next_since: query.since.clone(),
                    proxy_next_since_ms,
                    proxy_enabled_at_ms: 0,
                    missing_usage,
                    proxy_next_offset,
                    proxy_generation,
                    proxy_reset,
                }
            }
            Err(error) => TokenUsageResult {
                events: Vec::new(),
                warnings: vec![format!("proxy token usage ledger: {error}")],
                next_since: query.since.clone(),
                proxy_next_since_ms: query.proxy_since_ms,
                proxy_enabled_at_ms: 0,
                missing_usage: 0,
                proxy_next_offset: query.proxy_offset,
                proxy_generation: query.proxy_generation.clone(),
                proxy_reset: false,
            },
        };
    }

    let rollout = read_rollout_events_from_roots(roots, query);
    merge_full_rollout_with_proxy(rollout, ledger_path, query, cutoff_ms)
}

fn merge_full_rollout_with_proxy(
    mut rollout: TokenUsageResult,
    ledger_path: &Path,
    query: &TokenUsageQuery,
    cutoff_ms: u64,
) -> TokenUsageResult {
    let proxy_match_since_ms = cutoff_ms.saturating_sub(PROXY_ROLLOUT_MATCH_WINDOW_MS);
    let (all_proxy, proxy_next_offset, proxy_reset, proxy_generation) =
        match read_all_proxy_usage_records(ledger_path) {
            Ok((events, next_offset, reset, generation)) => (
                events
                    .into_iter()
                    .filter(|event| event.timestamp_ms >= proxy_match_since_ms)
                    .collect(),
                next_offset,
                reset,
                generation,
            ),
            Err(error) => {
                rollout
                    .warnings
                    .push(format!("proxy token usage ledger: {error}"));
                (Vec::new(), 0, false, String::new())
            }
        };
    let proxy_enabled_at_ms = all_proxy
        .iter()
        .map(|event| event.timestamp_ms)
        .min()
        .unwrap_or_default();
    remove_rollouts_captured_by_proxy(&mut rollout.events, &all_proxy);

    let mut proxy = all_proxy
        .into_iter()
        .filter(|event| event.timestamp_ms >= cutoff_ms)
        .filter(|event| event.timestamp_ms > query.proxy_since_ms)
        .collect::<Vec<_>>();
    let missing_usage = proxy.iter().filter(|event| event.usage_missing).count();
    let proxy_next_since_ms = proxy
        .last()
        .map(|event| event.timestamp_ms)
        .unwrap_or(query.proxy_since_ms);
    rollout.events.append(&mut proxy);
    rollout.events.sort_by(|left, right| {
        left.timestamp_ms
            .cmp(&right.timestamp_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    rollout.events.truncate(query.limit.clamp(1, 1_000_000));
    rollout.proxy_next_since_ms = proxy_next_since_ms;
    rollout.proxy_enabled_at_ms = proxy_enabled_at_ms;
    rollout.missing_usage = missing_usage;
    rollout.proxy_next_offset = proxy_next_offset;
    rollout.proxy_generation = proxy_generation;
    rollout.proxy_reset = proxy_reset;
    rollout
}

pub fn read_token_usage_events(query: &TokenUsageQuery) -> TokenUsageResult {
    let codex_home = crate::codex_home::default_codex_home_dir();
    let roots = [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ];
    let ledger_path = crate::paths::default_token_usage_proxy_ledger_path();
    if !query.include_rollout {
        return read_token_usage_events_from_sources(&roots, &ledger_path, query);
    }

    let mut tailer = match ROLLOUT_TAILER
        .get_or_init(|| Mutex::new(RolloutTailer::default()))
        .lock()
    {
        Ok(tailer) => tailer,
        Err(_) => {
            let mut result = read_token_usage_events_from_sources(&roots, &ledger_path, query);
            result
                .warnings
                .push("rollout tailer lock poisoned; used full scan".to_string());
            return result;
        }
    };
    if !query.rollout_incremental {
        tailer.reset();
    }
    let rollout = tailer.read_from_roots(&roots, query);
    let cutoff_ms = now_ms().saturating_sub(query.days.clamp(1, 31) * 24 * 60 * 60 * 1_000);
    if !query.rollout_incremental {
        merge_full_rollout_with_proxy(rollout, &ledger_path, query, cutoff_ms)
    } else {
        merge_incremental_rollout_with_proxy(rollout, &ledger_path, query, cutoff_ms)
    }
}

fn merge_incremental_rollout_with_proxy(
    mut rollout: TokenUsageResult,
    ledger_path: &Path,
    query: &TokenUsageQuery,
    cutoff_ms: u64,
) -> TokenUsageResult {
    let (mut proxy, proxy_next_offset, proxy_reset, proxy_generation) =
        match read_proxy_usage_records_from_offset(
            ledger_path,
            query.proxy_offset,
            &query.proxy_generation,
            query.limit.clamp(1, 1_000_000),
        ) {
            Ok(result) => result,
            Err(error) => {
                rollout
                    .warnings
                    .push(format!("proxy token usage ledger: {error}"));
                return rollout;
            }
        };
    proxy.retain(|event| event.timestamp_ms >= cutoff_ms);
    remove_rollouts_captured_by_proxy(&mut rollout.events, &proxy);
    let missing_usage = proxy.iter().filter(|event| event.usage_missing).count();
    let proxy_next_since_ms = proxy
        .last()
        .map(|event| event.timestamp_ms)
        .unwrap_or(query.proxy_since_ms);
    rollout.events.append(&mut proxy);
    rollout.events.sort_by(|left, right| {
        left.timestamp_ms
            .cmp(&right.timestamp_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    rollout.events.truncate(query.limit.clamp(1, 1_000_000));
    rollout.proxy_next_since_ms = proxy_next_since_ms;
    rollout.missing_usage = missing_usage;
    rollout.proxy_next_offset = proxy_next_offset;
    rollout.proxy_generation = proxy_generation;
    rollout.proxy_reset = proxy_reset;
    rollout
}

fn stable_proxy_event_id(
    response_id: &str,
    captured: &CapturedResponseUsage,
    timestamp_ms: u64,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"proxy-v1\0");
    if response_id.is_empty() {
        digest.update(timestamp_ms.to_le_bytes());
        digest.update(captured.model.as_bytes());
        digest.update(captured.status.as_bytes());
        digest.update(
            LEDGER_GENERATION_COUNTER
                .fetch_add(1, Ordering::Relaxed)
                .to_le_bytes(),
        );
    } else {
        digest.update(response_id.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn timestamp_to_epoch_ms(timestamp: &str) -> Option<u64> {
    let bytes = timestamp.as_bytes();
    if bytes.len() < 20 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let year = timestamp.get(0..4)?.parse::<i64>().ok()?;
    let month = timestamp.get(5..7)?.parse::<i64>().ok()?;
    let day = timestamp.get(8..10)?.parse::<i64>().ok()?;
    let hour = timestamp.get(11..13)?.parse::<i64>().ok()?;
    let minute = timestamp.get(14..16)?.parse::<i64>().ok()?;
    let second = timestamp.get(17..19)?.parse::<i64>().ok()?;
    let millis = timestamp
        .get(19..)
        .and_then(|rest| rest.strip_prefix('.'))
        .map(|fraction| {
            fraction
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .take(3)
                .collect::<String>()
        })
        .filter(|fraction| !fraction.is_empty())
        .and_then(|mut fraction| {
            while fraction.len() < 3 {
                fraction.push('0');
            }
            fraction.parse::<i64>().ok()
        })
        .unwrap_or(0);
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?;
    u64::try_from(seconds.checked_mul(1_000)?.checked_add(millis)?).ok()
}

pub fn events_value(payload: Value) -> anyhow::Result<Value> {
    let query = serde_json::from_value::<TokenUsageQuery>(payload).unwrap_or_default();
    Ok(serde_json::to_value(read_token_usage_events(&query))?)
}
