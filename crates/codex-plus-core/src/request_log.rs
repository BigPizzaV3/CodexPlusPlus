//! 请求日志：按请求记录代理转发的状态、模型、重试、token 用量与缓存命中。
//!
//! 存储为 `<state dir>/request-log.jsonl`，只保留最近的若干条（追加时按大小裁剪），
//! 管理器「请求日志」页通过 Tauri 命令读取。日志里不保存提示词/回复正文，
//! 只保存模型名、token 计数和错误摘要，避免把会话内容写进明文日志。

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[path = "request_log_context.rs"]
mod context;
pub use context::RequestContextSummary;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const REQUEST_LOG_FILE: &str = "request-log.jsonl";
pub const FINAL_RESPONSE_EVIDENCE_V1: &str = "final-response-v1";
pub const FINAL_RESPONSE_SUPPLIER_EVIDENCE_V1: &str = "final-response-supplier-v1";
/// 日志文件达到该大小后裁剪到约一半，避免无限增长。
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const KEEP_LOG_BYTES: u64 = 1024 * 1024;

static ENTRY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static LOG_PATH_OVERRIDE: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();
static REQUEST_LOG_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 同进程读取器只在复制日志字节时持锁，解析应在释放锁后进行。
#[doc(hidden)]
pub fn lock_request_log() -> std::io::Result<std::sync::MutexGuard<'static, ()>> {
    REQUEST_LOG_WRITE_LOCK
        .lock()
        .map_err(|_| std::io::Error::other("request log lock poisoned"))
}

/// 一次上游请求的 token 用量。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cached_tokens: u64,
    /// 缓存数是否由上游显式提供；旧日志和缺失字段不能当作精确零值。
    #[serde(default)]
    pub cached_tokens_known: bool,
    #[serde(default)]
    pub reasoning_tokens: u64,
}

