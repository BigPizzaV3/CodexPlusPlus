//! 本机结构化用量查询。所有调用共用一个增量索引，不修改原始会话。

use chrono::{Datelike, Duration as Days, Local, NaiveDate};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, Metadata};
use std::path::Path;

mod structured;

pub(crate) fn storage_dir(home: &Path, settings_path: &Path) -> std::path::PathBuf {
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let id = format!("{:x}", Sha256::digest(home.as_os_str().as_encoded_bytes()));
    settings_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("usage-history")
        .join(&id[..16])
}

pub(crate) fn normalize_session_id(id: &str) -> Option<String> {
    let id = id.strip_prefix("local:").unwrap_or(id);
    let normalized = uuid::Uuid::parse_str(id).ok()?.to_string();
    (id.len() == 36 && normalized.eq_ignore_ascii_case(id)).then_some(normalized)
}

/// 调用方先检查设置开关；请求不能指定读取路径，也不会开启全局功能。
pub fn query_history(codex_home: &Path, query: &Value) -> Value {
    let Some(options) = Query::parse(query, Local::now().date_naive()) else {
        return json!({"status":"unavailable","complete":false,"message":"用量查询参数无效"});
    };
    let Ok(home) = fs::canonicalize(codex_home) else {
        return json!({"status":"unavailable","complete":false,"message":"本机会话目录尚不可用"});
    };
    structured::query(&home, &options, query)
}

