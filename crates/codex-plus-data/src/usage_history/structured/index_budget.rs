//! 全部 FileState 共用的保守配额；宁可返回 partial，也不丢弃未接收行或累计基线。
use super::{FileState, MAX_ACTIVITY, MAX_EVENTS};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::ops::Deref;
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
const MAX_ROUND_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;
static PENDING_BYTES: AtomicUsize = AtomicUsize::new(0);
// 解压窗口限制为 2 MiB，另给输入/输出缓冲及解码器管理结构留出余量。
pub(super) const CURSOR_BYTES: usize = 3 * 1024 * 1024;
const LINE_OVERHEAD: usize = 4096;

#[derive(Default)]
pub(super) struct PendingLine {
    bytes: Vec<u8>,
    reserved: usize,
}

impl PendingLine {
    pub(super) fn capacity(&self) -> usize {
        self.bytes.capacity()
    }

    pub(super) fn try_extend(&mut self, part: &[u8]) -> bool {
        let Some(length) = self.bytes.len().checked_add(part.len()) else {
            return false;
        };
        if length <= self.bytes.capacity() {
            self.bytes.extend_from_slice(part);
            return true;
        }
        if !Self::reserve(length) {
            return false;
        }
        let mut pending = Self {
            bytes: Vec::new(),
            reserved: length,
        };
        if pending.bytes.try_reserve_exact(length).is_err() {
            return false;
        }
        let capacity = pending.bytes.capacity();
        if capacity > pending.reserved {
            if !Self::reserve(capacity - pending.reserved) {
                return false;
            }
            pending.reserved = capacity;
        }
        pending.bytes.extend_from_slice(&self.bytes);
        pending.bytes.extend_from_slice(part);
        *self = pending;
        true
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn shrink_to_fit(&mut self) {
        self.bytes.shrink_to_fit();
        let capacity = self.bytes.capacity();
        if capacity < self.reserved {
            PENDING_BYTES.fetch_sub(self.reserved - capacity, Ordering::AcqRel);
            self.reserved = capacity;
        }
    }

    fn reserve(bytes: usize) -> bool {
        PENDING_BYTES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= MAX_PENDING_BYTES)
            })
            .is_ok()
    }
}

impl Deref for PendingLine {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

impl Drop for PendingLine {
    fn drop(&mut self) {
        drop(std::mem::take(&mut self.bytes));
        if self.reserved != 0 {
            PENDING_BYTES.fetch_sub(self.reserved, Ordering::AcqRel);
        }
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct Usage {
    events: usize,
    activity: usize,
    bytes: usize,
    readers: usize,
}

impl Usage {
    pub(super) fn of(state: &FileState) -> Self {
        Self {
            events: state.events.len(),
            activity: state.tools.len()
                + state.calls.len()
                + state.exec_skills.len()
                + state.exec_polls.len(),
            bytes: state.retained_charge,
            readers: usize::from(state.reader.is_some()),
        }
    }
}

pub(super) struct Budget {
    // 不含当前正在读取的 FileState；其用量在准入时实时检查。
    used: Usage,
    pub(super) read_left: u64,
    pub(super) source_left: u64,
    memory_limit: usize,
    event_limit: usize,
    activity_limit: usize,
}

impl Budget {
    pub(super) fn new<'a>(states: impl Iterator<Item = &'a FileState>) -> Self {
        let mut budget = Self {
            used: Usage::default(),
            read_left: MAX_ROUND_BYTES,
            source_left: MAX_ROUND_BYTES,
            memory_limit: MAX_RETAINED_BYTES,
            event_limit: MAX_EVENTS,
            activity_limit: MAX_ACTIVITY,
        };
        for state in states {
            budget.replace(Usage::default(), Usage::of(state));
        }
        budget
    }

    pub(super) fn replace(&mut self, before: Usage, after: Usage) {
        self.used.events = self.used.events.saturating_sub(before.events) + after.events;
        self.used.activity = self.used.activity.saturating_sub(before.activity) + after.activity;
        self.used.bytes = self.used.bytes.saturating_sub(before.bytes) + after.bytes;
        self.used.readers = self.used.readers.saturating_sub(before.readers) + after.readers;
    }

    fn memory_left(&self, state: &FileState) -> usize {
        self.memory_limit
            .saturating_sub(self.used.bytes.saturating_add(state.retained_charge))
    }

    pub(super) fn read_limit(&self, state: &FileState) -> usize {
        // Vec 扩容最多预留约两倍新增输入，不把正文计为永久保留量。
        (self.read_left as usize).min(self.memory_left(state) / 2)
    }

    pub(super) fn can_read(&self, state: &FileState) -> bool {
        self.read_limit(state) > 0
    }

    pub(super) fn can_accept_activity(&self, state: &FileState) -> bool {
        self.used.activity + Usage::of(state).activity + 4 <= self.activity_limit
    }

    pub(super) fn can_skip_activity_line(&self, state: &FileState) -> bool {
        self.memory_left(state) >= LINE_OVERHEAD && skips_tool_before_parsing(&state.pending)
    }

    pub(super) fn can_open(&self, state: &FileState) -> bool {
        self.used.readers == 0
            && self.can_read(state)
            && self.source_left > 0
            && self.memory_left(state) > CURSOR_BYTES + LINE_OVERHEAD
    }

    pub(super) fn charge_read(&mut self, bytes: usize) {
        self.read_left -= bytes as u64;
    }

    pub(super) fn accept_line(&self, state: &FileState) -> bool {
        if self.event_limit_reached(state) && may_record_event(&state.pending) {
            return false;
        }
        let reserve = self.line_reserve(state);
        if reserve == LINE_OVERHEAD {
            return self.memory_left(state) >= reserve;
        }
        let vector_growth = |len: usize, capacity: usize, element: usize| {
            if len == capacity {
                capacity.max(4).saturating_mul(element)
            } else {
                0
            }
        };
        let growth = vector_growth(
            state.events.len(),
            state.events.capacity(),
            std::mem::size_of::<super::UsageEvent>(),
        ) + vector_growth(
            state.tools.len(),
            state.tools.capacity(),
            std::mem::size_of::<super::ToolRecord>(),
        ) + state.calls.capacity()
            * (std::mem::size_of::<(String, super::Call)>() + 32)
            + state.exec_skills.capacity()
                * (std::mem::size_of::<(String, super::ExecSkill)>() + 32)
            + state.exec_polls.capacity() * (std::mem::size_of::<(String, String)>() + 32)
            + [
                &state.session,
                &state.turn,
                &state.provider,
                &state.model,
                &state.role,
            ]
            .iter()
            .map(|s| s.capacity())
            .sum::<usize>()
            + state
                .inherited_contexts
                .iter()
                .map(|context| {
                    [
                        &context.session,
                        &context.turn,
                        &context.provider,
                        &context.model,
                        &context.role,
                    ]
                    .iter()
                    .map(|s| s.capacity())
                    .sum::<usize>()
                })
                .sum::<usize>();
        if self.memory_left(state) < reserve.saturating_add(growth) {
            return false;
        }
        true
    }

    pub(super) fn line_exceeds_budget(&self, state: &FileState) -> bool {
        if skips_tool_before_parsing(&state.pending) {
            return false;
        }
        if self.event_limit_reached(state) && may_record_event(&state.pending) {
            return false;
        }
        self.line_reserve(state)
            .saturating_add(CURSOR_BYTES)
            .saturating_add(state.pending.capacity())
            > self.memory_limit
    }

    fn event_limit_reached(&self, state: &FileState) -> bool {
        self.used.events.saturating_add(state.events.len()) >= self.event_limit
    }

    fn line_reserve(&self, state: &FileState) -> usize {
        parse_reserve(&state.pending, !self.can_accept_activity(state))
    }
}

// 与 accept 的借用式入口保持一致；不能仅凭 type 放宽格式损坏的行。
#[derive(serde::Deserialize)]
struct AcceptEnvelope<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(rename = "timestamp")]
    _timestamp: Option<&'a str>,
    #[serde(rename = "ordinal")]
    _ordinal: Option<i64>,
    #[serde(borrow)]
    payload: &'a serde_json::value::RawValue,
}

