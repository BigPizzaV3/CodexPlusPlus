//! 请求日志：按请求记录代理转发的状态、模型、重试、token 用量与缓存命中。
//!
//! 存储为 `<state dir>/request-log.jsonl`，只保留最近的若干条（追加时按大小裁剪），
//! 管理器「请求日志」页通过 Tauri 命令读取。日志里不保存提示词/回复正文，
//! 只保存模型名、token 计数和错误摘要，避免把会话内容写进明文日志。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const REQUEST_LOG_FILE: &str = "request-log.jsonl";
/// 日志文件达到该大小后裁剪到约一半，避免无限增长。
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const KEEP_LOG_BYTES: u64 = 1024 * 1024;

static ENTRY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static LOG_PATH_OVERRIDE: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

/// 一次上游请求的 token 用量。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cached_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
}

impl RequestUsage {
    /// 缓存命中率（cached / input），没有输入 token 时为 None。
    pub fn cache_hit_ratio(&self) -> Option<f64> {
        (self.input_tokens > 0).then(|| self.cached_tokens as f64 / self.input_tokens as f64)
    }
}

/// 一次被代理自动重试的上游尝试摘要（请求日志详情用）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestAttemptDetail {
    /// 第几次上游尝试（从 1 开始）。
    pub attempt: u32,
    /// 触发重试的上游错误 code（若有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// 上游错误 message（最多 200 字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 本次重试前的退避等待（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
}

/// 代理内部的重试/路由痕迹，随 `UpstreamProxyResponse` 交给启动器写日志。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestTrace {
    pub relay_id: String,
    pub relay_name: String,
    /// 客户端请求的模型名（聚合前缀/后缀原样）。
    pub model: String,
    /// 真正发给上游的模型名。
    pub upstream_model: String,
    pub wire_api: String,
    /// 上游请求总次数（含失败切换与自动重试）。
    pub attempts: u32,
    /// 代理自动重试次数（不含成员间失败切换）。
    pub auto_retries: u32,
    /// 上游第一段输出到达的 unix 毫秒时间（用于计算首字耗时）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_output_at_ms: Option<u64>,
    #[serde(default)]
    pub retry_reasons: Vec<String>,
    /// 每次自动重试前的失败尝试摘要。
    #[serde(default)]
    pub attempt_details: Vec<RequestAttemptDetail>,
}

/// 管理器「请求日志」页展示的一条记录。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogEntry {
    pub id: String,
    pub timestamp_ms: u64,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub method: String,
    pub path: String,
    pub status_code: u16,
    pub success: bool,
    pub stream: bool,
    pub model: String,
    /// Codex 的会话/线程 id（来自请求头），用于把状态条限定在当前会话。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub upstream_model: String,
    pub relay_id: String,
    pub relay_name: String,
    pub wire_api: String,
    pub attempts: u32,
    pub auto_retries: u32,
    #[serde(default)]
    pub retry_reasons: Vec<String>,
    /// 每次自动重试前的失败尝试摘要（旧日志没有该字段）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempt_details: Vec<RequestAttemptDetail>,
    /// `running` 表示请求还在进行中；旧日志没有该字段时按已完成处理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// 首字耗时（从请求开始到第一段上游输出），毫秒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
    /// 请求进行中按流式 delta 粗估的输出 token，完成后清空并写入准确 usage。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_output_tokens: Option<u64>,
    /// 请求进行中的阶段：requesting / waiting / retrying / generating。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RequestUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_ratio: Option<f64>,
    /// 网关模式下发起请求的 Agent（claude / grok）；Codex 直连代理的旧记录为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

impl RequestLogEntry {
    pub fn new_id(started_at_ms: u64) -> String {
        let sequence = ENTRY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        format!("{started_at_ms}-{sequence}")
    }

    /// 发起请求的 Agent：没写的是 Codex 直连代理的旧记录。
    pub fn agent_id(&self) -> &str {
        self.agent
            .as_deref()
            .map(str::trim)
            .filter(|agent| !agent.is_empty())
            .unwrap_or("codex")
    }