impl RequestUsage {
    /// 缓存命中率（cached / input），没有输入 token 时为 None。
    pub fn cache_hit_ratio(&self) -> Option<f64> {
        (self.cached_tokens_known && self.input_tokens > 0)
            .then(|| self.cached_tokens as f64 / self.input_tokens as f64)
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
    /// 仅由当前内置发送链绑定；反序列化的旧 trace 不能取得此资格。
    #[serde(skip)]
    pub upstream_model_bound: bool,
    pub wire_api: String,
    /// 上游请求总次数（含失败切换与自动重试）。
    pub attempts: u32,
    /// 代理自动重试次数（不含成员间失败切换）。
    pub auto_retries: u32,
    /// 上游第一段输出到达的 unix 毫秒时间（用于计算首字耗时）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_output_at_ms: Option<u64>,
    /// 成功那次上游请求真正发出的 unix 毫秒时间：首字耗时里转发前的排队/重试可据此单独统计（#3267）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_started_at_ms: Option<u64>,
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
    /// 成功那次上游请求真正发出的 unix 毫秒时间：首字耗时 - (它 - started_at_ms) 即上游真实首字（#3267）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_started_at_ms: Option<u64>,
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
    pub context_summary: Option<RequestContextSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<crate::gateway::activity::RouteRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_ratio: Option<f64>,
    /// `final-response-v1`：内置 Responses 或 Chat 转换链路已写出完整成功响应，
    /// input/output/cached 均显式存在，路由与模型绑定最后一次 middleware 后发送。
    /// `final-response-supplier-v1`：同样已送达且响应 ID、路由与模型明确，但缓存计数未知。
    /// 未验证的响应改写、压缩链路不发此标记；不证明失败尝试的计费。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_response_evidence: Option<String>,
    /// 已校验并送达客户端的最终响应 ID，不是本地请求日志的 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_response_id: Option<String>,
    /// 网关模式下发起请求的 Agent（claude / grok）；Codex 直连代理的旧记录为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

impl RequestTrace {
    pub(crate) fn bind_upstream_model(&mut self, body: &Value) {
        self.upstream_model = body.get("model").and_then(Value::as_str).unwrap_or("").to_string();
        self.upstream_model_bound = !self.upstream_model.trim().is_empty();
    }
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

pub(crate) fn record_running_route(id: &str, route: crate::gateway::activity::RouteRecord) {
    if let Ok(mut live) = live_requests().lock()
        && let Some(entry) = live.get_mut(id)
    {
        entry.route = Some(route);
    }
}

pub(crate) fn update_running_context(id: &str, body: &Value) {
    let summary = RequestContextSummary::from_responses(body);
    if let Ok(mut live) = live_requests().lock()
        && let Some(entry) = live.get_mut(id)
    {
        entry.context_summary = summary;
    }
}

/// 请求结束：从进行中列表移除并写入 JSONL。
pub fn finish_request(entry: &RequestLogEntry) {
    let recorded = live_requests().lock().ok().and_then(|mut live| live.remove(&entry.id));
    let logged = recorded.and_then(|recorded| {
        if recorded.context_summary.is_none() && recorded.route.is_none() { return None; }
        let mut logged = entry.clone();
        logged.context_summary = recorded.context_summary;
        logged.route = recorded.route;
        Some(logged)
    });
    if let Err(error) = append_request_log(logged.as_ref().unwrap_or(entry))
        && let Err(diagnostic_error) = crate::diagnostic_log::append_diagnostic_log(
            "request_log.write_or_trim_failed",
            serde_json::json!({ "error": error.to_string() }),
        )
    {
        eprintln!(
            "request log write or trim failed: {error}; diagnostic log failed: {diagnostic_error}"
        );
    }
    // 单元测试里默认路径指向真实用户状态目录，同样不写。
    if !(cfg!(test) && log_path_override().is_none()) {
        crate::gateway::usage::record_entry(entry);
    }
}

/// 更新进行中请求的快照（例如上游地址/首字耗时在请求结束前就已知）。
pub fn update_running(entry: &RequestLogEntry) {
    if let Ok(mut live) = live_requests().lock()
        && let Some(current) = live.get_mut(&entry.id)
    {
        let mut updated = entry.clone();
        // 预读/转发会直接发布进度，调用方持有的快照可能尚未同步，不能将其清掉。
        updated.first_token_ms = current.first_token_ms.or(updated.first_token_ms);
        updated.estimated_output_tokens = current
            .estimated_output_tokens
            .or(updated.estimated_output_tokens);
        if current.first_token_ms.is_some() {
            updated.phase = current.phase.clone();
        }
        updated.route = current.route.take().or(updated.route);
        updated.context_summary = current.context_summary.take().or(updated.context_summary);
        *current = updated;
    }
}

/// 选定真实上游后只更新路由；不触碰预读/转发已发布的首字、阶段和输出估算。
pub(crate) fn update_running_route(id: &str, trace: &RequestTrace, stream: bool) {
    if let Ok(mut live) = live_requests().lock()
        && let Some(entry) = live.get_mut(id)
    {
        entry.relay_id = trace.relay_id.clone();
        entry.relay_name = trace.relay_name.clone();
        entry.upstream_model = trace.upstream_model.clone();
        entry.wire_api = trace.wire_api.clone();
        entry.stream = stream;
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
    use std::io::{Read, Seek, SeekFrom, Write};

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct PhysicalEntry<'a> {
        #[serde(flatten)]
        entry: &'a RequestLogEntry,
        appended_at_ms: u64,
    }
    let _guard = lock_request_log()?;
    // 完成时间取在锁外；只有这里的时间才能界定被轮转的物理前缀。
    let mut line = serde_json::to_vec(&PhysicalEntry {
        entry,
        appended_at_ms: crate::launcher::current_time_ms(),
    })
    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    line.push(b'\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    // 上次写入若在换行前中断，不让残行吞掉这次完整的请求。
    if file.metadata()?.len() != 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut last = [0];
        file.read_exact(&mut last)?;
        if last != [b'\n'] {
            file.write_all(b"\n")?;
        }
    }
    file.write_all(&line)?;
    let needs_trim = file.metadata()?.len() > MAX_LOG_BYTES;
    drop(file);
    if needs_trim {
        trim_request_log_to_tail(path, KEEP_LOG_BYTES)?;
    }
    Ok(())
}

fn trim_request_log_to_tail(path: &Path, keep_bytes: u64) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path)?;
    let offset = file.metadata()?.len().saturating_sub(keep_bytes);
    file.seek(SeekFrom::Start(offset))?;
    let mut tail = Vec::new();
    file.take(keep_bytes).read_to_end(&mut tail)?;
    let start = if offset > 0 {
        tail.iter()
            .position(|byte| *byte == b'\n')
            .map_or(tail.len(), |position| position + 1)
    } else {
        0
    };
    std::fs::write(path, &tail[start..])
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
    /// 只要没有供应商归属（`relay_id` 为空）的请求：对应用量统计「按供应商」里的「未知」行，和不筛供应商是两回事。
    pub relay_unknown: bool,
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
            && (!self.relay_unknown || entry.relay_id.trim().is_empty())
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
    /// 文件里最后一条记录的位置，下一轮拿它增量读；空文件时没有。
    pub cursor: Option<RequestLogCursor>,
}

/// 增量读请求日志的游标：文件里最后读到的那一行的起始字节和它的请求 id。
/// 下次从这一行读起，先核对还是不是同一条记录——日志被清空或按大小裁剪后对不上，就整页重读。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogCursor {
    pub offset: u64,
    pub id: String,
}