fn skips_tool_before_parsing(line: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct ToolItem<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
        #[serde(rename = "call_id")]
        _call_id: Option<&'a str>,
    }
    let Ok(envelope) = serde_json::from_slice::<AcceptEnvelope<'_>>(line) else {
        return false;
    };
    if envelope.kind != "response_item" {
        return false;
    }
    serde_json::from_str::<ToolItem<'_>>(envelope.payload.get()).is_ok_and(|item| {
        matches!(
            item.kind,
            "function_call"
                | "custom_tool_call"
                | "function_call_output"
                | "custom_tool_call_output"
        )
    })
}

// 不复制 token 字段的解析规则；有歧义时保守等待事件配额。
fn may_record_event(line: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct EventKind<'a> {
        #[serde(rename = "type")]
        kind: Option<&'a str>,
    }
    #[derive(serde::Deserialize)]
    struct TokenInfo<'a> {
        #[serde(borrow)]
        info: &'a serde_json::value::RawValue,
    }
    let Ok(envelope) = serde_json::from_slice::<AcceptEnvelope<'_>>(line) else {
        return false;
    };
    if !matches!(envelope.kind, "event_msg" | "turn_context") {
        return false;
    }
    let Ok(event) = serde_json::from_str::<EventKind<'_>>(envelope.payload.get()) else {
        // event_msg 本身会以相同的借用式 type 解析提前拒绝；turn_context 则会解析为 Value。
        return envelope.kind != "event_msg";
    };
    if event.kind != Some("token_count") {
        return false;
    }
    match serde_json::from_str::<TokenInfo<'_>>(envelope.payload.get()) {
        Ok(token) => token.info.get().trim() != "null",
        Err(_) => true,
    }
}

// 正文由 accept 的借用 RawValue 路径丢弃，不为它申请 Value 解析空间。
fn parse_reserve(line: &[u8], skip_activity: bool) -> usize {
    #[derive(serde::Deserialize)]
    struct Item<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
    }
    let mut nested_reserve = 0;
    if let Ok(envelope) = serde_json::from_slice::<AcceptEnvelope<'_>>(line) {
        // 与实际入口共用校验；立即拒绝或忽略的行不应占住唯一 reader 等待完整解析空间。
        if !matches!(
            envelope.kind,
            "session_meta" | "turn_context" | "event_msg" | "response_item"
        ) {
            return LINE_OVERHEAD;
        }
        if matches!(envelope.kind, "session_meta" | "turn_context") {
            if let Ok(fields) = super::context_fields(envelope.payload.get()) {
                let selected = fields
                    .iter()
                    .filter_map(|(_, value)| *value)
                    .map(|value| value.get().len().saturating_add(128))
                    .sum::<usize>();
                return selected
                    .saturating_mul(64)
                    .saturating_add(line.len().saturating_mul(2))
                    .saturating_add(LINE_OVERHEAD);
            } else {
                return LINE_OVERHEAD;
            }
        }
        if envelope.kind == "event_msg" {
            if let Ok(item) = serde_json::from_str::<Item<'_>>(envelope.payload.get()) {
                if item.kind != "token_count"
                    && !item.kind.starts_with("thread_settings")
                    && !matches!(item.kind, "task_started" | "turn_started")
                {
                    return LINE_OVERHEAD;
                }
            } else {
                return LINE_OVERHEAD;
            }
        }
        if envelope.kind == "response_item" {
            if let Ok(item) = serde_json::from_str::<Item<'_>>(envelope.payload.get()) {
                if !matches!(
                    item.kind,
                    "function_call"
                        | "custom_tool_call"
                        | "function_call_output"
                        | "custom_tool_call_output"
                ) {
                    return LINE_OVERHEAD;
                }
                if skip_activity && skips_tool_before_parsing(line) {
                    return LINE_OVERHEAD;
                }
                if skips_tool_before_parsing(line) {
                    nested_reserve = line
                        .len()
                        .min(super::tool_receipts::MAX_JSON_TEXT_BYTES)
                        .saturating_mul(64);
                }
            } else {
                return LINE_OVERHEAD;
            }
        }
    } else {
        return LINE_OVERHEAD;
    }
    // 字符串正文只需要解码/复制空间；对象、数组仍按容器最坏开销预留。
    // 全行乘 64 会让不到 1 MiB 的普通工具输出永久等不到解析配额。
    let mut quoted = false;
    let mut escaped = false;
    let mut reserve = LINE_OVERHEAD
        .saturating_add(nested_reserve)
        .saturating_add(line.len().saturating_mul(2));
    for &byte in line {
        reserve = reserve.saturating_add(if quoted { 4 } else { 64 });
        if escaped {
            escaped = false;
        } else if quoted && byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        }
    }
    reserve
}