    /// 用量统计按哪个模型名记：上游实际用的优先，没有时用客户端请求的。
    pub fn usage_model(&self) -> &str {
        if self.upstream_model.trim().is_empty() {
            self.model.trim()
        } else {
            self.upstream_model.trim()
        }
    }

    /// 生图请求（生成或编辑）。
    pub fn is_image(&self) -> bool {
        self.wire_api.starts_with("image")
    }

    pub fn finish(&mut self, finished_at_ms: u64) {
        self.timestamp_ms = finished_at_ms;
        self.duration_ms = finished_at_ms.saturating_sub(self.started_at_ms);
        self.cache_hit_ratio = self.usage.as_ref().and_then(RequestUsage::cache_hit_ratio);
        self.state = Some(if self.success { "success" } else { "failed" }.to_string());
        self.estimated_output_tokens = None;
    }
}

fn log_path_override() -> Option<PathBuf> {
    LOG_PATH_OVERRIDE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub fn request_log_path() -> PathBuf {
    log_path_override().unwrap_or_else(|| crate::paths::default_app_state_dir().join(REQUEST_LOG_FILE))
}

fn live_requests() -> &'static std::sync::Mutex<std::collections::BTreeMap<String, RequestLogEntry>> {
    static LIVE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, RequestLogEntry>>,
    > = std::sync::OnceLock::new();
    LIVE.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()))
}

/// 请求开始：先登记到内存里的进行中列表，管理器即可显示「运行中」并计时。
pub fn start_request(entry: &RequestLogEntry) {
    let mut live = live_requests()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    live.insert(entry.id.clone(), entry.clone());
    // 防止异常退出前请求堆积；只保留最近的 64 条进行中记录。
    while live.len() > 64 {
        if let Some(oldest) = live.keys().next().cloned() {
            live.remove(&oldest);
        } else {
            break;
        }
    }
}

/// 请求结束：从进行中列表移除并写入 JSONL。
pub fn finish_request(entry: &RequestLogEntry) {
    if let Ok(mut live) = live_requests().lock() {
        live.remove(&entry.id);
    }
    let _ = append_request_log(entry);
    // 单元测试里默认路径指向真实用户状态目录，同样不写。
    if !(cfg!(test) && log_path_override().is_none()) {
        crate::gateway::usage::record_entry(entry);
    }
}

/// 更新进行中请求的快照（例如上游地址/首字耗时在请求结束前就已知）。
pub fn update_running(entry: &RequestLogEntry) {
    if let Ok(mut live) = live_requests().lock()
        && live.contains_key(&entry.id)
    {
        live.insert(entry.id.clone(), entry.clone());
    }
}

/// 更新进行中请求的阶段（供 Codex 应用内的状态条展示）。
pub fn update_running_phase(id: &str, phase: &str) {
    if let Ok(mut live) = live_requests().lock()
        && let Some(entry) = live.get_mut(id)
    {
        if entry.first_token_ms.is_some() && phase == "waiting" {
            return;
        }
        entry.phase = Some(phase.to_string());
    }
}

/// 缓冲读取到第一段输出时，立刻更新首字耗时与阶段。
pub fn update_running_first_token(id: &str) {
    if let Ok(mut live) = live_requests().lock()
        && let Some(entry) = live.get_mut(id)
    {
        if entry.first_token_ms.is_none() {
            entry.first_token_ms = Some(crate::request_log::current_time_ms_since(entry.started_at_ms));
        }
        entry.phase = Some("generating".to_string());
    }
}

fn current_time_ms_since(started_at_ms: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| (duration.as_millis() as u64).saturating_sub(started_at_ms))
        .unwrap_or(0)
}

/// 请求重试时实时更新进行中记录，管理器无需等请求结束就能看到重试次数递增。
pub fn update_running_retry(
    id: &str,
    attempts: u32,
    auto_retries: u32,
    retry_reasons: Vec<String>,
) {
    if let Ok(mut live) = live_requests().lock()
        && let Some(entry) = live.get_mut(id)
    {
        entry.attempts = attempts;
        entry.auto_retries = auto_retries;
        entry.retry_reasons = retry_reasons;
    }
}