/// 关闭功能或更换实例时释放本进程的索引和采集器。
pub fn clear_history(codex_home: &Path) {
    if let Ok(home) = fs::canonicalize(codex_home) {
        structured::clear(&home);
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Tokens {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) total_tokens: u64,
    reasoning_output_tokens: Option<u64>,
    // 不参与序列化，保持既有事件 ID 和证据文件的签名不变。
    #[serde(skip)]
    cached_known: bool,
}

impl Default for Tokens {
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

impl Tokens {
    fn new(input: u64, cached: u64, output: u64) -> Self {
        Self {
            input_tokens: input,
            cached_input_tokens: cached.min(input),
            output_tokens: output,
            total_tokens: input.saturating_add(output),
            reasoning_output_tokens: None,
            cached_known: true,
        }
    }

    pub(crate) fn parse(value: &Value) -> Option<Self> {
        let cached = value["cached_input_tokens"].as_u64();
        let mut tokens = Self::new(
            value["input_tokens"].as_u64()?,
            cached.unwrap_or(0),
            value["output_tokens"].as_u64()?,
        );
        tokens.cached_known = cached.is_some_and(|count| count <= tokens.input_tokens);
        tokens.reasoning_output_tokens = value["reasoning_output_tokens"]
            .as_u64()
            .filter(|n| *n <= tokens.output_tokens);
        Some(tokens)
    }

    fn value(self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        value["cachedKnown"] = json!(self.cached_known);
        value["reasoningKnown"] = json!(self.reasoning_output_tokens.is_some());
        value
    }

    fn same_counts(self, other: Self) -> bool {
        self.input_tokens == other.input_tokens
            && self.cached_input_tokens == other.cached_input_tokens
            && self.output_tokens == other.output_tokens
            && self.reasoning_output_tokens == other.reasoning_output_tokens
    }

    fn add(&mut self, other: Self) {
        let reasoning = if self.total_tokens == 0 {
            other.reasoning_output_tokens
        } else {
            self.reasoning_output_tokens
                .zip(other.reasoning_output_tokens)
                .map(|(a, b)| a.saturating_add(b))
        };
        let cached_known = self.cached_known && other.cached_known;
        *self = Self::new(
            self.input_tokens.saturating_add(other.input_tokens),
            self.cached_input_tokens
                .saturating_add(other.cached_input_tokens),
            self.output_tokens.saturating_add(other.output_tokens),
        );
        self.cached_known = cached_known;
        self.reasoning_output_tokens = reasoning;
    }

    fn delta(self, before: Self) -> Self {
        let cached_known = self.cached_known
            && before.cached_known
            && self.cached_input_tokens >= before.cached_input_tokens
            && self.cached_input_tokens - before.cached_input_tokens
                <= self.input_tokens.saturating_sub(before.input_tokens);
        let mut tokens = Self::new(
            self.input_tokens.saturating_sub(before.input_tokens),
            if cached_known {
                self.cached_input_tokens
                    .saturating_sub(before.cached_input_tokens)
            } else {
                0
            },
            self.output_tokens.saturating_sub(before.output_tokens),
        );
        tokens.cached_known = cached_known;
        tokens.reasoning_output_tokens = self
            .reasoning_output_tokens
            .zip(before.reasoning_output_tokens)
            .and_then(|(a, b)| a.checked_sub(b))
            .filter(|n| *n <= tokens.output_tokens);
        tokens
    }

    fn is_monotonic_after(self, before: Self) -> bool {
        self.input_tokens >= before.input_tokens && self.output_tokens >= before.output_tokens
    }

    fn requires_last_after(self, before: Self) -> bool {
        (self.input_tokens != before.input_tokens || self.output_tokens != before.output_tokens)
            && !self.is_monotonic_after(before)
    }

    pub(crate) fn usage_delta(self, before: Option<Self>, last: Option<Self>) -> Self {
        match before {
            None => last.unwrap_or(self),
            // 缓存或推理分量的修正不是新消费，不能重放旧的 last_usage。
            Some(before)
                if self.input_tokens == before.input_tokens
                    && self.output_tokens == before.output_tokens =>
            {
                Self::default()
            }
            Some(before) if self.is_monotonic_after(before) => {
                let mut delta = self.delta(before);
                if let Some(last) = last.filter(|last| {
                    last.input_tokens == delta.input_tokens
                        && last.output_tokens == delta.output_tokens
                }) {
                    if !delta.cached_known
                        && (!self.cached_known || !before.cached_known)
                        && last.cached_known
                    {
                        delta.cached_input_tokens =
                            last.cached_input_tokens.min(delta.input_tokens);
                        delta.cached_known = true;
                    } else if delta.cached_known
                        && last.cached_known
                        && delta.cached_input_tokens != last.cached_input_tokens
                    {
                        delta.cached_input_tokens = 0;
                        delta.cached_known = false;
                    }
                    if delta
                        .reasoning_output_tokens
                        .zip(last.reasoning_output_tokens)
                        .is_some_and(|(cumulative, last)| cumulative != last)
                    {
                        delta.reasoning_output_tokens = None;
                    }
                }
                delta
            }
            // 分量回退只能确认最后一次消费；缺少该证据时不回放累计值。
            Some(_) => last.unwrap_or_default(),
        }
    }
}

#[derive(Default)]
pub(crate) struct UsageHistoryBoundary {
    pub(crate) parent: Option<String>,
    start: Option<u64>,
}

impl UsageHistoryBoundary {
    pub(crate) fn from_meta(payload: &Value, session: &str) -> Self {
        Self {
            parent: payload["forked_from_id"]
                .as_str()
                .or_else(|| payload["parent_thread_id"].as_str())
                .and_then(normalize_session_id)
                .filter(|parent| parent != session),
            start: payload["subagent_history_start_ordinal"].as_u64(),
        }
    }

    pub(crate) fn contains(&self, ordinal: Option<u64>) -> bool {
        self.parent.is_some()
            && self
                .start
                .zip(ordinal)
                .is_some_and(|(start, ordinal)| ordinal < start)
    }
}

struct Record {
    session_id: String,
    turn_id: String,
    model: Option<String>,
    date: Option<NaiveDate>,
    usage: Option<Tokens>,
}

struct Query {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    model: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    search: String,
    include_session_cursors: bool,
    include_activity: bool,
}

impl Query {
    fn parse(value: &Value, today: NaiveDate) -> Option<Self> {
        if !value.is_object() {
            return None;
        }
        let mut from = match value["period"].as_str().unwrap_or("all") {
            "today" => Some(today),
            "week" => Some(today - Days::days(6)),
            "month" => today.with_day(1),
            "all" => None,
            _ => return None,
        };
        let mut to = from.map(|_| today);
        for (name, field) in [("from", &mut from), ("to", &mut to)] {
            if !value[name].is_null() {
                *field = Some(NaiveDate::parse_from_str(value[name].as_str()?, "%Y-%m-%d").ok()?);
            }
        }
        if from.zip(to).is_some_and(|(f, t)| f > t) {
            return None;
        }
        let text = |name: &str, limit: usize| -> Option<Option<String>> {
            if value[name].is_null() {
                return Some(None);
            }
            let text = value[name].as_str()?;
            (text.len() <= limit).then(|| Some(text.to_owned()).filter(|s| !s.is_empty()))
        };
        let session_id = if value["sessionId"].is_null() {
            None
        } else {
            Some(normalize_session_id(value["sessionId"].as_str()?)?)
        };
        let search = value["search"].as_str().unwrap_or("").trim().to_lowercase();
        if search.len() > 200 {
            return None;
        }
        let include_session_cursors = match value.get("includeSessionCursors") {
            None => false,
            Some(value) => value.as_bool()?,
        };
        Some(Self {
            from,
            to,
            model: text("model", 256)?,
            session_id,
            turn_id: text("turnId", 200)?,
            search,
            include_session_cursors,
            include_activity: value["includeActivity"] == true,
        })
    }

    fn accepts(&self, record: &Record) -> bool {
        !self
            .session_id
            .as_ref()
            .is_some_and(|id| id != &record.session_id)
            && !self
                .turn_id
                .as_ref()
                .is_some_and(|id| id != &record.turn_id)
            && !self
                .from
                .is_some_and(|from| record.date.is_none_or(|d| d < from))
            && !self.to.is_some_and(|to| record.date.is_none_or(|d| d > to))
            && !self
                .model
                .as_ref()
                .is_some_and(|m| record.model.as_ref() != Some(m))
            && (self.search.is_empty()
                || record
                    .model
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&self.search)
                || record
                    .date
                    .is_some_and(|d| d.to_string().contains(&self.search)))
    }
}

#[derive(Default)]
struct Aggregate {
    usage: Tokens,
    records: usize,
}

impl Aggregate {
    fn add(&mut self, record: &Record) {
        self.records += 1;
        if let Some(usage) = record.usage {
            self.usage.add(usage);
        }
    }

    fn value(&self) -> Value {
        json!({"usage":self.usage.value(),"recordCount":self.records})
    }
}

#[cfg(unix)]
fn file_identity(meta: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}
#[cfg(not(unix))]
fn file_identity(_: &Metadata) -> Option<(u64, u64)> {
    None
}