/// 游标之后新写入的请求日志（已按条件筛过）。
#[derive(Clone, Debug, Default)]
pub struct RequestLogDelta {
    /// 新完成的请求 + 现在仍在进行的请求，按开始时间倒序。
    pub entries: Vec<RequestLogEntry>,
    /// 新的游标；没有新记录时和传入的相同。
    pub cursor: RequestLogCursor,
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
    let contents = match {
        let _guard = lock_request_log()?;
        std::fs::read(path)
    } {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(RequestLogPage::default()),
        Err(error) => return Err(error),
    };
    let mut page = RequestLogPage::default();
    let mut entries = VecDeque::with_capacity(limit.min(1024));
    let mut offset = 0u64;
    for raw in contents.split_inclusive(|byte| *byte == b'\n') {
        let start = offset;
        offset += raw.len() as u64;
        let line = raw.trim_ascii();
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_slice::<RequestLogEntry>(line) else {
            continue;
        };
        if page.oldest_ms.is_none() {
            page.oldest_ms = Some(entry.started_at_ms.max(1));
        }
        page.cursor = Some(RequestLogCursor { offset: start, id: entry.id.clone() });
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

/// 只读游标之后新写入的请求日志（含正在进行的请求），给请求日志页的自动刷新用：
/// 不再每轮把整个日志读一遍、解析一遍。游标对不上时返回 `None`，调用方改为整页重读。
pub fn read_request_logs_after(
    filter: &RequestLogFilter,
    cursor: &RequestLogCursor,
) -> std::io::Result<Option<RequestLogDelta>> {
    let Some(mut delta) = read_request_logs_after_at(&request_log_path(), filter, cursor)? else {
        return Ok(None);
    };
    delta
        .entries
        .extend(running_requests().into_iter().filter(|entry| filter.matches(entry)));
    delta.entries.sort_by_key(|entry| std::cmp::Reverse(entry.started_at_ms));
    Ok(Some(delta))
}

pub fn read_request_logs_after_at(
    path: &Path,
    filter: &RequestLogFilter,
    cursor: &RequestLogCursor,
) -> std::io::Result<Option<RequestLogDelta>> {
    use std::io::{Read, Seek, SeekFrom};
    let guard = lock_request_log()?;
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() <= cursor.offset {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(cursor.offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    drop(guard);
    let mut chunks = bytes.split_inclusive(|byte| *byte == b'\n');
    // 游标那一行必须还是同一条记录，否则日志被清空或裁剪过，偏移已经不作数。
    let first = chunks.next().unwrap_or_default();
    let first_offset = first.len() as u64;
    match serde_json::from_slice::<RequestLogEntry>(first.trim_ascii()) {
        Ok(entry) if entry.id == cursor.id => {}
        _ => return Ok(None),
    }
    let mut delta = RequestLogDelta { entries: Vec::new(), cursor: cursor.clone() };
    let mut offset = cursor.offset + first_offset;
    for chunk in chunks {
        let start = offset;
        offset += chunk.len() as u64;
        // 写入方整行加锁追加，这里仍只认带换行的完整行，半行留到下一轮。
        if !chunk.ends_with(b"\n") {
            break;
        }
        let line = chunk.trim_ascii();
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_slice::<RequestLogEntry>(line) else {
            continue;
        };
        delta.cursor = RequestLogCursor { offset: start, id: entry.id.clone() };
        if filter.matches(&entry) {
            delta.entries.push(entry);
        }
    }
    delta.entries.reverse();
    Ok(Some(delta))
}

pub fn read_request_logs_at(path: &Path, limit: usize) -> std::io::Result<Vec<RequestLogEntry>> {
    let contents = match {
        let _guard = lock_request_log()?;
        std::fs::read(path)
    } {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut entries = VecDeque::with_capacity(limit.min(1024));
    for line in contents.split(|byte| *byte == b'\n') {
        if let Ok(entry) = serde_json::from_slice::<RequestLogEntry>(line.trim_ascii()) {
            entries.push_back(entry);
            if entries.len() > limit {
                entries.pop_front();
            }
        }
    }
    Ok(entries.into_iter().rev().collect())
}

/// 实时请求流只读日志末尾这么多字节，够放几百条记录。
const RECENT_TAIL_BYTES: u64 = 256 * 1024;

/// 读取最近 `limit` 条请求日志（含正在进行的请求），按开始时间倒序返回。
/// 给托盘、总览这种每秒轮询的实时请求流用：只读文件末尾，不每次解析整个日志。
pub fn read_recent_request_logs(limit: usize) -> std::io::Result<Vec<RequestLogEntry>> {
    let mut entries = read_recent_request_logs_at(&request_log_path(), limit)?;
    entries.extend(running_requests());
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.started_at_ms));
    entries.truncate(limit);
    Ok(entries)
}

/// 只读日志文件末尾一段；末尾凑不够 `limit` 条时退回整文件读取。
pub fn read_recent_request_logs_at(path: &Path, limit: usize) -> std::io::Result<Vec<RequestLogEntry>> {
    use std::io::{Read, Seek, SeekFrom};
    let guard = lock_request_log()?;
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let start = file.metadata()?.len().saturating_sub(RECENT_TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    drop(guard);
    let mut lines = bytes.split(|byte| *byte == b'\n');
    // 从文件中间开始读时，第一行多半被截断，丢掉。
    if start > 0 {
        lines.next();
    }
    let mut entries = VecDeque::with_capacity(limit.min(1024));
    for line in lines {
        if let Ok(entry) = serde_json::from_slice::<RequestLogEntry>(line.trim_ascii()) {
            entries.push_back(entry);
            if entries.len() > limit {
                entries.pop_front();
            }
        }
    }
    if start > 0 && entries.len() < limit {
        return read_request_logs_at(path, limit);
    }
    Ok(entries.into_iter().rev().collect())
}

pub fn clear_request_logs() -> std::io::Result<()> {
    clear_request_logs_at(&request_log_path())
}

pub fn clear_request_logs_at(path: &Path) -> std::io::Result<()> {
    let _guard = lock_request_log()?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// 强证据不使用旧解析器的 cached=0 回退，也不把 HTTP 200 当作完成。
#[derive(Clone, Debug)]
pub(crate) struct FinalResponseUsage {
    pub usage: RequestUsage,
    pub response_id: Option<String>,
}

pub(crate) fn final_response_usage(value: &Value) -> Option<FinalResponseUsage> {
    let response = value.get("response").unwrap_or(value);
    if response.get("status").and_then(Value::as_str) != Some("completed")
        || response.get("error").is_some_and(|error| !error.is_null())
        || response.get("incomplete_details").is_some_and(|details| !details.is_null())
    {
        return None;
    }
    let usage = response.get("usage")?;
    let parsed = usage_from_responses_usage(usage)?;
    let response_id = response.get("id").and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_graphic()))
        .map(str::to_owned);
    (parsed.cached_tokens <= parsed.input_tokens
        && parsed.reasoning_tokens <= parsed.output_tokens
        && (parsed.cached_tokens_known || response_id.is_some()))
        .then_some(FinalResponseUsage { usage: parsed, response_id })
}

/// Chat 转换的输入不含缓存；先对齐实际送达用量，日志仍保留上游含缓存口径。
pub(crate) fn converted_final_response_usage(
    response: &Value,
    usage: Option<RequestUsage>,
) -> Option<FinalResponseUsage> {
    let usage = usage?;
    if response["status"] != "completed"
        || response.get("error").is_some_and(|value| !value.is_null())
        || response.get("incomplete_details").is_some_and(|value| !value.is_null())
    {
        return None;
    }
    let delivered = usage_from_response_json(response)?;
    if delivered.input_tokens.checked_add(usage.cached_tokens) != Some(usage.input_tokens)
        || delivered.cached_tokens != usage.cached_tokens
        || delivered.output_tokens != usage.output_tokens
        || delivered.reasoning_tokens != usage.reasoning_tokens
        || usage.reasoning_tokens > usage.output_tokens
    {
        return None;
    }
    let response_id = response["id"].as_str()
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_graphic()))?;
    Some(FinalResponseUsage { usage, response_id: Some(response_id.to_owned()) })
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
        .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_u64));
    let reasoning_tokens = usage
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(RequestUsage {
        input_tokens,
        output_tokens,
        cached_tokens: cached_tokens.unwrap_or(0),
        cached_tokens_known: cached_tokens.is_some(),
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
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens").and_then(Value::as_u64));
    let reasoning_tokens = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(RequestUsage {
        input_tokens,
        output_tokens,
        cached_tokens: cached_tokens.unwrap_or(0),
        cached_tokens_known: cached_tokens.is_some(),
        reasoning_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn context_summary_stays_with_the_exact_live_request() {
        let request = entry("context-summary-exact");
        let other = entry("context-summary-concurrent");
        start_request(&request);
        start_request(&other);
        update_running_context(&request.id, &serde_json::json!({"input": "never persist this text"}));
        update_running(&request);
        let mut live = live_requests().lock().unwrap();
        let saved = live.remove(&request.id).unwrap();
        let other = live.remove(&other.id).unwrap();
        assert!(saved.context_summary.is_some());
        assert!(other.context_summary.is_none());
        assert!(!serde_json::to_string(&saved.context_summary).unwrap().contains("never persist"));
    }

    fn entry(id: &str) -> RequestLogEntry {
        RequestLogEntry {
            route: None,
            context_summary: None,
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
            upstream_started_at_ms: None,
            estimated_output_tokens: None,
            phase: None,
            error: None,
            stream_outcome: None,
            usage: Some(RequestUsage {
                input_tokens: 1000,
                output_tokens: 200,
                cached_tokens: 900,
                cached_tokens_known: true,
                reasoning_tokens: 50,
            }),
            cache_hit_ratio: None,
            final_response_evidence: None,
            final_response_id: None,
            agent: None,
        }
    }

    #[test]
    fn final_response_evidence_round_trip_and_legacy_absence() {
        let legacy = serde_json::to_value(entry("legacy")).unwrap();
        assert!(legacy.get("finalResponseEvidence").is_none());
        assert!(legacy.get("finalResponseId").is_none());
        let mut row: RequestLogEntry = serde_json::from_value(legacy).unwrap();
        assert!(row.final_response_evidence.is_none());
        assert!(row.final_response_id.is_none());
        for marker in [FINAL_RESPONSE_EVIDENCE_V1, FINAL_RESPONSE_SUPPLIER_EVIDENCE_V1] {
            row.final_response_evidence = Some(marker.to_string());
            row.final_response_id = Some("resp_delivered".to_string());
            let raw = serde_json::to_value(&row).unwrap();
            assert_eq!(raw["finalResponseEvidence"], marker);
            assert_eq!(raw["finalResponseId"], "resp_delivered");
            let parsed: RequestLogEntry = serde_json::from_value(raw).unwrap();
            assert_eq!(parsed.final_response_id.as_deref(), Some("resp_delivered"));
            assert_eq!(parsed.final_response_evidence.as_deref(), Some(marker));
        }
    }

    #[test]
    fn final_response_usage_requires_explicit_complete_counts_and_success() {
        let complete = json!({"status":"completed","usage":{
            "input_tokens":12,"output_tokens":3,"input_tokens_details":{"cached_tokens":0}
        }});
        assert_eq!(final_response_usage(&complete).unwrap().usage.cached_tokens, 0);
        let long = "response".repeat(96);
        for (id, expected) in [("resp_ok", Some("resp_ok")), ("", None), ("two words", None), (long.as_str(), Some(long.as_str()))] {
            let mut response = complete.clone();
            response["id"] = json!(id);
            assert_eq!(final_response_usage(&response).unwrap().response_id.as_deref(), expected);
        }
        for pointer in [
            "/status", "/usage/input_tokens", "/usage/output_tokens",
            "/usage/input_tokens_details/cached_tokens",
        ] {
            let mut missing = complete.clone();
            *missing.pointer_mut(pointer).unwrap() = Value::Null;
            assert!(final_response_usage(&missing).is_none(), "{pointer}");
        }
        for status in ["in_progress", "incomplete", "failed"] {
            let mut failed = complete.clone();
            failed["status"] = json!(status);
            assert!(final_response_usage(&failed).is_none());
        }
        for (pointer, invalid) in [
            ("/usage/input_tokens_details/cached_tokens", json!(13)),
            ("/usage/input_tokens", json!(-1)),
            ("/usage/output_tokens", json!("3")),
        ] {
            let mut failed = complete.clone();
            *failed.pointer_mut(pointer).unwrap() = invalid;
            assert!(final_response_usage(&failed).is_none());
        }
        let mut failed = complete;
        failed["error"] = json!({"code":"server_error"});
        assert!(final_response_usage(&failed).is_none());
    }

    #[test]
    fn final_response_supplier_receipt_preserves_unknown_cache() {
        let mut response = json!({"id":"resp_supplier","status":"completed",
            "usage":{"input_tokens":12,"output_tokens":3}});
        let terminal = final_response_usage(&response).unwrap();
        assert_eq!(terminal.response_id.as_deref(), Some("resp_supplier"));
        assert!(!terminal.usage.cached_tokens_known);
        assert_eq!(terminal.usage.cache_hit_ratio(), None);
        for id in ["", "two words"] {
            response["id"] = json!(id);
            assert!(final_response_usage(&response).is_none());
        }
        response["id"] = json!("resp_supplier");
        for status in ["in_progress", "incomplete", "failed"] {
            response["status"] = json!(status);
            assert!(final_response_usage(&response).is_none());
        }
    }

    #[test]
    fn request_usage_distinguishes_missing_and_explicit_zero_cache() {
        for (usage, known) in [
            (json!({"input_tokens": 10, "output_tokens": 2}), false),
            (json!({"input_tokens": 10, "output_tokens": 2,
                "input_tokens_details": {"cached_tokens": 0}}), true),
            (json!({"prompt_tokens": 10, "completion_tokens": 2}), false),
            (json!({"prompt_tokens": 10, "completion_tokens": 2,
                "prompt_tokens_details": {"cached_tokens": 0}}), true),
        ] {
            let parsed = usage_from_usage_value(&usage).unwrap();
            assert_eq!(parsed.cached_tokens, 0);
            assert_eq!(parsed.cached_tokens_known, known);
            assert_eq!(serde_json::to_value(&parsed).unwrap()["cachedTokensKnown"], known);
            assert_eq!(parsed.cache_hit_ratio(), known.then_some(0.0));
        }
        let legacy: RequestUsage = serde_json::from_value(json!({
            "inputTokens": 10, "outputTokens": 2, "cachedTokens": 0
        }))
        .unwrap();
        assert!(!legacy.cached_tokens_known);
    }

    #[test]
    fn upstream_model_binding_uses_mutated_body_and_is_not_deserialized() {
        let mut trace = RequestTrace::default();
        let mut body = json!({"model":"before-middleware"});
        body["model"] = json!("after-middleware");
        trace.bind_upstream_model(&body);
        assert_eq!(trace.upstream_model, "after-middleware");
        assert!(trace.upstream_model_bound);
        let mut raw = serde_json::to_value(&trace).unwrap();
        assert!(raw.get("upstreamModelBound").is_none());
        raw["upstreamModelBound"] = json!(true);
        let restored: RequestTrace = serde_json::from_value(raw).unwrap();
        assert!(!restored.upstream_model_bound);
        trace.bind_upstream_model(&json!({}));
        assert!(!trace.upstream_model_bound);
        assert!(trace.upstream_model.is_empty());
    }

    #[test]
    fn appending_records_physical_time_without_reordering_completion_times() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("request-log.jsonl");
        let before = crate::launcher::current_time_ms();
        for completed in [200, 100] {
            let mut row = entry(&completed.to_string());
            row.timestamp_ms = completed;
            append_request_log_at(&path, &row).unwrap();
        }
        let after = crate::launcher::current_time_ms();
        let rows: Vec<Value> = std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows[0]["timestampMs"], 200);
        assert_eq!(rows[1]["timestampMs"], 100);
        let first = rows[0]["appendedAtMs"].as_u64().unwrap();
        let last = rows[1]["appendedAtMs"].as_u64().unwrap();
        assert!(before <= first && first <= last && last <= after);
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
    fn delta_reads_only_lines_after_cursor_and_resets_when_log_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("request-log.jsonl");
        let ids = |entries: &[RequestLogEntry]| entries.iter().map(|entry| entry.id.clone()).collect::<Vec<_>>();
        let row = |id: &str, started: u64, relay: &str| {
            let mut row = entry(id);
            row.started_at_ms = started;
            row.relay_id = relay.to_string();
            row
        };
        append_request_log_at(&path, &row("e0", 1_000, "relay-a")).unwrap();
        append_request_log_at(&path, &row("e1", 2_000, "relay-b")).unwrap();
        let page = read_request_log_page_at(&path, 10, &RequestLogFilter::default()).unwrap();
        let cursor = page.cursor.clone().expect("非空日志要给游标");
        assert_eq!(cursor.id, "e1", "游标指向文件里最后一条");

        // 没有新记录：空结果、游标不变。
        let idle = read_request_logs_after_at(&path, &RequestLogFilter::default(), &cursor).unwrap().unwrap();
        assert!(idle.entries.is_empty());
        assert_eq!(idle.cursor, cursor);

        // 新写入的按条件筛；长请求开始得早、写得晚，也照样读到；游标跟到最后一行（不论是否符合条件）。
        append_request_log_at(&path, &row("e2", 3_000, "relay-a")).unwrap();
        append_request_log_at(&path, &row("e3", 500, "relay-a")).unwrap();
        append_request_log_at(&path, &row("e4", 4_000, "relay-b")).unwrap();
        let relay_a = RequestLogFilter { relay_id: Some("relay-a".to_string()), ..Default::default() };
        let delta = read_request_logs_after_at(&path, &relay_a, &cursor).unwrap().unwrap();
        assert_eq!(ids(&delta.entries), vec!["e3", "e2"], "新写入的倒序返回，供应商条件照常生效");
        assert_eq!(delta.cursor.id, "e4");
        let full = read_request_log_page_at(&path, 10, &RequestLogFilter::default()).unwrap();
        assert_eq!(full.cursor.as_ref(), Some(&delta.cursor), "增量游标和整页读的一致");

        // 日志被清空或改写过：游标对不上，要求整页重读。
        clear_request_logs_at(&path).unwrap();
        assert!(read_request_logs_after_at(&path, &RequestLogFilter::default(), &delta.cursor).unwrap().is_none());
        for index in 0..6 {
            append_request_log_at(&path, &row(&format!("n{index}"), 10_000 + index, "relay-a")).unwrap();
        }
        assert!(
            read_request_logs_after_at(&path, &RequestLogFilter::default(), &delta.cursor).unwrap().is_none(),
            "同一偏移上换成了别的记录"
        );
    }

    #[test]
    fn page_unknown_provider_filter_only_keeps_requests_without_relay() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("request-log.jsonl");
        for (index, relay) in ["relay-a", "", "relay-b", "  ", ""].into_iter().enumerate() {
            let mut row = entry(&format!("e{index}"));
            row.started_at_ms = 1_000 * (index as u64 + 1);
            row.relay_id = relay.to_string();
            row.relay_name = if relay.is_empty() { String::new() } else { format!("name-{relay}") };
            append_request_log_at(&path, &row).unwrap();
        }
        let ids = |filter: &RequestLogFilter| {
            read_request_log_page_at(&path, 10, filter)
                .unwrap()
                .entries
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>()
        };

        let unknown = RequestLogFilter { relay_unknown: true, ..Default::default() };
        assert_eq!(ids(&unknown), vec!["e4", "e3", "e1"], "「未知」只留没有供应商的请求");
        let ranged = RequestLogFilter { relay_unknown: true, since_ms: Some(3_500), ..Default::default() };
        assert_eq!(ids(&ranged), vec!["e4", "e3"], "时间段照常生效");
        assert_eq!(ids(&RequestLogFilter::default()).len(), 5, "不筛供应商仍是全部");
        let named = RequestLogFilter { relay_id: Some("relay-b".to_string()), ..Default::default() };
        assert_eq!(ids(&named), vec!["e2"], "普通供应商 id 筛选不变");
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
        retried.upstream_started_at_ms = Some(1_700_000_000_123);
        retried.started_at_ms = 1_700_000_000_123 - 9_000;
        let raw = serde_json::to_value(&retried).unwrap();
        assert_eq!(raw["upstreamStartedAtMs"], json!(1_700_000_000_123u64));
        let parsed: RequestLogEntry = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(parsed.upstream_started_at_ms, retried.upstream_started_at_ms);
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
    fn live_route_and_snapshot_updates_preserve_stream_progress() {
        let _guard = request_log_test_guard();
        let mut running = entry("live-route-progress");
        running.first_token_ms = None;
        running.phase = Some("requesting".to_string());
        start_request(&running);
        let mut trace = RequestTrace {
            relay_id: "selected-relay".to_string(),
            relay_name: "Selected relay".to_string(),
            upstream_model: "real-model".to_string(),
            wire_api: "responses".to_string(),
            ..Default::default()
        };
        update_running_route(&running.id, &trace, true);
        let selected = running_requests().into_iter().find(|row| row.id == running.id).unwrap();
        assert_eq!(selected.relay_id, trace.relay_id);
        assert_eq!(selected.relay_name, trace.relay_name);
        assert_eq!(selected.upstream_model, trace.upstream_model);
        assert_eq!(selected.wire_api, trace.wire_api);
        assert!(selected.stream);
        assert!(selected.first_token_ms.is_none());
        assert_eq!(selected.phase.as_deref(), Some("requesting"));

        update_running_first_token(&running.id);
        update_running_output_estimate(&running.id, 42);
        update_running_retry(&running.id, 2, 1, vec!["overload".to_string()]);
        trace.relay_id = "fallback-relay".to_string();
        update_running_route(&running.id, &trace, true);
        let generating = running_requests().into_iter().find(|row| row.id == running.id).unwrap();
        assert_eq!(generating.relay_id, "fallback-relay");
        assert_eq!(generating.phase.as_deref(), Some("generating"));
        assert_eq!(generating.estimated_output_tokens, Some(42));
        assert_eq!(generating.attempts, 2);
        assert_eq!(generating.auto_retries, 1);
        assert_eq!(generating.retry_reasons, vec!["overload"]);

        // record_upstream 的本地快照仍没有预读期间的生成进度。
        running.relay_id = trace.relay_id;
        update_running(&running);
        let updated = running_requests().into_iter().find(|row| row.id == running.id).unwrap();
        assert_eq!(updated.first_token_ms, generating.first_token_ms);
        assert!(updated.first_token_ms.is_some());
        assert_eq!(updated.phase.as_deref(), Some("generating"));
        assert_eq!(updated.estimated_output_tokens, Some(42));
        live_requests().lock().unwrap().remove(&running.id);
        update_running_route(&running.id, &RequestTrace::default(), false);
        assert!(!running_requests().iter().any(|row| row.id == running.id));
    }

    #[test]
    fn interrupted_utf8_row_does_not_hide_or_consume_completed_requests() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.jsonl");
        append_request_log_at(&path, &entry("before")).unwrap();
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"relayName\":\"\xe6\xb5").unwrap();
        drop(file);

        let first = read_request_log_page_at(&path, 10, &Default::default()).unwrap();
        assert_eq!(first.entries.len(), 1);
        let cursor = first.cursor.unwrap();
        assert_eq!(cursor.id, "before");
        append_request_log_at(&path, &entry("after")).unwrap();
        for rows in [
            read_request_logs_at(&path, 10).unwrap(),
            read_recent_request_logs_at(&path, 10).unwrap(),
            read_request_log_page_at(&path, 10, &Default::default()).unwrap().entries,
        ] {
            assert_eq!(
                rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
                ["after", "before"]
            );
        }
        let delta = read_request_logs_after_at(&path, &Default::default(), &cursor).unwrap().unwrap();
        assert_eq!(delta.entries.len(), 1);
        assert_eq!(delta.entries[0].id, "after");
    }

    #[test]
    fn recent_logs_read_only_the_tail_and_match_full_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REQUEST_LOG_FILE);
        assert!(read_recent_request_logs_at(&path, 5).unwrap().is_empty());

        // 每条约 8KB，60 条超过末尾窗口（256KB），走「从中间开始读」的分支。
        let padding = "x".repeat(8 * 1024);
        for index in 0..60 {
            let mut item = entry(&format!("tail-{index}"));
            item.relay_name = format!("{padding}-{index}");
            append_request_log_at(&path, &item).unwrap();
        }
        let recent = read_recent_request_logs_at(&path, 5).unwrap();
        let ids: Vec<&str> = recent.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["tail-59", "tail-58", "tail-57", "tail-56", "tail-55"]);

        // 末尾窗口凑不够时退回整文件读取，结果和完整读取一致。
        let many = read_recent_request_logs_at(&path, 50).unwrap();
        let full = read_request_logs_at(&path, 50).unwrap();
        assert_eq!(
            many.iter().map(|item| &item.id).collect::<Vec<_>>(),
            full.iter().map(|item| &item.id).collect::<Vec<_>>()
        );
        assert_eq!(many.len(), 50);
    }

    #[test]
    fn recent_logs_include_running_requests() {
        let _guard = request_log_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(REQUEST_LOG_FILE);
        set_request_log_path_for_tests(Some(path.clone()));

        let mut done = entry("flow-done");
        done.started_at_ms = 100;
        append_request_log_at(&path, &done).unwrap();
        let mut running = entry("flow-live");
        running.started_at_ms = 200;
        running.state = Some("running".to_string());
        start_request(&running);

        let recent = read_recent_request_logs(10).unwrap();
        assert_eq!(recent[0].id, "flow-live");
        assert_eq!(recent[1].id, "flow-done");

        finish_request(&running);
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
    fn concurrent_appends_survive_log_trimming() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REQUEST_LOG_FILE);
        let filler = format!("{}\n", serde_json::to_string(&entry("filler")).unwrap());
        let count = (MAX_LOG_BYTES as usize - 16 * 1024) / filler.len();
        std::fs::write(&path, filler.repeat(count)).unwrap();

        let workers = 12;
        let per_worker = 96;
        let start = std::sync::Arc::new(std::sync::Barrier::new(workers));
        let threads: Vec<_> = (0..workers)
            .map(|worker| {
                let path = path.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    for number in 0..per_worker {
                        append_request_log_at(&path, &entry(&format!("marker-{worker}-{number}")))
                            .unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }

        let contents = std::fs::read_to_string(&path).unwrap();
        let mut markers = std::collections::HashSet::new();
        for line in contents.lines() {
            let entry: RequestLogEntry = serde_json::from_str(line).unwrap();
            if entry.id.starts_with("marker-") {
                assert!(markers.insert(entry.id));
            }
        }
        assert_eq!(markers.len(), workers * per_worker);
    }

    #[test]
    fn readers_wait_for_in_process_trimming() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REQUEST_LOG_FILE);
        std::fs::write(&path, "").unwrap();
        let guard = lock_request_log().unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let read_path = path.clone();
        let thread = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            result_tx.send(read_request_logs_at(&read_path, 1)).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(matches!(
            result_rx.recv_timeout(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&entry("retained")).unwrap()),
        )
        .unwrap();
        drop(guard);
        let rows = result_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(rows[0].id, "retained");
        thread.join().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn trimming_preserves_custom_windows_acl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REQUEST_LOG_FILE);
        let filler = format!("{}\n", serde_json::to_string(&entry("filler")).unwrap());
        std::fs::write(
            &path,
            filler.repeat(MAX_LOG_BYTES as usize / filler.len() + 1),
        )
        .unwrap();
        let configured = std::process::Command::new("icacls")
            .arg(&path)
            .args(["/inheritance:r", "/grant:r", "*S-1-5-11:(F)"])
            .output()
            .unwrap();
        assert!(configured.status.success(), "failed to configure test ACL");
        let before = std::process::Command::new("icacls")
            .arg(&path)
            .output()
            .unwrap();
        assert!(before.status.success());

        append_request_log_at(&path, &entry("trigger")).unwrap();

        let after = std::process::Command::new("icacls")
            .arg(&path)
            .output()
            .unwrap();
        assert!(after.status.success());
        assert!(
            after.stdout == before.stdout,
            "trimming changed the custom ACL"
        );
    }

    #[cfg(unix)]
    #[test]
    fn trimming_preserves_existing_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REQUEST_LOG_FILE);
        let filler = format!("{}\n", serde_json::to_string(&entry("filler")).unwrap());
        std::fs::write(
            &path,
            filler.repeat(MAX_LOG_BYTES as usize / filler.len() + 1),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        append_request_log_at(&path, &entry("trigger")).unwrap();

        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn concurrent_appends_and_clears_leave_complete_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REQUEST_LOG_FILE);
        let workers = 8;
        let start = std::sync::Arc::new(std::sync::Barrier::new(workers + 1));
        let clear_finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let threads: Vec<_> = (0..workers)
            .map(|worker| {
                let path = path.clone();
                let start = start.clone();
                let clear_finished = clear_finished.clone();
                std::thread::spawn(move || {
                    start.wait();
                    for number in 0..96 {
                        append_request_log_at(&path, &entry(&format!("marker-{worker}-{number}")))
                            .unwrap();
                    }
                    while !clear_finished.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    append_request_log_at(&path, &entry(&format!("after-clear-{worker}"))).unwrap();
                })
            })
            .collect();
        start.wait();
        for _ in 0..64 {
            clear_request_logs_at(&path).unwrap();
            std::thread::yield_now();
        }
        clear_finished.store(true, Ordering::Release);
        for thread in threads {
            thread.join().unwrap();
        }

        let contents = std::fs::read_to_string(&path).unwrap();
        let entries: Vec<RequestLogEntry> = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        for worker in 0..workers {
            assert!(
                entries
                    .iter()
                    .any(|entry| entry.id == format!("after-clear-{worker}"))
            );
        }
    }

    #[test]
    fn append_failure_is_reported_without_stopping_usage() {
        const CHILD_DIR: &str = "CODEX_PLUS_REQUEST_LOG_FAILURE_TEST_DIR";
        const POISON_LOCK: &str = "CODEX_PLUS_REQUEST_LOG_POISON_LOCK";
        if let Some(root) = std::env::var_os(CHILD_DIR) {
            let root = PathBuf::from(root);
            let request_path = root.join("request-log-directory");
            let diagnostic_path = root.join("diagnostic.jsonl");
            if std::env::var_os(POISON_LOCK).is_some() {
                let poisoned = std::thread::spawn(|| {
                    let _guard = lock_request_log().unwrap();
                    panic!("poison test lock");
                });
                assert!(poisoned.join().is_err());
            } else {
                std::fs::create_dir(&request_path).unwrap();
            }
            set_request_log_path_for_tests(Some(request_path));
            crate::diagnostic_log::set_diagnostic_log_path_for_tests(Some(diagnostic_path.clone()));

            finish_request(&entry("failed-append"));

            let diagnostics = std::fs::read_to_string(diagnostic_path).unwrap();
            assert!(diagnostics.contains("\"event\":\"request_log.write_or_trim_failed\""));
            let usage_path = root.join("state").join("gateway-usage.json");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let usage = loop {
                if let Ok(contents) = std::fs::read(&usage_path) {
                    break serde_json::from_slice::<Value>(&contents).unwrap();
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "usage was not recorded"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            let rows = usage["days"]
                .as_object()
                .unwrap()
                .values()
                .flat_map(|day| day.as_array().unwrap().iter());
            assert!(rows.into_iter().any(|row| {
                row["providerId"] == "relay-a" && row["calls"] == 1 && row["inputTokens"] == 1000
            }));
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        for mode in ["unwritable", "poisoned"] {
            let root = dir.path().join(mode);
            std::fs::create_dir(&root).unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "request_log::tests::append_failure_is_reported_without_stopping_usage",
                    "--nocapture",
                ])
                .env(CHILD_DIR, &root)
                .env("CODEX_PLUS_STATE_DIR", root.join("state"))
                .env("CODEX_HOME", &root)
                .env("APPDATA", &root)
                .env("LOCALAPPDATA", &root)
                .env("TEMP", &root)
                .env("TMP", &root)
                .env_remove("CODEX_SQLITE_HOME")
                .env_remove(POISON_LOCK);
            if mode == "poisoned" {
                command.env(POISON_LOCK, "1");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{mode}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
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