/// 运行中请求的输出 token 粗估（按已收到的流式 delta 字符数 / 4 计算）。
pub fn update_running_output_estimate(id: &str, tokens: u64) {
    if let Ok(mut live) = live_requests().lock()
        && let Some(entry) = live.get_mut(id)
    {
        entry.estimated_output_tokens = Some(tokens);
    }
}

/// 记录一次失败尝试摘要；进行中的请求详情即可看到重试原因。
pub fn update_running_attempt_detail(id: &str, detail: RequestAttemptDetail) {
    if let Ok(mut live) = live_requests().lock()
        && let Some(entry) = live.get_mut(id)
    {
        entry.attempt_details.push(detail);
    }
}

pub fn running_requests() -> Vec<RequestLogEntry> {
    live_requests()
        .lock()
        .map(|live| live.values().cloned().collect())
        .unwrap_or_default()
}

/// 串行化会改全局日志路径/进行中列表的测试（Rust 测试默认并行）。
pub fn request_log_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 测试用：覆盖请求日志路径（传 None 恢复默认）。
pub fn set_request_log_path_for_tests(path: Option<PathBuf>) {
    *LOG_PATH_OVERRIDE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = path;
}

/// 追加一条请求日志；达到大小上限时保留最近的约一半。
pub fn append_request_log(entry: &RequestLogEntry) -> std::io::Result<()> {
    // 单元测试里默认路径指向真实用户状态目录，不让测试数据污染正式日志。
    if cfg!(test) && log_path_override().is_none() {
        return Ok(());
    }
    append_request_log_at(&request_log_path(), entry)
}

pub fn append_request_log_at(path: &Path, entry: &RequestLogEntry) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(entry)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")?;
    drop(file);
    if std::fs::metadata(path).map(|meta| meta.len() > MAX_LOG_BYTES).unwrap_or(false) {
        trim_request_log_to_tail(path, KEEP_LOG_BYTES)?;
    }
    Ok(())
}

fn trim_request_log_to_tail(path: &Path, keep_bytes: u64) -> std::io::Result<()> {
    let contents = std::fs::read(path)?;
    let keep_from = contents.len().saturating_sub(keep_bytes as usize);
    let start = contents[keep_from..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| keep_from + offset + 1)
        .unwrap_or(keep_from);
    std::fs::write(path, &contents[start..])
}

/// 读取最近 `limit` 条请求日志（含正在进行的请求），按开始时间倒序返回。
pub fn read_request_logs(limit: usize) -> std::io::Result<Vec<RequestLogEntry>> {
    let mut entries = read_request_logs_at(&request_log_path(), limit)?;
    entries.extend(running_requests());
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.started_at_ms));
    entries.truncate(limit);
    Ok(entries)
}

/// 请求日志的筛选条件（从用量统计点进来时用）：空的不筛。
#[derive(Clone, Debug, Default)]
pub struct RequestLogFilter {
    /// 只要这个时间之后开始的请求。
    pub since_ms: Option<u64>,
    pub agent: Option<String>,
    pub relay_id: Option<String>,
    /// 按用量统计的模型名（[`RequestLogEntry::usage_model`]）筛，忽略大小写。
    pub model: Option<String>,
}

impl RequestLogFilter {
    pub fn matches(&self, entry: &RequestLogEntry) -> bool {
        let started = if entry.started_at_ms > 0 {
            entry.started_at_ms
        } else {
            entry.timestamp_ms
        };
        self.since_ms.is_none_or(|since| started >= since)
            && self.agent.as_deref().is_none_or(|agent| entry.agent_id() == agent)
            && self.relay_id.as_deref().is_none_or(|relay| entry.relay_id == relay)
            && self
                .model
                .as_deref()
                .is_none_or(|model| entry.usage_model().eq_ignore_ascii_case(model.trim()))
    }
}

/// 按条件读出的一页请求日志。
#[derive(Clone, Debug, Default)]
pub struct RequestLogPage {
    /// 按开始时间倒序，最多 `limit` 条。
    pub entries: Vec<RequestLogEntry>,
    /// 符合条件的不止这些（更早的没返回）。
    pub more: bool,
    /// 日志文件里最早一条的开始时间：日志按大小滚动，更早的请求只留在用量统计里。
    pub oldest_ms: Option<u64>,
}