fn value_heap(value: &serde_json::Value) -> usize {
    use serde_json::Value;
    match value {
        Value::String(text) => text.capacity(),
        Value::Array(values) => {
            values.capacity() * std::mem::size_of::<Value>()
                + values.iter().map(value_heap).sum::<usize>()
        }
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| {
                // Map 的节点/桶及装载率留有余量，避免依赖 serde_json 的 Map 后端。
                128 + key.capacity() + std::mem::size_of::<Value>() + value_heap(value)
            })
            .sum(),
        _ => 0,
    }
}

fn call_heap(call: &super::Call) -> usize {
    call.timestamp.capacity() + call.session.capacity() + value_heap(&call.payload)
}

fn event_heap(event: &super::UsageEvent) -> usize {
    [
        &event.session,
        &event.turn,
        &event.model,
        &event.timestamp,
        &event.provider,
        &event.role,
    ]
    .iter()
    .map(|text| text.capacity())
    .sum::<usize>()
        + event.legacy_identity.as_ref().map_or(0, |id| {
            std::mem::size_of::<super::LegacyIdentity>()
                + id.session.capacity()
                + id.role.capacity()
        })
}

fn tool_heap(tool: &super::ToolRecord) -> usize {
    tool.id.capacity() + call_heap(&tool.call) + value_heap(&tool.output)
}

// 向量的已有记录只在每个文件/每轮入口遍历一次；每行仅计量新增尾部。
// 其余有界映射与元数据直接重估，正文清空后立即释放其预算。
pub(super) struct Meter {
    events: usize,
    tools: usize,
    event_heap: usize,
    tool_heap: usize,
}

impl Meter {
    pub(super) fn new(state: &FileState) -> Self {
        Self {
            events: state.events.len(),
            tools: state.tools.len(),
            event_heap: state.events.iter().map(event_heap).sum(),
            tool_heap: state.tools.iter().map(tool_heap).sum(),
        }
    }

    pub(super) fn refresh(&mut self, state: &mut FileState, cursor_bytes: usize) {
        if state.events.len() < self.events || state.tools.len() < self.tools {
            *self = Self::new(state);
        } else {
            self.event_heap += state.events[self.events..]
                .iter()
                .map(event_heap)
                .sum::<usize>();
            self.tool_heap += state.tools[self.tools..]
                .iter()
                .map(tool_heap)
                .sum::<usize>();
            self.events = state.events.len();
            self.tools = state.tools.len();
        }
        let strings = [
            &state.session,
            &state.model,
            &state.turn,
            &state.provider,
            &state.role,
        ];
        state.retained_charge = std::mem::size_of::<FileState>()
            + cursor_bytes
            + state.pending.capacity()
            + strings.iter().map(|text| text.capacity()).sum::<usize>()
            + state
                .legacy_header
                .as_ref()
                .map_or(0, |(session, role)| session.capacity() + role.capacity())
            + state.events.capacity() * std::mem::size_of::<super::UsageEvent>()
            + self.event_heap
            + state.tools.capacity() * std::mem::size_of::<super::ToolRecord>()
            + self.tool_heap
            + state.calls.capacity() * (std::mem::size_of::<(String, super::Call)>() + 32)
            + state
                .calls
                .iter()
                .map(|(key, call)| key.capacity() + call_heap(call))
                .sum::<usize>()
            + state.exec_skills.capacity()
                * (std::mem::size_of::<(String, super::ExecSkill)>() + 32)
            + state
                .exec_skills
                .iter()
                .map(|(key, skill)| {
                    key.capacity() + call_heap(&skill.call) + skill.evidence.capacity()
                })
                .sum::<usize>()
            + state.exec_polls.capacity() * (std::mem::size_of::<(String, String)>() + 32)
            + state
                .exec_polls
                .iter()
                .map(|(key, value)| key.capacity() + value.capacity())
                .sum::<usize>()
            + state.inherited_contexts.capacity()
                * std::mem::size_of::<super::InheritedUsageContext>()
            + state
                .inherited_contexts
                .iter()
                .map(|context| {
                    [
                        &context.session,
                        &context.model,
                        &context.turn,
                        &context.provider,
                        &context.role,
                    ]
                    .iter()
                    .map(|text| text.capacity())
                    .sum::<usize>()
                        + context
                            .prefix
                            .as_ref()
                            .map_or(0, |(parent, _)| parent.capacity())
                })
                .sum::<usize>()
            + state
                .history_boundary
                .parent
                .as_ref()
                .map_or(0, String::capacity)
            + state
                .scope_parent_candidates
                .iter()
                .map(|(session, parents)| {
                    128 + session.capacity()
                        + parents.capacity() * (std::mem::size_of::<Option<String>>() + 32)
                        + parents
                            .iter()
                            .flatten()
                            .map(String::capacity)
                            .sum::<usize>()
                })
                .sum::<usize>();
    }
}

struct Source {
    file: File,
    left: Arc<AtomicU64>,
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let limit = (self.left.load(Ordering::Relaxed) as usize).min(buffer.len());
        if limit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "index read budget",
            ));
        }
        let bytes = self.file.read(&mut buffer[..limit])?;
        self.left.fetch_sub(bytes as u64, Ordering::Relaxed);
        Ok(bytes)
    }
}

pub(super) struct ReadCursor {
    pub(super) input: Box<dyn BufRead + Send>,
    left: Arc<AtomicU64>,
}

impl ReadCursor {
    pub(super) fn new(file: File, compressed: bool) -> io::Result<Self> {
        let left = Arc::new(AtomicU64::new(0));
        let source = Source {
            file,
            left: left.clone(),
        };
        let input: Box<dyn BufRead + Send> = if compressed {
            let mut decoder = zstd::stream::read::Decoder::new(source)?;
            decoder.window_log_max(21)?;
            Box::new(BufReader::with_capacity(128 * 1024, decoder))
        } else {
            Box::new(BufReader::with_capacity(128 * 1024, source))
        };
        Ok(Self { input, left })
    }