/// 读取符合条件的最近 `limit` 条请求日志（含正在进行的请求）。
pub fn read_request_log_page(limit: usize, filter: &RequestLogFilter) -> std::io::Result<RequestLogPage> {
    let mut page = read_request_log_page_at(&request_log_path(), limit, filter)?;
    page.entries
        .extend(running_requests().into_iter().filter(|entry| filter.matches(entry)));
    page.entries.sort_by_key(|entry| std::cmp::Reverse(entry.started_at_ms));
    if page.entries.len() > limit {
        page.entries.truncate(limit);
        page.more = true;
    }
    Ok(page)
}

pub fn read_request_log_page_at(
    path: &Path,
    limit: usize,
    filter: &RequestLogFilter,
) -> std::io::Result<RequestLogPage> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(RequestLogPage::default()),
        Err(error) => return Err(error),
    };
    let mut page = RequestLogPage::default();
    let mut entries = VecDeque::with_capacity(limit.min(1024));
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<RequestLogEntry>(line) else {
            continue;
        };
        if page.oldest_ms.is_none() {
            page.oldest_ms = Some(entry.started_at_ms.max(1));
        }
        if !filter.matches(&entry) {
            continue;
        }
        entries.push_back(entry);
        if entries.len() > limit {
            entries.pop_front();
            page.more = true;
        }
    }
    page.entries = entries.into_iter().rev().collect();
    Ok(page)
}

pub fn read_request_logs_at(path: &Path, limit: usize) -> std::io::Result<Vec<RequestLogEntry>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut entries = VecDeque::with_capacity(limit.min(1024));
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<RequestLogEntry>(line) {
            entries.push_back(entry);
            if entries.len() > limit {
                entries.pop_front();
            }
        }
    }
    Ok(entries.into_iter().rev().collect())
}

pub fn clear_request_logs() -> std::io::Result<()> {
    clear_request_logs_at(&request_log_path())
}

pub fn clear_request_logs_at(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// 从非流式 Responses JSON（或 `response.completed` 的 data）里提取 usage。
pub fn usage_from_response_json(value: &Value) -> Option<RequestUsage> {
    let usage = value
        .get("response")
        .and_then(|response| response.get("usage"))
        .or_else(|| value.get("usage"))?;
    usage_from_responses_usage(usage)
}

/// Responses / Anthropic 形状的 usage 对象（input_tokens / output_tokens）。
fn usage_from_responses_usage(usage: &Value) -> Option<RequestUsage> {
    let input_tokens = usage.get("input_tokens").and_then(Value::as_u64)?;
    let output_tokens = usage.get("output_tokens").and_then(Value::as_u64)?;
    let cached_tokens = usage
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_u64))
        .unwrap_or(0);
    let reasoning_tokens = usage
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(RequestUsage {
        input_tokens,
        output_tokens,
        cached_tokens,
        reasoning_tokens,
    })
}

/// 从任意响应 JSON 里尽力提取 usage（含 chat completions 的 prompt/completion 字段）。
pub fn usage_from_body_bytes(body: &[u8]) -> Option<RequestUsage> {
    let value: Value = serde_json::from_slice(body).ok()?;
    if let Some(usage) = usage_from_response_json(&value) {
        return Some(usage);
    }
    usage_from_usage_value(value.get("usage")?)
}