    pub(super) fn set_allowance(&self, bytes: u64) {
        self.left.store(bytes, Ordering::Relaxed);
    }

    pub(super) fn allowance(&self) -> u64 {
        self.left.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Local};
    use serde_json::{Value, json};
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    const SESSION: &str = "00000000-0000-4000-8000-000000000001";
    const REPLACEMENT_SESSION: &str = "00000000-0000-4000-8000-000000000002";

    fn token_row(total: u64, timestamp: &str) -> Value {
        json!({"type":"event_msg", "timestamp":timestamp, "payload":{
            "type":"token_count", "info":{
                "total_token_usage":{"input_tokens":total,"output_tokens":0,"total_tokens":total},
                "last_token_usage":{"input_tokens":1,"output_tokens":0,"total_tokens":1}
            }
        }})
    }

    fn rollout(session: &str, totals: std::ops::RangeInclusive<u64>, activity: bool) -> String {
        let timestamp = Local::now().to_rfc3339();
        let mut rows = vec![json!({"type":"session_meta","payload":{"id":session}})];
        if activity {
            rows.push(
                json!({"type":"response_item","timestamp":timestamp,"payload":{
                    "type":"function_call","call_id":"test-call","name":"functions.exec_command",
                    "arguments":"{\"cmd\":\"cat /skills/pdf/SKILL.md\"}"
                }}),
            );
        }
        rows.extend(totals.map(|total| token_row(total, &timestamp)));
        rows.iter().map(|row| format!("{row}\n")).collect()
    }