/// 从单个 usage 对象里提取用量：Responses / Anthropic 的 input/output 字段与
/// chat completions 的 prompt/completion 字段都认。流式转换器收尾时用它记账（#2803）。
pub fn usage_from_usage_value(usage: &Value) -> Option<RequestUsage> {
    if let Some(parsed) = usage_from_responses_usage(usage) {
        return Some(parsed);
    }
    let input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)?;
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)?;
    let cached_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens").and_then(Value::as_u64))
        .unwrap_or(0);
    let reasoning_tokens = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(RequestUsage {
        input_tokens,
        output_tokens,
        cached_tokens,
        reasoning_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str) -> RequestLogEntry {
        RequestLogEntry {
            id: id.to_string(),
            timestamp_ms: 0,
            started_at_ms: 0,
            duration_ms: 0,
            method: "POST".to_string(),
            path: "/v1/responses".to_string(),
            status_code: 200,
            success: true,
            stream: true,
            model: "gpt-6.1-sol".to_string(),
            thread_id: Some("thread-1".to_string()),
            upstream_model: "gpt-6.1-sol".to_string(),
            relay_id: "relay-a".to_string(),
            relay_name: "供应商 4".to_string(),
            wire_api: "Responses".to_string(),
            attempts: 2,
            auto_retries: 1,
            retry_reasons: vec!["upstream_stream_failed:server_is_overloaded".to_string()],
            attempt_details: Vec::new(),
            state: None,
            first_token_ms: Some(120),
            estimated_output_tokens: None,
            phase: None,
            error: None,
            stream_outcome: None,
            usage: Some(RequestUsage {
                input_tokens: 1000,
                output_tokens: 200,
                cached_tokens: 900,
                reasoning_tokens: 50,
            }),
            cache_hit_ratio: None,
            agent: None,
        }
    }

    #[test]
    fn page_filters_by_time_agent_provider_and_model() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("request-log.jsonl");
        let mut rows = Vec::new();
        for (index, (agent, relay, model)) in [
            (None, "relay-a", "gpt-6.1-sol"),
            (Some("claude"), "relay-a", "kimi-k2"),
            (Some("claude"), "relay-b", "Kimi-K2"),
            (Some("grok"), "relay-b", "glm-4.6"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut row = entry(&format!("e{index}"));
            row.started_at_ms = 1_000 * (index as u64 + 1);
            row.agent = agent.map(str::to_string);
            row.relay_id = relay.to_string();
            row.upstream_model = model.to_string();
            rows.push(row);
        }
        for row in &rows {
            append_request_log_at(&path, row).unwrap();
        }
        let ids = |page: &RequestLogPage| page.entries.iter().map(|entry| entry.id.clone()).collect::<Vec<_>>();

        let all = read_request_log_page_at(&path, 10, &RequestLogFilter::default()).unwrap();
        assert_eq!(ids(&all), vec!["e3", "e2", "e1", "e0"]);
        assert!(!all.more);
        assert_eq!(all.oldest_ms, Some(1_000));

        let codex = RequestLogFilter { agent: Some("codex".to_string()), ..Default::default() };
        assert_eq!(ids(&read_request_log_page_at(&path, 10, &codex).unwrap()), vec!["e0"], "没写 Agent 的算 Codex");

        let kimi = RequestLogFilter { model: Some("kimi-k2".to_string()), ..Default::default() };
        assert_eq!(ids(&read_request_log_page_at(&path, 10, &kimi).unwrap()), vec!["e2", "e1"], "模型名忽略大小写");

        let relay = RequestLogFilter { relay_id: Some("relay-b".to_string()), since_ms: Some(3_500), ..Default::default() };
        assert_eq!(ids(&read_request_log_page_at(&path, 10, &relay).unwrap()), vec!["e3"]);

        let limited = read_request_log_page_at(&path, 2, &RequestLogFilter::default()).unwrap();
        assert_eq!(ids(&limited), vec!["e3", "e2"]);
        assert!(limited.more, "符合条件的不止这些");
        assert_eq!(limited.oldest_ms, Some(1_000), "最早一条按整个文件算");
    }

    #[test]
    fn attempt_details_round_trip_and_legacy_entries_default_empty() {
        let mut retried = entry("retried");
        retried.attempt_details.push(RequestAttemptDetail {
            attempt: 1,
            code: Some("server_is_overloaded".to_string()),
            message: Some("Our servers are currently overloaded.".to_string()),
            delay_ms: Some(250),
        });
        let raw = serde_json::to_value(&retried).unwrap();
        assert_eq!(raw["attemptDetails"][0]["attempt"], json!(1));
        assert_eq!(
            raw["attemptDetails"][0]["code"],
            json!("server_is_overloaded")
        );
        assert_eq!(raw["attemptDetails"][0]["delayMs"], json!(250));
        let parsed: RequestLogEntry = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.attempt_details, retried.attempt_details);

        // 旧日志没有 attemptDetails：读入后为空，且空明细不写盘。
        let mut legacy = serde_json::to_value(entry("legacy")).unwrap();
        legacy.as_object_mut().unwrap().remove("attemptDetails");
        let parsed: RequestLogEntry = serde_json::from_value(legacy).unwrap();
        assert!(parsed.attempt_details.is_empty());
        let empty = serde_json::to_value(entry("empty")).unwrap();
        assert!(empty.get("attemptDetails").is_none());
    }

    #[test]
    fn appends_reads_and_clears_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REQUEST_LOG_FILE);
        let mut first = entry("first");
        first.finish(120);
        assert_eq!(first.duration_ms, 120);
        assert_eq!(first.cache_hit_ratio, Some(0.9));
        append_request_log_at(&path, &first).unwrap();
        append_request_log_at(&path, &entry("second")).unwrap();

        let logs = read_request_logs_at(&path, 10).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].id, "second", "按时间倒序");
        assert_eq!(logs[1].id, "first");
        assert_eq!(logs[1].duration_ms, 120);
        assert_eq!(read_request_logs_at(&path, 1).unwrap().len(), 1);

        clear_request_logs_at(&path).unwrap();
        assert!(read_request_logs_at(&path, 10).unwrap().is_empty());
        assert!(clear_request_logs_at(&path).is_ok());
    }

    #[test]
    fn live_requests_are_visible_until_finished() {
        let _guard = request_log_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(REQUEST_LOG_FILE);
        set_request_log_path_for_tests(Some(path.clone()));

        let mut running = entry("live-1");
        running.state = Some("running".to_string());
        running.success = false;
        start_request(&running);
        update_running_output_estimate("live-1", 42);
        update_running_retry("live-1", 2, 1, vec!["upstream_buffered_overload".to_string()]);
        let visible = read_request_logs(10).unwrap();
        assert!(visible.iter().any(|item| item.id == "live-1" && item.state.as_deref() == Some("running")));
        assert!(visible.iter().any(|item| item.estimated_output_tokens == Some(42)));
        assert!(visible.iter().any(|item| item.attempts == 2 && item.auto_retries == 1));

        let mut finished = running.clone();
        finished.success = true;
        finished.finish(200);
        finish_request(&finished);
        assert!(running_requests().is_empty());
        let entries = read_request_logs(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].state.as_deref(), Some("success"));

        set_request_log_path_for_tests(None);
    }

    #[test]
    fn oversized_log_keeps_only_recent_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REQUEST_LOG_FILE);
        let padding = "x".repeat(64 * 1024);
        for index in 0..40 {
            let mut item = entry(&format!("entry-{index}"));
            item.relay_name = format!("{padding}-{index}");
            append_request_log_at(&path, &item).unwrap();
        }
        assert!(std::fs::metadata(&path).unwrap().len() <= MAX_LOG_BYTES);
        let logs = read_request_logs_at(&path, 1000).unwrap();
        assert!(!logs.is_empty());
        assert_eq!(logs[0].id, "entry-39");
        assert!(logs.len() < 40, "裁剪后不应保留全部记录: {}", logs.len());
    }

    #[test]
    fn usage_is_extracted_from_completed_response() {
        let completed = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 111,
                    "output_tokens": 22,
                    "input_tokens_details": {"cached_tokens": 100},
                    "output_tokens_details": {"reasoning_tokens": 7}
                }
            }
        });
        let usage = usage_from_response_json(&completed).unwrap();
        assert_eq!(usage.input_tokens, 111);
        assert_eq!(usage.output_tokens, 22);
        assert_eq!(usage.cached_tokens, 100);
        assert_eq!(usage.reasoning_tokens, 7);
        assert_eq!(usage.cache_hit_ratio(), Some(100.0 / 111.0));

        let chat = br#"{"usage":{"prompt_tokens":10,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":4}}}"#;
        let usage = usage_from_body_bytes(chat).unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.cached_tokens, 4);
        assert!(usage_from_body_bytes(b"not-json").is_none());
    }
}