    fn fixture(home: &Path, suffix: &str, compressed: bool, text: &str) -> PathBuf {
        let directory = home.join("sessions");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!(
            "rollout-{suffix}.jsonl{}",
            if compressed { ".zst" } else { "" }
        ));
        let bytes = if compressed {
            zstd::stream::encode_all(text.as_bytes(), 1).unwrap()
        } else {
            text.as_bytes().to_vec()
        };
        fs::write(&path, bytes).unwrap();
        fs::canonicalize(path).unwrap()
    }

    fn cutoff() -> chrono::NaiveDate {
        Local::now().date_naive() - Duration::days(1)
    }

    fn assert_events(state: &FileState, totals: std::ops::RangeInclusive<u64>) {
        let expected: Vec<_> = totals.collect();
        assert_eq!(
            state
                .events
                .iter()
                .map(|event| event.total.total_tokens)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            state
                .events
                .iter()
                .all(|event| event.delta.total_tokens == 1)
        );
        assert_eq!(state.issues, 0);
    }

    #[test]
    fn local_large_tool_output_keeps_reading_later_usage_under_pressure() {
        for compressed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let timestamp = Local::now().to_rfc3339();
            let output = json!({"type":"response_item", "timestamp":timestamp,
                "payload":{"type":"function_call_output","call_id":"untracked",
                    "output":"quoted \\\" output ".repeat(47_000)}})
            .to_string();
            assert!(output.len() < super::super::MAX_LINE);
            let text = format!(
                "{}{}\n{}\n",
                rollout(SESSION, 1..=3, false),
                output,
                token_row(4, &timestamp)
            );
            let path = fixture(temp.path(), "large-output", compressed, &text);
            let mut state = FileState::default();
            let mut budget = Budget::new(std::iter::empty());
            budget.used.bytes = 48 * 1024 * 1024;
            assert!(
                state
                    .read_with_budget(&path, cutoff(), &mut budget)
                    .unwrap()
            );
            assert_events(&state, 1..=4);
            assert!(state.reader.is_none());
        }
    }

    fn pressured_budget(activity_limit: usize) -> Budget {
        let mut budget = Budget::new(std::iter::empty());
        budget.used.bytes = 60 * 1024 * 1024;
        budget.activity_limit = activity_limit;
        budget
    }

    #[test]
    fn lightweight_reserve_requires_an_identifiable_skipped_tool() {
        for kind in [
            "function_call",
            "custom_tool_call",
            "function_call_output",
            "custom_tool_call_output",
        ] {
            let row = json!({"type":"response_item", "payload":{"type":kind,"call_id":"id","data":"x".repeat(1024)}}).to_string();
            assert_eq!(parse_reserve(row.as_bytes(), true), LINE_OVERHEAD);
            assert!(parse_reserve(row.as_bytes(), false) > LINE_OVERHEAD);
        }
        for row in [
            json!({"type":"response_item","timestamp":42,"payload":{"type":"function_call","data":"x".repeat(1024)}}),
            json!({"type":"response_item","ordinal":"bad","payload":{"type":"function_call","data":"x".repeat(1024)}}),
        ] {
            let row = row.to_string();
            assert_eq!(parse_reserve(row.as_bytes(), true), LINE_OVERHEAD);
            assert!(!skips_tool_before_parsing(row.as_bytes()));
        }
        for row in [
            json!({"type":"response_item","payload":{"type":"function_call","call_id":42,"data":"x".repeat(1024)}}),
            json!({"type":"event_msg","payload":{"type":"token_count","info":null,"data":"x".repeat(1024)}}),
        ] {
            let row = row.to_string();
            assert!(parse_reserve(row.as_bytes(), true) > LINE_OVERHEAD);
        }
    }

    #[test]
    fn ignored_compaction_does_not_block_later_tokens_under_memory_pressure() {
        for compressed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut text = rollout(SESSION, 1..=1, false);
            text.push_str(&format!(
                "{}\n",
                json!({"type":"compacted","payload":{"message":"x".repeat(305_332)}})
            ));
            text.push_str(&format!("{}\n", token_row(2, &Local::now().to_rfc3339())));
            let path = fixture(temp.path(), "ignored-compaction", compressed, &text);
            let mut state = FileState::default();
            let mut budget = pressured_budget(MAX_ACTIVITY);
            assert!(
                state
                    .read_with_budget(&path, cutoff(), &mut budget)
                    .unwrap()
            );
            assert_events(&state, 1..=2);
            assert!(state.reader.is_none());
        }
    }

    #[test]
    fn event_classification_keeps_ambiguous_token_rows() {
        for row in [
            json!({"type":"event_msg","payload":{"type":"token_count","info":{}}}),
            json!({"type":"event_msg","payload":{"type":"token_count"}}),
            json!({"type":"turn_context","payload":{"type":42}}),
            json!({"type":"turn_context","payload":{"type":"token_count","info":{}}}),
        ] {
            assert!(may_record_event(row.to_string().as_bytes()));
        }
        for row in [
            json!({"type":"event_msg","payload":{"type":"token_count","info":null}}),
            json!({"type":"event_msg","payload":{"type":42}}),
            json!({"type":"event_msg","payload":{"type":"task_started"}}),
            json!({"type":"turn_context","payload":{"model":"test"}}),
            json!({"type":"response_item","payload":{"type":"function_call"}}),
        ] {
            assert!(!may_record_event(row.to_string().as_bytes()));
        }
    }

    #[test]
    fn full_event_quota_keeps_read_source_and_memory_limits() {
        let state = FileState::default();
        let mut budget = Budget::new(std::iter::empty());
        budget.event_limit = 0;
        assert!(budget.can_read(&state));
        assert!(budget.can_open(&state));
        budget.read_left = 0;
        assert!(!budget.can_read(&state));
        assert!(!budget.can_open(&state));
        budget.read_left = 1;
        budget.source_left = 0;
        assert!(!budget.can_open(&state));
        budget.source_left = 1;
        budget.memory_limit = CURSOR_BYTES + LINE_OVERHEAD;
        assert!(!budget.can_open(&state));
        budget.memory_limit = 1;
        assert!(!budget.can_read(&state));
    }

    #[test]
    fn full_activity_quota_skips_large_tool_rows_before_later_tokens() {
        for compressed in [false, true] {
            for kind in [
                "function_call",
                "custom_tool_call",
                "function_call_output",
                "custom_tool_call_output",
            ] {
                let temp = tempfile::tempdir().unwrap();
                let timestamp = Local::now().to_rfc3339();
                let tool = json!({"type":"response_item", "timestamp":timestamp, "payload":{
                    "type":kind, "call_id":"test-call", "arguments":"x".repeat(48 * 1024)
                }});
                let text = format!(
                    "{}{}\n{}\n",
                    rollout(SESSION, 1..=1, false),
                    tool,
                    token_row(2, &timestamp)
                );
                let path = fixture(temp.path(), "activity", compressed, &text);
                let mut state = FileState::default();
                let mut budget = pressured_budget(0);
                assert!(
                    state
                        .read_with_budget(&path, cutoff(), &mut budget)
                        .unwrap(),
                    "compressed={compressed}, kind={kind}"
                );
                assert!(state.activity_truncated);
                assert!(state.calls.is_empty());
                assert!(state.tools.is_empty());
                assert_eq!(state.previous.unwrap().total_tokens, 2);
                assert_events(&state, 1..=2);
                let offset = state.offset;
                assert!(
                    state
                        .read_with_budget(&path, cutoff(), &mut budget)
                        .unwrap()
                );
                assert_eq!(state.offset, offset);
                assert_events(&state, 1..=2);
            }
        }
    }

    #[test]
    fn skipped_tool_larger_than_full_parse_budget_is_not_discarded_as_an_issue() {
        for compressed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let timestamp = Local::now().to_rfc3339();
            let tool = json!({"type":"response_item", "timestamp":timestamp, "payload":{
                "type":"function_call", "call_id":"id", "arguments":"x".repeat(970 * 1024)
            }});
            let text = format!(
                "{}{}\n{}\n",
                rollout(SESSION, 1..=1, false),
                tool,
                token_row(2, &timestamp)
            );
            let path = fixture(temp.path(), "large-skipped-tool", compressed, &text);
            let mut state = FileState::default();
            let mut budget = Budget::new(std::iter::empty());
            budget.activity_limit = 0;
            assert!(
                state
                    .read_with_budget(&path, cutoff(), &mut budget)
                    .unwrap()
            );
            assert!(state.activity_truncated);
            assert_events(&state, 1..=2);
        }
    }

    #[test]
    fn full_parse_reserve_still_applies_to_other_large_rows() {
        for compressed in [false, true] {
            for case in ["activity_allowed", "invalid_call_id", "token"] {
                let temp = tempfile::tempdir().unwrap();
                let timestamp = Local::now().to_rfc3339();
                let target = match case {
                    "activity_allowed" => {
                        json!({"type":"response_item", "timestamp":timestamp, "payload":{
                            "type":"function_call", "call_id":"test-call", "name":"test",
                            "arguments":"x".repeat(200 * 1024)
                        }})
                    }
                    "invalid_call_id" => {
                        json!({"type":"response_item", "timestamp":timestamp, "payload":{
                            "type":"function_call", "call_id":42, "arguments":"x".repeat(200 * 1024)
                        }})
                    }
                    _ => {
                        let mut row = token_row(2, &timestamp);
                        row["payload"]["padding"] = json!("x".repeat(200 * 1024));
                        row
                    }
                };
                let tail = if case == "token" { 3 } else { 2 };
                let text = format!(
                    "{}{}\n{}\n",
                    rollout(SESSION, 1..=1, false),
                    target,
                    token_row(tail, &timestamp)
                );
                let path = fixture(temp.path(), "reserve", compressed, &text);
                let mut state = FileState::default();
                let activity_limit = if case == "activity_allowed" {
                    MAX_ACTIVITY
                } else {
                    0
                };
                let mut budget = pressured_budget(activity_limit);
                if case == "activity_allowed" {
                    assert!(
                        state
                            .read_with_budget(&path, cutoff(), &mut budget)
                            .unwrap()
                    );
                    assert!(state.activity_truncated);
                    assert!(state.reader.is_none());
                    assert!(state.pending.is_empty());
                    assert_events(&state, 1..=tail);
                    continue;
                }
                assert!(
                    !state
                        .read_with_budget(&path, cutoff(), &mut budget)
                        .unwrap(),
                    "compressed={compressed}, case={case}"
                );
                assert_eq!(&state.pending[..], format!("{target}\n").as_bytes());
                assert_eq!(state.previous.unwrap().total_tokens, 1);
                assert_events(&state, 1..=1);
                let offset = state.offset;
                let mut same = pressured_budget(activity_limit);
                assert!(!state.read_with_budget(&path, cutoff(), &mut same).unwrap());
                assert_eq!(state.offset, offset);
                assert_eq!(&state.pending[..], format!("{target}\n").as_bytes());
                assert_eq!(state.previous.unwrap().total_tokens, 1);
                assert_events(&state, 1..=1);
                let mut relaxed = Budget::new(std::iter::empty());
                relaxed.activity_limit = activity_limit;
                assert!(
                    state
                        .read_with_budget(&path, cutoff(), &mut relaxed)
                        .unwrap()
                );
                assert!(!state.activity_truncated);
                assert_events(&state, 1..=tail);
            }
        }
    }

    #[test]
    fn event_quota_allows_non_usage_and_eof_but_preserves_next_token() {
        for compressed in [false, true] {
            for count in 1..=3 {
                let temp = tempfile::tempdir().unwrap();
                let timestamp = Local::now().to_rfc3339();
                let non_usage = format!(
                    "{}\n{}\n{}\n",
                    json!({"type":"event_msg", "timestamp":timestamp, "payload":{"type":"task_started","turn_id":"after-limit"}}),
                    json!({"type":"event_msg", "timestamp":timestamp, "payload":{"type":"token_count","info":null}}),
                    json!({"type":"turn_context", "timestamp":timestamp, "payload":{"model":"test-model"}})
                );
                let mut text = rollout(SESSION, 1..=count.min(2), false);
                text.push_str(&non_usage);
                let next_token = format!("{}\n", token_row(3, &timestamp));
                if count == 3 {
                    text.push_str(&next_token);
                    text.push_str(&non_usage);
                }
                let path = fixture(temp.path(), "events", compressed, &text);
                let mut state = FileState::default();
                let mut budget = Budget::new(std::iter::empty());
                budget.event_limit = 2;
                let complete = state
                    .read_with_budget(&path, cutoff(), &mut budget)
                    .unwrap();
                assert_eq!(
                    complete,
                    count < 3,
                    "compressed={compressed}, count={count}"
                );
                assert_events(&state, 1..=count.min(2));
                assert_eq!(state.turn, "after-limit");
                assert_eq!(state.model, "test-model");
                assert_eq!(state.previous.unwrap().total_tokens, count.min(2));
                if count < 3 {
                    assert_eq!(state.offset, text.len() as u64);
                    assert!(state.reader.is_none());
                    continue;
                }
                assert_eq!(&state.pending[..], next_token.as_bytes());
                let offset = state.offset;
                let baseline = state.previous;
                let mut same = Budget::new(std::iter::empty());
                same.event_limit = 2;
                assert!(!state.read_with_budget(&path, cutoff(), &mut same).unwrap());
                assert_eq!(state.offset, offset);
                assert_eq!(&state.pending[..], next_token.as_bytes());
                assert_eq!(state.previous, baseline);
                assert_events(&state, 1..=2);
                let mut relaxed = Budget::new(std::iter::empty());
                relaxed.event_limit = 3;
                assert!(
                    state
                        .read_with_budget(&path, cutoff(), &mut relaxed)
                        .unwrap()
                );
                assert_eq!(state.offset, text.len() as u64);
                assert_eq!(state.previous.unwrap().total_tokens, 3);
                assert_events(&state, 1..=3);
            }
        }
    }

    #[test]
    fn shared_event_quota_preserves_the_first_token_of_another_file() {
        for compressed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let text = rollout(SESSION, 1..=1, false);
            let path = fixture(temp.path(), "shared-full", compressed, &text);
            let mut state = FileState::default();
            let mut budget = Budget::new(std::iter::empty());
            budget.event_limit = 2;
            budget.used.events = 2;
            assert!(
                !state
                    .read_with_budget(&path, cutoff(), &mut budget)
                    .unwrap()
            );
            assert_eq!(state.session, SESSION);
            assert!(state.events.is_empty());
            assert!(state.previous.is_none());
            assert_eq!(
                &state.pending[..],
                text.split_inclusive('\n').last().unwrap().as_bytes()
            );
            let pending = state.pending.to_vec();
            let offset = state.offset;
            let mut same = Budget::new(std::iter::empty());
            same.event_limit = 2;
            same.used.events = 2;
            assert!(!state.read_with_budget(&path, cutoff(), &mut same).unwrap());
            assert_eq!(&state.pending[..], pending.as_slice());
            assert_eq!(state.offset, offset);
            assert!(state.previous.is_none());
            let mut relaxed = Budget::new(std::iter::empty());
            relaxed.event_limit = 2;
            relaxed.used.events = 1;
            assert!(
                state
                    .read_with_budget(&path, cutoff(), &mut relaxed)
                    .unwrap()
            );
            assert_events(&state, 1..=1);
        }
    }

    #[test]
    fn compressed_final_token_without_newline_obeys_event_quota() {
        for count in [2, 3] {
            let temp = tempfile::tempdir().unwrap();
            let text = rollout(SESSION, 1..=count, false)
                .trim_end_matches('\n')
                .to_owned();
            let path = fixture(temp.path(), "no-final-newline", true, &text);
            let mut state = FileState::default();
            let mut budget = Budget::new(std::iter::empty());
            budget.event_limit = 2;
            assert_eq!(
                state
                    .read_with_budget(&path, cutoff(), &mut budget)
                    .unwrap(),
                count == 2
            );
            assert_events(&state, 1..=2);
            if count == 2 {
                assert!(state.pending.is_empty());
                continue;
            }
            let original = state.pending.to_vec();
            assert!(!original.ends_with(b"\n"));
            let baseline = state.previous;
            let offset = state.offset;
            let mut same = Budget::new(std::iter::empty());
            same.event_limit = 2;
            assert!(!state.read_with_budget(&path, cutoff(), &mut same).unwrap());
            assert_eq!(&state.pending[..], original.as_slice());
            assert_eq!(state.previous, baseline);
            assert_eq!(state.offset, offset);
            let mut relaxed = Budget::new(std::iter::empty());
            relaxed.event_limit = 3;
            assert!(
                state
                    .read_with_budget(&path, cutoff(), &mut relaxed)
                    .unwrap()
            );
            assert_events(&state, 1..=3);
        }
    }

    #[test]
    fn exhausted_byte_budget_defers_eof_until_the_next_round() {
        for compressed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let text = rollout(SESSION, 1..=2, false);
            let path = fixture(temp.path(), "eof-budget", compressed, &text);
            let mut state = FileState::default();
            let mut budget = Budget::new(std::iter::empty());
            budget.event_limit = 2;
            budget.read_left = text.len() as u64;
            assert!(
                !state
                    .read_with_budget(&path, cutoff(), &mut budget)
                    .unwrap()
            );
            assert_eq!(budget.read_left, 0);
            assert_eq!(state.offset, text.len() as u64);
            assert_events(&state, 1..=2);
            let mut next = Budget::new(std::iter::empty());
            next.event_limit = 2;
            assert!(state.read_with_budget(&path, cutoff(), &mut next).unwrap());
            assert_eq!(state.offset, text.len() as u64);
            assert_events(&state, 1..=2);
        }
    }

    #[test]
    fn event_limit_preserves_large_token_before_permanent_memory_rejection() {
        for compressed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let timestamp = Local::now().to_rfc3339();
            let mut oversized = token_row(3, &timestamp);
            oversized["payload"]["padding"] = json!(vec![Value::Null; 200_000]);
            let blocked_line = format!("{oversized}\n");
            assert!(blocked_line.len() < super::super::MAX_LINE);
            let text = format!(
                "{}{}{}\n",
                rollout(SESSION, 1..=2, false),
                blocked_line,
                token_row(4, &timestamp)
            );
            let path = fixture(temp.path(), "large-token", compressed, &text);
            let mut state = FileState::default();
            let mut budget = Budget::new(std::iter::empty());
            budget.event_limit = 2;
            assert!(
                !state
                    .read_with_budget(&path, cutoff(), &mut budget)
                    .unwrap()
            );
            assert_eq!(&state.pending[..], blocked_line.as_bytes());
            assert_eq!(state.previous.unwrap().total_tokens, 2);
            assert_events(&state, 1..=2);
            let mut relaxed = Budget::new(std::iter::empty());
            relaxed.event_limit = 4;
            assert!(
                state
                    .read_with_budget(&path, cutoff(), &mut relaxed)
                    .unwrap()
            );
            assert_eq!(state.issues, 1);
            assert_eq!(state.previous.unwrap().total_tokens, 4);
            assert_eq!(
                state
                    .events
                    .iter()
                    .map(|event| event.total.total_tokens)
                    .collect::<Vec<_>>(),
                vec![1, 2, 4]
            );
        }
    }

    #[test]
    fn event_limit_is_shared_across_files_and_unread_rows_resume_once() {
        let temp = tempfile::tempdir().unwrap();
        let text = rollout(SESSION, 1..=6, false);
        let paths = [
            fixture(temp.path(), "a", false, &text),
            fixture(temp.path(), "b", false, &text),
        ];
        let mut states = [FileState::default(), FileState::default()];
        let mut budget = Budget::new(std::iter::empty());
        budget.event_limit = 8;
        assert!(
            states[0]
                .read_with_budget(&paths[0], cutoff(), &mut budget)
                .unwrap()
        );
        budget.replace(Usage::default(), Usage::of(&states[0]));
        assert!(
            !states[1]
                .read_with_budget(&paths[1], cutoff(), &mut budget)
                .unwrap()
        );
        assert_events(&states[0], 1..=6);
        assert_events(&states[1], 1..=2);
        assert!(states[1].reader.is_some());

        let mut next = Budget::new(std::iter::once(&states[0]));
        assert!(
            states[1]
                .read_with_budget(&paths[1], cutoff(), &mut next)
                .unwrap()
        );
        assert_events(&states[1], 1..=6);
        let offset = states[1].offset;
        assert!(
            states[1]
                .read_with_budget(&paths[1], cutoff(), &mut next)
                .unwrap()
        );
        assert_eq!(states[1].offset, offset);
        assert_events(&states[1], 1..=6);
    }

    #[test]
    fn byte_budget_is_shared_and_activity_truncation_keeps_tokens() {
        for source_limited in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let text = rollout(SESSION, 1..=6, true);
            let paths = [
                fixture(temp.path(), "first", false, &text),
                fixture(temp.path(), "second", false, &text),
            ];
            let mut states = [FileState::default(), FileState::default()];
            let limit = fs::metadata(&paths[0]).unwrap().len() + 50;
            let mut budget = Budget::new(std::iter::empty());
            if source_limited {
                budget.source_left = limit;
            } else {
                budget.read_left = limit;
            }
            budget.activity_limit = 0;
            assert!(
                states[0]
                    .read_with_budget(&paths[0], cutoff(), &mut budget)
                    .unwrap()
            );
            budget.replace(Usage::default(), Usage::of(&states[0]));
            assert!(
                !states[1]
                    .read_with_budget(&paths[1], cutoff(), &mut budget)
                    .unwrap()
            );
            assert_eq!(states[0].bytes_read + states[1].bytes_read, limit);
            assert_eq!(states[1].offset, 50);
            if source_limited {
                assert_eq!(budget.source_left, 0);
            } else {
                assert_eq!(budget.read_left, 0);
            }
            assert!(states[0].activity_truncated);
            assert!(states[0].calls.is_empty());
            assert_events(&states[0], 1..=6);

            let mut next = Budget::new(std::iter::once(&states[0]));
            next.activity_limit = 0;
            assert!(
                states[1]
                    .read_with_budget(&paths[1], cutoff(), &mut next)
                    .unwrap()
            );
            assert!(states[1].activity_truncated);
            assert!(states[1].calls.is_empty());
            assert_events(&states[1], 1..=6);
        }
    }

    #[test]
    fn plain_and_zstd_resume_across_byte_and_source_budgets_without_replay() {
        for compressed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let text = rollout(SESSION, 1..=6, false);
            let path = fixture(temp.path(), "bounded", compressed, &text);
            let mut state = FileState::default();
            let mut complete = false;
            let mut rounds = 0;
            for _ in 0..200 {
                let mut budget = Budget::new(std::iter::empty());
                budget.read_left = 73;
                budget.source_left = 37;
                let before = state.bytes_read;
                complete = state
                    .read_with_budget(&path, cutoff(), &mut budget)
                    .unwrap();
                assert!(state.bytes_read - before <= 73);
                assert!(budget.source_left <= 37);
                rounds += 1;
                if complete {
                    break;
                }
            }
            assert!(complete, "compressed={compressed} did not reach EOF");
            assert!(rounds > 1);
            assert_eq!(state.offset, text.len() as u64);
            assert_eq!(state.bytes_read, state.offset);
            assert!(state.reader.is_none());
            assert_events(&state, 1..=6);
            let before = state.bytes_read;
            assert!(
                state
                    .read_with_budget(&path, cutoff(), &mut Budget::new(std::iter::empty()))
                    .unwrap()
            );
            assert_eq!(state.bytes_read, before);
            assert_events(&state, 1..=6);
        }
    }

    #[test]
    fn oversized_line_is_skipped_across_rounds_and_later_tokens_resume() {
        let temp = tempfile::tempdir().unwrap();
        let text = rollout(SESSION, 1..=6, false);
        let path = fixture(temp.path(), "oversized", false, &text);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", "x".repeat(super::super::MAX_LINE + 64)).unwrap();
        writeln!(file, "{}", token_row(7, &Local::now().to_rfc3339())).unwrap();
        drop(file);

        let mut state = FileState::default();
        let mut budget = Budget::new(std::iter::empty());
        budget.read_left = (text.len() + super::super::MAX_LINE + 8) as u64;
        assert!(
            !state
                .read_with_budget(&path, cutoff(), &mut budget)
                .unwrap()
        );
        assert_eq!(state.events.len(), 6);
        assert!(state.skipping);
        assert_eq!(state.issues, 1);

        let mut next = Budget::new(std::iter::empty());
        assert!(state.read_with_budget(&path, cutoff(), &mut next).unwrap());
        assert!(!state.skipping);
        assert_eq!(state.issues, 1);
        assert_eq!(
            state
                .events
                .iter()
                .map(|event| event.total.total_tokens)
                .collect::<Vec<_>>(),
            (1..=7).collect::<Vec<_>>()
        );
        assert!(
            state
                .events
                .iter()
                .all(|event| event.delta.total_tokens == 1)
        );
        let before = state.bytes_read;
        assert!(state.read_with_budget(&path, cutoff(), &mut next).unwrap());
        assert_eq!(state.bytes_read, before);
        assert_eq!(state.events.len(), 7);
    }

    #[test]
    fn partial_reader_resets_cursor_and_baseline_when_path_is_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let original = rollout(SESSION, 1..=6, false);
        let path = fixture(temp.path(), "original", false, &original);
        let mut state = FileState::default();
        let mut budget = Budget::new(std::iter::empty());
        budget.read_left = original
            .split_inclusive('\n')
            .take(4)
            .map(str::len)
            .sum::<usize>() as u64
            + 11;
        assert!(
            !state
                .read_with_budget(&path, cutoff(), &mut budget)
                .unwrap()
        );
        assert_events(&state, 1..=3);
        assert!(!state.pending.is_empty());
        assert!(state.reader.is_some());
        let old_identity = state.stamp.unwrap().2;
        assert!(old_identity.is_some());

        let replacement_text = rollout(REPLACEMENT_SESSION, 11..=19, false);
        let replacement = fixture(temp.path(), "replacement", false, &replacement_text);
        assert!(replacement_text.len() > original.len());
        fs::rename(&path, path.with_extension("previous")).unwrap();
        fs::rename(&replacement, &path).unwrap();

        let mut next = Budget::new(std::iter::empty());
        assert!(state.read_with_budget(&path, cutoff(), &mut next).unwrap());
        assert_ne!(state.stamp.unwrap().2, old_identity);
        assert_eq!(state.offset, replacement_text.len() as u64);
        assert!(state.pending.is_empty());
        assert!(state.reader.is_none());
        assert!(
            state
                .events
                .iter()
                .all(|event| event.session == REPLACEMENT_SESSION)
        );
        assert!(!state.events[0].has_file_baseline);
        assert_events(&state, 11..=19);
        let before = state.bytes_read;
        assert!(state.read_with_budget(&path, cutoff(), &mut next).unwrap());
        assert_eq!(state.bytes_read, before);
        assert_events(&state, 11..=19);
    }

    #[test]
    fn local_completed_archive_releases_unused_vector_capacity() {
        let temp = tempfile::tempdir().unwrap();
        let path = fixture(
            temp.path(),
            "compact-archive",
            true,
            &rollout(SESSION, 1..=100, false),
        );
        let mut state = FileState::default();
        assert!(
            state
                .read_with_budget(&path, cutoff(), &mut Budget::new(std::iter::empty()))
                .unwrap()
        );
        assert_events(&state, 1..=100);
        assert_eq!(state.events.capacity(), state.events.len());
    }

    #[test]
    fn local_early_return_does_not_reserve_unrelated_vector_growth() {
        let timestamp = Local::now().to_rfc3339();
        let mut state = FileState::default();
        state.accept(
            rollout(SESSION, 1..=1, false)
                .lines()
                .next()
                .unwrap()
                .as_bytes(),
            cutoff(),
        );
        state.accept(token_row(1, &timestamp).to_string().as_bytes(), cutoff());
        state.events = vec![state.events[0].clone(); 128];
        state.events.shrink_to_fit();
        assert!(
            state.pending.try_extend(
                &json!({"type":"event_msg", "timestamp":timestamp,
            "payload":{"type":"item_completed"}})
                .to_string()
                .into_bytes()
            )
        );
        Meter::new(&state).refresh(&mut state, 0);
        let mut budget = Budget::new(std::iter::empty());
        budget.memory_limit = state.retained_charge + LINE_OVERHEAD;
        assert_eq!(state.events.len(), state.events.capacity());
        assert!(budget.accept_line(&state));
        state.pending.clear();
        assert!(
            state
                .pending
                .try_extend(token_row(2, &timestamp).to_string().as_bytes())
        );
        assert!(!budget.accept_line(&state));
        assert!(!budget.can_skip_activity_line(&state));
    }
}
