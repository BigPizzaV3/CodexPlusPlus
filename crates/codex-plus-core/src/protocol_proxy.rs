//! Codex Responses API 与 OpenAI Chat Completions 的本地协议转换。
//!
//! Codex Chat 与 Responses 协议之间的转换实现。

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use base64::Engine as _;
use serde_json::{Map, Value, json};

use crate::relay_rotation::{RotationContext, RotationEvent};
use crate::settings::{RelayProtocol, SettingsStore};

pub const DEFAULT_PROTOCOL_PROXY_PORT: u16 = 57321;

/// 工具调用上需要跨 turn 原样带回的供应商附带数据，按 `call_id` 索引。
///
/// Gemini 3 系在 functionCall 上要求回传 `thought_signature`（OpenAI 兼容层放在
/// tool_call 的 `extra_content.google.thought_signature`），缺失时整轮 400
/// （issue #332 / #1012）。这类字段由上游产生、本仓不理解其语义，因此不硬编码
/// 任何字段名，而是把整个 `extra_content` 原样记住，下一轮构造请求时挂回对应
/// tool_call。只做透传，解析失败/无该字段时行为与改动前完全一致。
///
/// 键是 call_id：上游保证同一次调用内唯一，跨 turn 也由客户端原样回传。
fn tool_call_extra_content_cache() -> &'static Mutex<ExtraContentCache> {
    static CACHE: OnceLock<Mutex<ExtraContentCache>> = OnceLock::new();
    // 缓存上限：一次会话里同时活跃的工具调用远小于此数；超出后按插入序淘汰最旧的，
    // 避免长时间运行时无界增长。
    CACHE.get_or_init(|| Mutex::new(ExtraContentCache::new(256)))
}

#[derive(Debug)]
struct ExtraContentCache {
    entries: BTreeMap<String, Value>,
    order: VecDeque<String>,
    capacity: usize,
}

impl ExtraContentCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    fn remember(&mut self, call_id: &str, extra_content: Value) {
        if call_id.is_empty() {
            return;
        }
        if self
            .entries
            .insert(call_id.to_string(), extra_content)
            .is_none()
        {
            self.order.push_back(call_id.to_string());
        }
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                // 只有仍是最旧那一条时才删，避免把刚更新过的键误删。
                if !self.order.contains(&oldest) {
                    self.entries.remove(&oldest);
                }
            }
        }
    }

    fn recall(&self, call_id: &str) -> Option<Value> {
        self.entries.get(call_id).cloned()
    }
}

/// 从 chat 侧的 tool_call 里取出需要透传的附带数据。
///
/// `extra_content` 是 OpenAI 兼容层的通用扩展位（Gemini 的 thought_signature 在此）；
/// 部分供应商也会用 `thought_signature` / `thoughtSignature` 直接挂在 tool_call 上，
/// 两种形态都收。
fn tool_call_extra_content(tool_call: &Value) -> Option<Value> {
    if let Some(value) = tool_call.get("extra_content") {
        if value.is_object() && !value.as_object().is_some_and(Map::is_empty) {
            return Some(value.clone());
        }
    }
    for key in ["thought_signature", "thoughtSignature"] {
        if let Some(value) = tool_call.get(key) {
            if !value.is_null() {
                return Some(json!({ "google": { "thought_signature": value } }));
            }
        }
    }
    None
}

/// 记住本次上游返回的工具调用附带数据（按 call_id）。
fn remember_tool_call_extra_content(call_id: &str, tool_call: &Value) {
    let Some(extra) = tool_call_extra_content(tool_call) else {
        return;
    };
    if let Ok(mut cache) = tool_call_extra_content_cache().lock() {
        cache.remember(call_id, extra);
    }
}

/// 取回该 call_id 之前记住的附带数据，用于下一轮请求回传。
fn recall_tool_call_extra_content(call_id: &str) -> Option<Value> {
    tool_call_extra_content_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.recall(call_id))
}

const PROTOCOL_PROXY_PORT_FILE: &str = "protocol-proxy-port";
const PROTOCOL_PROXY_PORT_HISTORY_FILE: &str = "protocol-proxy-port-history";

/// 协议代理的实际生效端口，默认 [`DEFAULT_PROTOCOL_PROXY_PORT`]，
/// 可用环境变量 `CODEX_PLUS_PROTOCOL_PROXY_PORT` 覆盖。
///
/// 端口要写进 `config.toml` 的 `base_url`，不能像普通 helper 端口那样自动换；
/// 但少数机器（issue #2189 / #1163）上 57321 恰好被 Hyper-V/WSL 开机划进了
/// Windows 动态端口排除区间，bind 报 os error 10013 永远起不来，只能整体挪
/// 一个端口。迁移后的端口持久化在状态目录里，管理器与启动器读同一份值。
/// 写入 base_url 与读取校验必须都走本函数，保证同一进程内一致。
pub fn protocol_proxy_port() -> u16 {
    if let Some(port) = env_protocol_proxy_port() {
        return port;
    }
    read_persisted_protocol_proxy_port(&persisted_protocol_proxy_port_path())
        .unwrap_or(DEFAULT_PROTOCOL_PROXY_PORT)
}

fn env_protocol_proxy_port() -> Option<u16> {
    std::env::var("CODEX_PLUS_PROTOCOL_PROXY_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .filter(|port| *port > 0)
}

fn persisted_protocol_proxy_port_path() -> std::path::PathBuf {
    crate::paths::default_app_state_dir().join(PROTOCOL_PROXY_PORT_FILE)
}

fn read_persisted_protocol_proxy_port(path: &std::path::Path) -> Option<u16> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
}

fn persist_protocol_proxy_port(path: &std::path::Path, port: u16) -> anyhow::Result<()> {
    // 在覆盖当前值前保存迁移链。只记录本工具实际使用过的端口，不能把任意
    // loopback URL 当成自己的代理，否则会覆盖用户配置的其它本地服务。
    let history_path = path.with_file_name(PROTOCOL_PROXY_PORT_HISTORY_FILE);
    let mut ports = read_protocol_proxy_port_history(&history_path);
    ports.extend(read_persisted_protocol_proxy_port(path));
    ports.insert(port);
    let history = ports.iter().map(u16::to_string).collect::<Vec<_>>().join("\n") + "\n";
    crate::settings::atomic_write(&history_path, history.as_bytes())
        .context("持久化协议代理历史端口失败")?;
    crate::settings::atomic_write(path, format!("{port}\n").as_bytes())
        .with_context(|| format!("持久化协议代理端口失败：{}", path.display()))
}

fn read_protocol_proxy_port_history(path: &std::path::Path) -> BTreeSet<u16> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().parse::<u16>().ok())
        .filter(|port| *port > 0)
        .collect()
}

pub(crate) fn managed_protocol_proxy_base_urls() -> Vec<String> {
    let path = persisted_protocol_proxy_port_path();
    let mut ports = read_protocol_proxy_port_history(&path.with_file_name(PROTOCOL_PROXY_PORT_HISTORY_FILE));
    ports.insert(DEFAULT_PROTOCOL_PROXY_PORT);
    ports.insert(protocol_proxy_port());
    // 即使显式环境覆盖了当前端口，之前持久化的端口也仍是本工具写入过的值。
    ports.extend(read_persisted_protocol_proxy_port(&path));
    ports.into_iter().map(local_responses_proxy_base_url).collect()
}

pub(crate) fn is_managed_protocol_proxy_base_url(value: &str) -> bool {
    let Ok(mut parsed) = reqwest::Url::parse(value.trim()) else {
        return false;
    };
    let path = parsed.path().trim_end_matches('/').to_string();
    parsed.set_path(&path);
    managed_protocol_proxy_base_urls().iter().any(|url| parsed.as_str() == url)
}

#[cfg(test)]
mod managed_port_history_tests {
    use super::*;

    #[test]
    fn repeated_port_migrations_keep_every_managed_port() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROTOCOL_PROXY_PORT_FILE);
        // 模拟旧版本只有当前端口文件；首次升级迁移也要保留该端口。
        std::fs::write(&path, "58077\n").unwrap();
        persist_protocol_proxy_port(&path, 58088).unwrap();
        persist_protocol_proxy_port(&path, 58099).unwrap();
        assert_eq!(read_persisted_protocol_proxy_port(&path), Some(58099));
        assert_eq!(
            read_protocol_proxy_port_history(&dir.path().join(PROTOCOL_PROXY_PORT_HISTORY_FILE)),
            BTreeSet::from([58077, 58088, 58099]),
        );
    }
}

/// 启动前确认协议代理端口可绑定。
///
/// 端口被**占用**（AddrInUse）时保持原值，交给既有的等待与重试逻辑——旧
/// launcher 释放端口通常只需要几秒。只有 `PermissionDenied`（Windows 10013，
/// 端口落在系统排除区间）才会永久失败，此时换一个空闲端口并持久化，让
/// `config.toml` 的 base_url、helper 绑定和路由校验都跟随同一个新端口。
///
/// 显式设置 `CODEX_PLUS_PROTOCOL_PROXY_PORT` 时尊重用户选择，不自动迁移。
pub fn ensure_protocol_proxy_port_available() -> u16 {
    let port = protocol_proxy_port();
    if env_protocol_proxy_port().is_some() {
        return port;
    }
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => {
            drop(listener);
            port
        }
        Err(error) if crate::ports::port_bind_forbidden(&error) => {
            let replacement = crate::ports::find_available_loopback_port();
            if replacement != 0 && replacement != port {
                let path = persisted_protocol_proxy_port_path();
                match persist_protocol_proxy_port(&path, replacement) {
                    Ok(()) => {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "launcher.protocol_proxy_port_relocated",
                            serde_json::json!({
                                "from": port,
                                "to": replacement,
                                "path": path.to_string_lossy(),
                                "error": error.to_string(),
                            }),
                        );
                        return replacement;
                    }
                    Err(persist_error) => {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "launcher.protocol_proxy_port_relocate_failed",
                            serde_json::json!({
                                "from": port,
                                "to": replacement,
                                "error": persist_error.to_string(),
                            }),
                        );
                    }
                }
            }
            port
        }
        Err(_) => port,
    }
}
pub const NO_AUTH_PROXY_BEARER_TOKEN: &str = "codex-plus-no-auth";
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const UPSTREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const UPSTREAM_STREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(120);
const UPSTREAM_IMAGE_HEADER_TIMEOUT: Duration = Duration::from_secs(600);
/// 连接/TLS 握手阶段的失败发生在请求送达上游之前，可以安全重试。
/// 系统代理抖动时（如 `client error (Connect): tls handshake eof`）直接返回 502
/// 会让采样/压缩无故失败，两次短退避能把大部分瞬时故障挡在本地。
const UPSTREAM_CONNECT_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_millis(150), Duration::from_millis(400)];

/// 容量错误自动重试：单次等待上限，避免把 Codex 的请求挂太久。
const PROXY_AUTO_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
/// 容量错误重试的总等待预算；用完后把最后一个上游错误原样交给 Codex。
const PROXY_AUTO_RETRY_TOTAL_BUDGET: Duration = Duration::from_secs(120);
/// 即使用户把间隔设成 0 秒，也给一个短地板值，避免容量错误被打成热循环。
const PROXY_AUTO_RETRY_MIN_DELAY: Duration = Duration::from_millis(250);

/// HTTP 错误重试时预读错误体的上限：只用来记请求日志，读不全就退回状态码。
const PROXY_AUTO_RETRY_ERROR_BODY_LIMIT: usize = 64 * 1024;

/// 自动重试的退避状态：200 流内的过载/容量失败，以及命中用户所选状态码的
/// HTTP 错误共用同一份次数与等待预算。不跨请求持久化。
struct ProxyAutoRetry {
    remaining: u32,
    next_delay: Duration,
    waited: Duration,
    status_codes: Vec<u16>,
}

impl ProxyAutoRetry {
    fn from_settings(settings: &crate::settings::BackendSettings) -> Option<Self> {
        if !settings.protocol_proxy_auto_retry {
            return None;
        }
        let attempts = settings.protocol_proxy_auto_retry_max_attempts;
        if attempts == 0 {
            return None;
        }
        let base = Duration::from_secs(settings.protocol_proxy_auto_retry_interval_seconds)
            .max(PROXY_AUTO_RETRY_MIN_DELAY);
        Some(Self {
            remaining: attempts,
            next_delay: base,
            waited: Duration::ZERO,
            status_codes: settings.protocol_proxy_auto_retry_status_codes.clone(),
        })
    }

    fn retries_status(&self, status_code: u16) -> bool {
        self.status_codes.contains(&status_code)
    }

    /// 返回本次应等待的时长；None 表示重试预算已用尽。
    /// Retry-After 优先，但同样受单次上限与总预算约束。
    fn schedule(&mut self, retry_after: Option<Duration>) -> Option<Duration> {
        if self.remaining == 0 {
            return None;
        }
        let delay = retry_after
            .unwrap_or(self.next_delay)
            .clamp(PROXY_AUTO_RETRY_MIN_DELAY, PROXY_AUTO_RETRY_MAX_DELAY);
        if self.waited.saturating_add(delay) > PROXY_AUTO_RETRY_TOTAL_BUDGET {
            return None;
        }
        self.remaining -= 1;
        self.waited += delay;
        self.next_delay = self
            .next_delay
            .saturating_mul(2)
            .min(PROXY_AUTO_RETRY_MAX_DELAY);
        Some(delay)
    }
}

/// 等待并在诊断日志里留痕；返回本次退避时长，None 表示不重试。
async fn schedule_proxy_auto_retry(
    state: &mut Option<ProxyAutoRetry>,
    relay: &crate::settings::RelayProfile,
    request_attempt: usize,
    reason: &str,
    retry_after: Option<Duration>,
    retry_reasons: &mut Vec<String>,
    live_request_id: Option<&str>,
) -> Option<Duration> {
    let Some(state) = state.as_mut() else {
        return None;
    };
    let delay = state.schedule(retry_after)?;
    wait_scheduled_proxy_auto_retry(
        delay,
        state.remaining,
        relay,
        request_attempt,
        reason,
        retry_reasons,
        live_request_id,
    )
    .await;
    Some(delay)
}

/// 已经占用了一次重试预算之后的等待：写诊断日志、更新请求日志并 sleep。
async fn wait_scheduled_proxy_auto_retry(
    delay: Duration,
    retries_remaining: u32,
    relay: &crate::settings::RelayProfile,
    request_attempt: usize,
    reason: &str,
    retry_reasons: &mut Vec<String>,
    live_request_id: Option<&str>,
) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.upstream_auto_retry",
        json!({
            "relayId": relay.id,
            "relayName": relay.name,
            "reason": reason,
            "requestAttempt": request_attempt,
            "delayMs": delay.as_millis(),
            "retriesRemaining": retries_remaining,
        }),
    );
    if let Some(id) = live_request_id {
        let mut next_reasons = retry_reasons.clone();
        next_reasons.push(reason.to_string());
        crate::request_log::update_running_retry(
            id,
            request_attempt as u32,
            next_reasons.len() as u32,
            next_reasons,
        );
        crate::request_log::update_running_phase(id, "retrying");
    }
    tokio::time::sleep(delay).await;
    retry_reasons.push(reason.to_string());
}

/// 构造一次失败尝试的日志摘要；message 只保留前 200 字符，避免日志膨胀。
fn attempt_detail(
    request_attempt: usize,
    code: Option<&str>,
    message: Option<&str>,
    delay: Duration,
) -> crate::request_log::RequestAttemptDetail {
    crate::request_log::RequestAttemptDetail {
        attempt: request_attempt as u32,
        code: code.map(str::to_string),
        message: message.map(|message| message.chars().take(200).collect()),
        delay_ms: Some(delay.as_millis() as u64),
    }
}
/// 流式响应预读上限：推理阶段可能带加密推理内容，留足空间；超过则直接放行。
const STREAM_RETRY_PEEK_MAX_BYTES: usize = 8 * 1024 * 1024;
/// 预读最长等待：超时后放行已读前缀，之后不再具备重试条件。
const STREAM_RETRY_PEEK_MAX_WAIT: Duration = Duration::from_secs(120);
/// 整段缓冲模式的上限：超过后回退为流式（已读部分作为前缀转发）。
const BUFFERED_STREAM_MAX_BYTES: usize = 32 * 1024 * 1024;
/// 整段缓冲模式的最长等待，避免客户端请求无限挂起。
const BUFFERED_STREAM_MAX_WAIT: Duration = Duration::from_secs(300);

/// 整段缓冲模式的读取结果。
enum BufferedResponsesStream {
    /// 已读到终态事件或 EOF：body 是完整的 SSE 文本。
    Complete {
        body: Vec<u8>,
        first_output_at_ms: Option<u64>,
        terminal_failure: Option<(Option<String>, Option<String>)>,
    },
    /// 超过容量/时间上限：把已读部分作为流式前缀交给调用方继续转发。
    Overflow {
        prefix: Vec<u8>,
        first_output_at_ms: Option<u64>,
    },
}

fn estimated_tokens_from_chars(chars: usize) -> u64 {
    ((chars as u64) + 3) / 4
}

/// 整段读取 Responses SSE：用于 buffered 模式在转发前识别过载并整段重试。
async fn read_responses_stream_for_buffering(
    upstream: &mut reqwest::Response,
    live_request_id: Option<&str>,
) -> BufferedResponsesStream {
    let started = Instant::now();
    let mut buffered: Vec<u8> = Vec::new();
    let mut scanned = 0usize;
    let mut first_output_at_ms: Option<u64> = None;
    let mut estimated_delta_chars = 0usize;
    let mut last_estimate_update = Instant::now();
    loop {
        let remaining = BUFFERED_STREAM_MAX_WAIT.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return BufferedResponsesStream::Overflow {
                prefix: buffered,
                first_output_at_ms,
            };
        }
        let chunk = match tokio::time::timeout(remaining, upstream.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                return BufferedResponsesStream::Complete {
                    body: buffered,
                    first_output_at_ms,
                    terminal_failure: None,
                };
            }
        };
        buffered.extend_from_slice(&chunk);
        if buffered.len() > BUFFERED_STREAM_MAX_BYTES {
            return BufferedResponsesStream::Overflow {
                prefix: buffered,
                first_output_at_ms,
            };
        }
        while let Some(boundary) = next_sse_boundary(&buffered, scanned) {
            let block = String::from_utf8_lossy(&buffered[scanned..boundary]).into_owned();
            scanned = boundary;
            let Some(event) = sse_block_type(&block) else {
                continue;
            };
            if !is_pre_output_stream_event(&event) && first_output_at_ms.is_none() {
                first_output_at_ms = Some(current_epoch_ms());
                if let Some(id) = live_request_id {
                    crate::request_log::update_running_first_token(id);
                }
            }
            if event.ends_with(".delta")
                && let Some(delta) = sse_block_json(&block)
                    .and_then(|json| json.get("delta").and_then(Value::as_str).map(str::to_string))
            {
                estimated_delta_chars += delta.chars().count();
                if let Some(id) = live_request_id
                    && last_estimate_update.elapsed() >= Duration::from_millis(300)
                {
                    crate::request_log::update_running_output_estimate(
                        id,
                        estimated_tokens_from_chars(estimated_delta_chars),
                    );
                    last_estimate_update = Instant::now();
                }
            }
            match event.as_str() {
                "response.completed" => {
                    if let Some(id) = live_request_id {
                        crate::request_log::update_running_output_estimate(
                            id,
                            estimated_tokens_from_chars(estimated_delta_chars),
                        );
                    }
                    return BufferedResponsesStream::Complete {
                        body: buffered,
                        first_output_at_ms,
                        terminal_failure: None,
                    };
                }
                "response.failed" | "response.incomplete" | "error" => {
                    let (code, message) = stream_failure_payload(&block);
                    return BufferedResponsesStream::Complete {
                        body: buffered,
                        first_output_at_ms,
                        terminal_failure: Some((code, message)),
                    };
                }
                _ => {}
            }
        }
    }
}

/// 流式预读的提交模式。旧模式保留给需要「退回旧行为」的用户。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamCommitMode {
    /// 旧行为：遇到第一个不可安全重放的事件就立即提交，工具调用参数不缓冲。
    Immediate,
    /// 工具调用参数整项缓冲，首段可见输出后再等宽限窗口才提交。
    VisibleGrace,
}

/// 流式预读的提交策略：决定什么时候把已缓冲字节交给 Codex。
///
/// 「提交」之后响应就不可回滚，上游再报过载只能交给 Codex 原生重试；所以
/// 新模式下预读会尽量把提交点往后挪，且只在不影响可见延迟的阶段多缓冲。
#[derive(Debug, Clone, Copy)]
struct StreamStartCommitPolicy {
    mode: StreamCommitMode,
    /// 首段可见输出出现后继续缓冲的时长；0 表示一出现可见输出就提交。
    first_visible_grace: Duration,
}

impl StreamStartCommitPolicy {
    fn from_settings(settings: &crate::settings::BackendSettings) -> Self {
        let mode = if settings.protocol_proxy_auto_retry_mode == "streaming-legacy" {
            StreamCommitMode::Immediate
        } else {
            StreamCommitMode::VisibleGrace
        };
        Self {
            mode,
            // 设置加载/部分更新都会夹取；这里再兜一层，避免直接构造的设置
            // 让 `Instant + Duration` 溢出。
            first_visible_grace: Duration::from_millis(
                settings
                    .protocol_proxy_auto_retry_commit_delay_ms
                    .min(10_000),
            ),
        }
    }

    /// 重试预算已用尽时退回旧提交时机：继续缓冲不会带来任何重试收益。
    fn without_retry_budget(self) -> Self {
        Self {
            mode: StreamCommitMode::Immediate,
            ..self
        }
    }
}

/// 预读阶段单个事件的分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamStartEventKind {
    /// 预读可以结束：正常完成、终态失败、或非消息输出项已完成。
    Terminal,
    /// 用户可见内容（正文/拒绝/音频/图片）：开始或继续首字提交宽限。
    Visible,
    /// 不可见阶段（推理、工具调用参数、工具项生命周期、未知事件）：继续缓冲。
    Hold,
}

/// 判断事件属于哪一类。默认「继续按住」：未知事件宁可多缓冲（最坏退化成
/// 接近整段缓冲），也不要把还能重试的窗口提前关掉；总长度仍受 8 MiB / 120 秒
/// 预读上限约束，不会无限期扣住响应。
///
/// `event` 应使用 data JSON 里的 `type`（没有时回退 SSE `event:` 名），与
/// `ResponsesSseTransformer` 的判定顺序保持一致。
fn stream_start_event_kind(
    block: &str,
    event: &str,
    mode: StreamCommitMode,
) -> StreamStartEventKind {
    if stream_event_is_retry_safe(block, event) {
        return StreamStartEventKind::Hold;
    }
    // 旧行为：任何不可安全重放的事件（正文、工具调用参数、图片等）都立即提交。
    if mode == StreamCommitMode::Immediate {
        return StreamStartEventKind::Terminal;
    }
    match event {
        "response.completed" | "response.failed" | "response.incomplete" | "error" => {
            return StreamStartEventKind::Terminal;
        }
        // 非消息输出项完成（function_call / custom_tool_call / web_search_call …）：
        // Codex 要等参数完整才会执行工具，此时提交没有可见延迟损失；后续事件
        // 按普通流式转发，也不再占用重试窗口。
        "response.output_item.done" => {
            let item_type = sse_block_json(block)
                .and_then(|json| {
                    json.pointer("/item/type")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            return if !item_type.is_empty() && item_type != "reasoning" && item_type != "message" {
                StreamStartEventKind::Terminal
            } else {
                StreamStartEventKind::Hold
            };
        }
        _ => {}
    }
    match event {
        // 正常流式上游会先发 delta；部分兼容上游只在 done 事件里带完整文本，
        // 这里也算可见输出，避免白等预读上限。
        "response.output_text.delta"
        | "response.output_text.done"
        | "response.refusal.delta"
        | "response.refusal.done"
        | "response.audio.delta"
        | "response.audio.done"
        | "response.audio.transcript.delta"
        | "response.audio.transcript.done"
        | "response.image_generation_call.partial_image" => StreamStartEventKind::Visible,
        _ => StreamStartEventKind::Hold,
    }
}

fn forward_replay_safe(prefix: Vec<u8>, first_output_at_ms: Option<u64>) -> StreamStartPeek {
    StreamStartPeek::Forward {
        prefix,
        first_output_at_ms,
    }
}

/// 预读 200 SSE 开头后的结果。
enum StreamStartPeek {
    /// 放行：`prefix` 是已经读出的字节，必须原样写给 Codex，剩余部分继续流式转发。
    Forward {
        prefix: Vec<u8>,
        first_output_at_ms: Option<u64>,
    },
    /// 收到终态失败（response.failed / incomplete / error），尚未提交给 Codex。
    ///
    /// 即使之前已经缓冲了可见输出（宽限窗口内），只要还没提交就仍可重试；
    /// 不能重试时把 `prefix` 和首字时间一起交回，避免请求日志把首字耗时
    /// 记成提交时刻。
    Failed {
        prefix: Vec<u8>,
        code: Option<String>,
        message: Option<String>,
        first_output_at_ms: Option<u64>,
    },
}

fn current_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 响应生命周期事件：既没有内容，也不代表模型开始产出。官方后端在容量不足时
/// 先回 200 和 response.created/in_progress，再在流里发 response.failed
/// （Codex 映射为 server_overloaded）。这些事件留在预读缓冲里不影响正确性。
fn is_pre_output_stream_event(event: &str) -> bool {
    matches!(
        event,
        "response.created" | "response.in_progress" | "response.queued" | "response.metadata"
    )
}

/// 该事件是否还处于「可以安全重放」的阶段：没有给 Codex 看过可见内容。
///
/// 除了响应创建类事件，推理摘要与加密推理项也先按住——官方过载错误经常在
/// 模型思考阶段才返回；此时客户端还没收到正文，重放不会出现重复内容。
/// 可见正文/图片等由 [`stream_start_event_kind`] 单独判定，不再「一出现就放行」。
fn stream_event_is_retry_safe(block: &str, event: &str) -> bool {
    if is_pre_output_stream_event(event) {
        return true;
    }
    match event {
        "response.output_item.added" | "response.output_item.done" => sse_block_json(block)
            .and_then(|json| {
                json.pointer("/item/type")
                    .and_then(Value::as_str)
                    .map(|item_type| item_type == "reasoning")
            })
            .unwrap_or(false),
        "response.reasoning_summary_part.added"
        | "response.reasoning_summary_text.delta"
        | "response.reasoning_summary_text.done"
        | "response.reasoning_text.delta"
        | "response.reasoning_text.done" => true,
        "response.content_part.added" => sse_block_json(block)
            .and_then(|json| {
                json.pointer("/part/type")
                    .and_then(Value::as_str)
                    .map(|part_type| part_type.starts_with("reasoning"))
            })
            .unwrap_or(false),
        _ => false,
    }
}

/// 把上游过载错误规范成 Codex 能识别的 `response.failed` 事件。
///
/// 官方有时用 `error` 事件 + `server_is_overloaded`，Codex 只把它当普通流错误
/// （显示 stream disconnected 并走重连）。改写成 `response.failed` 且 code 使用
/// Codex 已登记过的 `server_overloaded`，客户端会按容量错误走自己的重试阶梯。
fn normalized_overload_failure_event(data_json: &Value) -> Option<String> {
    let (code, message) = (
        data_json
            .pointer("/error/code")
            .or_else(|| data_json.pointer("/response/error/code"))
            .and_then(Value::as_str),
        data_json
            .pointer("/error/message")
            .or_else(|| data_json.pointer("/response/error/message"))
            .and_then(Value::as_str),
    );
    let message = message.unwrap_or("Our servers are currently overloaded. Please try again later.");
    let response_id = data_json
        .pointer("/response/id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let payload = json!({
        "type": "response.failed",
        "response": {
            "id": response_id,
            "status": "failed",
            "error": {
                "code": "server_overloaded",
                "message": message,
            }
        }
    });
    let _ = code;
    Some(format!(
        "event: response.failed\ndata: {}\n\n",
        serde_json::to_string(&payload).ok()?
    ))
}

/// 找到 `from` 之后第一个完整 SSE 事件边界（返回边界之后的字节下标）。
fn next_sse_boundary(bytes: &[u8], from: usize) -> Option<usize> {
    let mut index = from;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            if bytes.get(index + 1) == Some(&b'\n') {
                return Some(index + 2);
            }
            if bytes.get(index + 1) == Some(&b'\r') && bytes.get(index + 2) == Some(&b'\n') {
                return Some(index + 3);
            }
        }
        index += 1;
    }
    None
}

/// 从 response.failed / error 事件里提取 (code, message)，兼容几种常见包裹层级。
fn stream_failure_payload(block: &str) -> (Option<String>, Option<String>) {
    let mut data_parts = Vec::new();
    for line in block.lines() {
        if let Some(data) = strip_sse_field(line.trim_end_matches('\r'), "data") {
            data_parts.push(data);
        }
    }
    let data = data_parts.join("\n");
    let Ok(json) = serde_json::from_str::<Value>(data.trim()) else {
        return (None, None);
    };
    let error = json
        .get("response")
        .and_then(|response| response.get("error"))
        .or_else(|| json.get("error"));
    let code = error
        .and_then(|error| error.get("code").or_else(|| error.get("type")))
        .and_then(Value::as_str)
        .or_else(|| json.get("code").and_then(Value::as_str))
        .map(ToString::to_string);
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .or_else(|| json.get("message").and_then(Value::as_str))
        .map(ToString::to_string);
    (code, message)
}

/// 把流内失败事件的 code/message 压成一行，写进请求日志与错误原因。
fn stream_failure_detail(code: Option<&str>, message: Option<&str>) -> String {
    let message = message
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(|message| message.chars().take(200).collect::<String>());
    let mut parts = Vec::new();
    if let Some(code) = code.map(str::trim).filter(|code| !code.is_empty()) {
        parts.push(format!("code={code}"));
    }
    if let Some(message) = message {
        parts.push(format!("message={message}"));
    }
    parts.join("，")
}

/// 流内终态失败是否值得重试。只认容量/过载，避免把上下文超限等永久错误重放。
fn stream_failure_is_retryable(code: Option<&str>, message: Option<&str>) -> bool {
    let code = code.unwrap_or("").trim().to_ascii_lowercase();
    // 官方至少见过两种拼法：server_overloaded 与 server_is_overloaded；
    // 其它上游也会用 capacity / overloaded 系列 code。
    if code.contains("overload")
        || code.contains("capacity")
        || matches!(code.as_str(), "server_overloaded" | "server_is_overloaded")
    {
        return true;
    }
    let message = message.unwrap_or("").trim().to_ascii_lowercase();
    message.contains("selected model is at capacity")
        || message.contains("model is at capacity")
        || message.contains("at capacity")
        || message.contains("overloaded")
}

/// 预读 Responses 流开头：识别「200 + 流内 response.failed」的容量错误，
/// 并尽量推迟不可回滚的提交点，给代理重试多留窗口。
///
/// 提交时机：
/// - `streaming-legacy`：遇到第一个不可安全重放的事件立即提交（旧行为）；
/// - 推理、工具调用参数等不可见阶段一直按住，随时可以整体丢弃重发；
/// - 非消息输出项完成（例如 function_call 参数收齐）立即提交：Codex 本来就要
///   等参数完整才会执行工具，这里没有可见延迟；
/// - 第一段可见输出（正文/拒绝/音频/图片）后，再等
///   [`StreamStartCommitPolicy::first_visible_grace`]；窗口内遇到容量错误仍丢弃
///   重试，窗口到期才提交；
/// - response.completed、失败事件、EOF/读失败、8 MiB/120 秒上限都结束预读。
async fn peek_responses_stream_start(
    upstream: &mut reqwest::Response,
    policy: StreamStartCommitPolicy,
    live_request_id: Option<&str>,
) -> StreamStartPeek {
    let started = Instant::now();
    let mut buffered: Vec<u8> = Vec::new();
    let mut scanned = 0usize;
    let mut first_output_at_ms: Option<u64> = None;
    let mut visible_deadline: Option<Instant> = None;
    loop {
        if buffered.len() > STREAM_RETRY_PEEK_MAX_BYTES {
            return forward_replay_safe(buffered, first_output_at_ms);
        }
        let now = Instant::now();
        if visible_deadline.is_some_and(|deadline| now >= deadline) {
            return forward_replay_safe(buffered, first_output_at_ms);
        }
        let remaining = STREAM_RETRY_PEEK_MAX_WAIT.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return forward_replay_safe(buffered, first_output_at_ms);
        }
        // 同时受预读总超时和首字提交宽限约束：宽限到期也要醒来提交，
        // 不能等上游再发 chunk 才检查。
        let wait = match visible_deadline {
            Some(deadline) => remaining.min(deadline.saturating_duration_since(now)),
            None => remaining,
        };
        let chunk = match tokio::time::timeout(wait, upstream.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            // EOF、读失败、宽限到期或预读超时：原样放行已缓冲前缀。
            Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                return forward_replay_safe(buffered, first_output_at_ms);
            }
        };
        buffered.extend_from_slice(&chunk);
        while let Some(boundary) = next_sse_boundary(&buffered, scanned) {
            let block = String::from_utf8_lossy(&buffered[scanned..boundary]).into_owned();
            scanned = boundary;
            let Some(sse_event) = sse_block_type(&block) else {
                continue;
            };
            // 与 ResponsesSseTransformer 一致：data JSON 的 type 优先，回退 SSE
            // event: 名。少数上游两者不一致，只认 event: 会把可见内容当成未知事件。
            let event = sse_block_json(&block)
                .and_then(|json| json.get("type").and_then(Value::as_str).map(str::to_string))
                .filter(|event| !event.is_empty())
                .unwrap_or_else(|| sse_event.clone());
            if first_output_at_ms.is_none() && !is_pre_output_stream_event(&event) {
                first_output_at_ms = Some(current_epoch_ms());
                // 请求日志的「首字耗时」仍按上游第一段输出计；预读可能还要
                // 继续按住一段时间，这里先让进行中的日志行看到首字。
                if let Some(id) = live_request_id {
                    crate::request_log::update_running_first_token(id);
                }
            }
            match stream_start_event_kind(&block, &event, policy.mode) {
                StreamStartEventKind::Terminal => {
                    if matches!(
                        event.as_str(),
                        "response.failed" | "response.incomplete" | "error"
                    ) {
                        let (code, message) = stream_failure_payload(&block);
                        return StreamStartPeek::Failed {
                            prefix: buffered,
                            code,
                            message,
                            first_output_at_ms,
                        };
                    }
                    return forward_replay_safe(buffered, first_output_at_ms);
                }
                StreamStartEventKind::Visible => {
                    if policy.first_visible_grace.is_zero() {
                        return forward_replay_safe(buffered, first_output_at_ms);
                    }
                    visible_deadline
                        .get_or_insert_with(|| Instant::now() + policy.first_visible_grace);
                }
                StreamStartEventKind::Hold => {}
            }
        }
    }
}

/// 请求日志里使用的上游协议名。
fn upstream_wire_api_label(api: UpstreamWireApi) -> &'static str {
    match api {
        UpstreamWireApi::Responses => "responses",
        UpstreamWireApi::ChatCompletions => "chatCompletions",
        UpstreamWireApi::AnthropicMessages => "anthropicMessages",
        UpstreamWireApi::AudioTranscriptions => "audioTranscriptions",
        UpstreamWireApi::ImageGenerations => "imageGenerations",
        UpstreamWireApi::ImageEdits => "imageEdits",
    }
}

const MAX_NATIVE_COMPACTION_PROBE_BODY_BYTES: usize = 32 * 1024 * 1024;
const THINK_OPEN_TAG: &str = "<think>";
const THINK_CLOSE_TAG: &str = "</think>";
const EXTRA_CHAT_PASSTHROUGH_FIELDS: &[&str] = &[
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "metadata",
    "n",
    "presence_penalty",
    "response_format",
    "seed",
    "service_tier",
    "stop",
    "stream_options",
    "top_logprobs",
    "user",
];
const ERROR_BODY_PREVIEW_LIMIT: usize = 1024;

fn exec_retry_after_duration(
    headers: &reqwest::header::HeaderMap,
    now: SystemTime,
) -> Option<Duration> {
    let value = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = httpdate::parse_http_date(value).ok()?;
    Some(retry_at.duration_since(now).unwrap_or_default())
}

/// codex v2 远程压缩请求在 input 末尾携带的控制 item（openai/codex compact_remote_v2）。
const COMPACTION_TRIGGER_TYPE: &str = "compaction_trigger";
/// codex 期望响应里恰好包含一个的压缩结果 item，`encrypted_content` 只透传不校验。
const COMPACTION_OUTPUT_TYPE: &str = "compaction";
/// 本地摘要的持久化格式标识；原生 encrypted_content 不使用这个命名空间。
/// 后缀为 JSON string，不依赖客户端可能丢弃的 item id 或额外字段。
const LOCAL_COMPACTION_PREFIX: &str = "codexplusplus:summary:v1:";
/// 本地代理生成摘要时注入的 user 指令（对齐 openai/codex prompts/templates/compact/prompt.md）。
const COMPACTION_SUMMARY_INSTRUCTION: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress and key decisions made\n- Important context, constraints, or user preferences\n- What remains to be done (clear next steps)\n- Any critical data, examples, or references needed to continue\n\nBe concise, structured, and focused on helping the next LLM seamlessly continue the work.";
/// 历史回放时 `compaction` item 展开成的文本前缀（对齐 codex SUMMARY_PREFIX 语义）。
const COMPACTION_REPLAY_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process. Here is the summary produced by the other language model:\n";

/// 判断 Responses 请求体是否为 codex v2 远程压缩请求。
/// 当前 Codex 把 trigger 追加在 input 末尾；这里仍接受任意位置，避免漏检。
pub fn request_has_compaction_trigger(body: &Value) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some(COMPACTION_TRIGGER_TYPE)
            })
        })
}

/// 从请求 input 中剥离全部 compaction_trigger 控制项。
/// 只在确认上游没有原生 compaction 项、需要改写成普通摘要时使用。
fn strip_compaction_trigger(mut body: Value) -> Value {
    if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
        items.retain(|item| {
            item.get("type").and_then(Value::as_str) != Some(COMPACTION_TRIGGER_TYPE)
        });
    }
    body
}

/// 把压缩摘要请求改写成上游能理解的普通生成请求：
/// - input 末尾注入 user 摘要指令；
/// - tools/parallel_tool_calls 清空，避免摘要阶段触发工具调用。
fn rewrite_request_for_compaction(mut body: Value) -> Value {
    if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
        items.push(json!({
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": COMPACTION_SUMMARY_INSTRUCTION }]
        }));
    }
    body["tools"] = json!([]);
    body["tool_choice"] = json!("none");
    body["parallel_tool_calls"] = json!(false);
    body
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatReasoningStyle {
    Default,
    DeepSeek,
    LowHigh,
    OpenRouter,
    Thinking,
    EnableThinking,
    ReasoningSplit,
}

#[derive(Debug, Clone, Default)]
struct CodexToolContext {
    custom_tools: BTreeMap<String, CodexCustomToolSpec>,
    function_tools: BTreeMap<String, CodexFunctionToolSpec>,
    has_custom_tools: bool,
    has_namespace_tools: bool,
    /// 只用于拒绝模型虚构的已移除原生调用，不参与客户端工具映射。
    rejected_native_search: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct CodexCustomToolSpec {
    openai_name: String,
    kind: CodexCustomToolKind,
}

#[derive(Debug, Clone, Default)]
struct CodexFunctionToolSpec {
    namespace: String,
    name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexCustomToolKind {
    Raw,
    ApplyPatch,
    BuiltIn,
    /// Codex 的 tool_search 工具（MCP 延迟加载检索，execution: client）。
    ToolSearch,
}

impl Default for CodexCustomToolKind {
    fn default() -> Self {
        Self::Raw
    }
}

impl CodexToolContext {
    fn is_custom_tool_proxy(&self, upstream_name: &str) -> bool {
        self.custom_tools.contains_key(upstream_name)
    }

    fn is_tool_search_proxy(&self, upstream_name: &str) -> bool {
        self.custom_tools.get(upstream_name).map(|spec| spec.kind)
            == Some(CodexCustomToolKind::ToolSearch)
    }

    /// 上游把 codex 的原生 `web_search` 工具当成普通自定义工具回传时，中转层
    /// 此前一律产出 `custom_tool_call`，而客户端只认 `web_search_call`
    /// （issue #1209 / #586：web_search 被错译成 custom_tool_call 后，客户端
    /// 拿不到搜索结果，表现为「工具调用后无反应」或上游报 tool 名不识别）。
    fn is_builtin_web_search_proxy(&self, upstream_name: &str) -> bool {
        self.rejected_native_search.contains(upstream_name) || self.custom_tools.get(upstream_name).is_some_and(|spec| {
            spec.kind == CodexCustomToolKind::BuiltIn && spec.openai_name == "web_search"
        })
    }

    fn original_custom_tool_name(&self, upstream_name: &str) -> String {
        self.custom_tools
            .get(upstream_name)
            .map(|spec| spec.openai_name.clone())
            .unwrap_or_else(|| upstream_name.to_string())
    }

    fn openai_name_for_function_tool(&self, upstream_name: &str) -> (String, String) {
        let Some(spec) = self.function_tools.get(upstream_name) else {
            return (upstream_name.to_string(), String::new());
        };
        let name = if spec.name.is_empty() {
            upstream_name.to_string()
        } else {
            spec.name.clone()
        };
        (name, spec.namespace.clone())
    }
}

pub fn local_responses_proxy_base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1")
}

#[derive(Debug)]
pub(crate) struct UnsupportedEncryptedAgentContent;

impl std::fmt::Display for UnsupportedEncryptedAgentContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "unsupported_encrypted_agent_content: Chat Completions 上游无法处理加密的 agent 消息内容，请使用支持该协议的 Responses 上游"
        )
    }
}

impl std::error::Error for UnsupportedEncryptedAgentContent {}

impl UnsupportedEncryptedAgentContent {
    pub(crate) fn response_body(&self) -> Value {
        json!({"error":{
            "type":"invalid_request_error",
            "code":"unsupported_encrypted_agent_content",
            "param":"input",
            "message":self.to_string()
        }})
    }
}

#[derive(Debug)]
pub(crate) struct UnsupportedNativeWebSearch;

impl std::fmt::Display for UnsupportedNativeWebSearch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "unsupported_native_web_search: Chat 路由不执行服务端原生搜索，请使用客户端搜索工具或支持原生搜索的 Responses 上游"
        )
    }
}

impl std::error::Error for UnsupportedNativeWebSearch {}

impl UnsupportedNativeWebSearch {
    pub(crate) fn response_body(&self) -> Value {
        json!({"error": {
            "type": "invalid_request_error", "code": "unsupported_native_web_search",
            "param": "tool_choice", "message": self.to_string()
        }})
    }
}

/// 附件尚无 Chat 映射，必须在任何上游发送之前明确拒绝。
#[derive(Debug)]
pub(crate) struct UnsupportedInputFile {
    param: String,
}

impl std::fmt::Display for UnsupportedInputFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Chat/Anthropic 转换不支持 input_file 附件，请使用支持附件的 Responses 上游"
        )
    }
}

impl std::error::Error for UnsupportedInputFile {}

impl UnsupportedInputFile {
    pub(crate) fn response_body(&self) -> Value {
        json!({"error":{"type":"invalid_request_error", "code":"unsupported_input_file",
            "param":self.param, "message":self.to_string()}})
    }
}

fn validate_chat_attachments(body: &Value) -> anyhow::Result<()> {
    fn content(value: &Value, path: &str) -> anyhow::Result<()> {
        if value.get("type").and_then(Value::as_str) == Some("input_file") {
            return Err(UnsupportedInputFile {
                param: path.to_string(),
            }
            .into());
        }
        if let Some(parts) = value.as_array() {
            for (index, part) in parts.iter().enumerate() {
                if part.get("type").and_then(Value::as_str) == Some("input_file") {
                    return Err(UnsupportedInputFile {
                        param: format!("{path}[{index}]"),
                    }
                    .into());
                }
            }
        }
        Ok(())
    }
    fn item(value: &Value, path: &str) -> anyhow::Result<()> {
        content(value, path)?;
        let kind = value.get("type").and_then(Value::as_str);
        if matches!(kind, None | Some("message" | "agent_message")) {
            if let Some(parts) = value.get("content") {
                content(parts, &format!("{path}.content"))?;
            }
        } else if kind == Some("function_call_output") {
            if let Some(parts) = value.get("output") {
                content(parts, &format!("{path}.output"))?;
            }
        }
        Ok(())
    }
    if let Some(input) = body.get("input") {
        if let Some(items) = input.as_array() {
            for (index, value) in items.iter().enumerate() {
                item(value, &format!("input[{index}]"))?;
            }
        } else {
            item(input, "input")?;
        }
    }
    Ok(())
}
fn remove_unsupported_native_search(body: &mut Value) -> anyhow::Result<()> {
    let choice = body.get("tool_choice");
    if choice
        .and_then(|choice| choice.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "web_search" | "web_search_preview"))
    {
        return Err(UnsupportedNativeWebSearch.into());
    }
    let required = choice.and_then(Value::as_str) == Some("required");
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        let before = tools.len();
        // 同名 function/custom 和命名空间工具由客户端执行，必须保留。
        tools.retain(|tool| {
            !matches!(
                tool.get("type").and_then(Value::as_str),
                Some("web_search" | "web_search_preview")
            )
        });
        if before != tools.len() && required && tools.is_empty() {
            return Err(UnsupportedNativeWebSearch.into());
        }
    }
    Ok(())
}

pub fn responses_to_chat_completions(body: Value) -> anyhow::Result<Value> {
    responses_to_chat_completions_with_options(body, false)
}

pub fn responses_to_chat_completions_with_options(
    mut body: Value,
    standard: bool,
) -> anyhow::Result<Value> {
    remove_unsupported_native_search(&mut body)?;
    validate_chat_attachments(&body)?;
    expand_local_compaction_history(&mut body);
    if input_has_native_compaction(&body) {
        return Err(NativeCompactionUnsupported::history().into());
    }
    // agent_message 的 encrypted_content 片段在 multi_agent v2 里实际承载明文任务；
    // 转换时按文本透传，opaque 内容则明确失败，避免静默发空任务。
    let mut result = json!({});

    if let Some(model) = body.get("model") {
        result["model"] = model.clone();
    }

    let mut messages = Vec::new();
    if let Some(instructions) = body.get("instructions") {
        let text = instruction_text(instructions);
        if !text.is_empty() {
            messages.push(json!({ "role": "system", "content": text }));
        }
    }

    if let Some(input) = body.get("input") {
        append_responses_input(input, &mut messages)?;
    }
    // 必须在 enforce_tool_call_pairing 之前：配对一旦被错误摘除就无法恢复。
    relocate_interleaved_non_tool_messages(&mut messages);
    enforce_tool_call_pairing(&mut messages);
    // 配对判定之后仍有无主 tool 消息（上游会直接 400），降级成 user 保住内容。
    degrade_unpaired_tool_messages(&mut messages);
    // 必须在 enforce_tool_call_pairing 之后：它依赖 tool 消息的连续性，
    // 而这一步会往中间插入 user 消息。
    relocate_tool_output_images(&mut messages);
    ensure_tool_call_reasoning_content(&mut messages);
    normalize_chat_messages(&mut messages);
    let model = body.get("model").and_then(Value::as_str).unwrap_or("");
    normalize_image_data_urls_for_model(&mut messages, model);
    let messages = collapse_system_messages_to_head(messages);
    result["messages"] = json!(messages);
    if let Some(value) = body.get("max_output_tokens") {
        if is_openai_o_series(model) {
            result["max_completion_tokens"] = value.clone();
        } else {
            result["max_tokens"] = value.clone();
        }
    }
    if let Some(value) = body.get("max_tokens") {
        result["max_tokens"] = value.clone();
    }
    if let Some(value) = body.get("max_completion_tokens") {
        result["max_completion_tokens"] = value.clone();
    }

    for key in ["temperature", "top_p", "stream"] {
        if let Some(value) = body.get(key) {
            result[key] = value.clone();
        }
    }
    if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        let mut stream_options = body
            .get("stream_options")
            .cloned()
            .unwrap_or_else(|| json!({}));
        stream_options["include_usage"] = json!(true);
        result["stream_options"] = stream_options;
    }

    apply_chat_reasoning_options(&mut result, &body, model, standard);

    let ChatToolProjection { context: tool_context, tools: converted } = chat_tool_projection(&body);
    // Codex 客户端把 tool_search 命中的 MCP 命名空间挂在历史里的 tool_search_output
    // item 上，却不会把它们提升进下一轮请求的 tools 数组。这里补上这一跳：先登记进
    // tool context（模型调用回来时才能还原 namespace），再用既有 namespace 链路展开
    // 成 mcp__<server>__<tool>，否则上游只看到命名空间外壳，调不到内层工具。
    let mut has_chat_tools = false;
    if !converted.is_empty() {
        has_chat_tools = true;
        let mut tools = converted;
        // Gemini 的 function_declarations 只接受字符串枚举值：Codex 工具 schema
        // 里出现 `enum: [true, false]` 这类布尔/数字值时会整轮 400
        // （`enum[0] (TYPE_STRING)`，issue #2404）。只对 Gemini 系上游改写，
        // 其他上游保持原始 JSON Schema 语义。
        if upstream_requires_string_enums(model) {
            for tool in tools.iter_mut() {
                coerce_enum_values_to_strings(tool);
            }
        }
        result["tools"] = json!(tools);
    }

    if has_chat_tools {
        if let Some(tool_choice) = body
            .get("tool_choice")
            .and_then(|value| responses_tool_choice_to_chat(value, &tool_context))
        {
            result["tool_choice"] = tool_choice;
        }
        if let Some(value) = body.get("parallel_tool_calls") {
            result["parallel_tool_calls"] = value.clone();
        }
    }

    for key in EXTRA_CHAT_PASSTHROUGH_FIELDS {
        if *key == "stream_options" && result.get("stream_options").is_some() {
            continue;
        }
        if let Some(value) = body.get(*key) {
            result[*key] = value.clone();
        }
    }

    Ok(result)
}

fn prompt_prefix_stabilizer_enabled(relay: &crate::settings::RelayProfile) -> bool {
    if crate::relay_features::stabilize_prompt_prefix(&relay.id) {
        return true;
    }
    let suffix = relay
        .id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_uppercase() } else { '_' })
        .collect::<String>();
    matches!(
        std::env::var(format!("CODEX_PLUS_STABILIZE_PROMPT_PREFIX_{suffix}"))
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1" | "true" | "yes" | "on")
    )
}

const IMAGE_RESIZE_NOTICE_OPEN: &str = "<image_resize_notice>";
const IMAGE_RESIZE_NOTICE_CLOSE: &str = "</image_resize_notice>";

/// 清理完整 notice 与跨字段残片。完整块保持周围字节不变；孤儿开始标签会吞掉
/// 其后的残片，孤儿结束标签会丢掉它之前的残片，避免客户端轮间重写把漂移字节
/// 固化进前缀（#2448）。
fn strip_notice_blocks_in_text(text: &str) -> (String, bool, bool) {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    let mut removed = false;
    let mut unpaired = false;

    loop {
        let open_at = rest.find(IMAGE_RESIZE_NOTICE_OPEN);
        let close_at = rest.find(IMAGE_RESIZE_NOTICE_CLOSE);
        let first_is_open = match (open_at, close_at) {
            (None, None) => {
                output.push_str(rest);
                break;
            }
            (Some(open), Some(close)) => close >= open,
            (Some(_), None) => true,
            (None, Some(_)) => false,
        };

        if first_is_open {
            let open = open_at.expect("open marker must exist");
            match close_at {
                Some(close) if close > open => {
                    output.push_str(&rest[..open]);
                    rest = &rest[close + IMAGE_RESIZE_NOTICE_CLOSE.len()..];
                    removed = true;
                }
                _ => {
                    output.push_str(&rest[..open]);
                    output.push_str(&rest[open + IMAGE_RESIZE_NOTICE_OPEN.len()..]);
                    removed = true;
                    unpaired = true;
                    break;
                }
            }
        } else {
            let close = close_at.expect("close marker must exist");
            output.push_str(&rest[..close]);
            rest = &rest[close + IMAGE_RESIZE_NOTICE_CLOSE.len()..];
            removed = true;
            unpaired = true;
        }
    }

    (output, removed, unpaired)
}

/// 只清理指令与用户消息中的客户端 notice；工具结果和模型输出可能包含同名标签原文。
fn strip_image_resize_notice_blocks(messages: &mut Vec<Value>) {
    let mut unpaired = false;
    messages.retain_mut(|message| {
        if !matches!(
            message.get("role").and_then(Value::as_str),
            Some("system" | "developer" | "user")
        ) {
            return true;
        }
        let has_tool_calls = message.get("tool_calls").is_some();
        let Some(content) = message.get_mut("content") else {
            return true;
        };
        match content {
            Value::String(text) => {
                let (cleaned, removed, orphan) = strip_notice_blocks_in_text(text);
                unpaired |= orphan;
                if !removed {
                    return true;
                }
                *text = cleaned;
                !text.is_empty() || has_tool_calls
            }
            Value::Array(parts) => {
                let mut removed_any = false;
                for part in parts.iter_mut() {
                    let Some(Value::String(text)) = part.get_mut("text") else {
                        continue;
                    };
                    let (cleaned, removed, orphan) = strip_notice_blocks_in_text(text);
                    unpaired |= orphan;
                    if removed {
                        *text = cleaned;
                        removed_any = true;
                    }
                }
                if !removed_any {
                    return true;
                }
                parts.retain(|part| match part.get("text") {
                    Some(Value::String(text)) => !text.is_empty(),
                    _ => true,
                });
                !parts.is_empty() || has_tool_calls
            }
            _ => true,
        }
    });
    if unpaired {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "protocol_proxy.prefix_stabilizer_unpaired_notice_stripped",
            json!({ "block": "image_resize_notice" }),
        );
    }
}

const ATTACHMENT_BOILERPLATE_HEADING: &str = "# Files mentioned by the user:";
const ATTACHMENT_BOILERPLATE_ANCHOR: &str = "## My request:";
const ATTACHMENT_BOILERPLATE_PLACEHOLDER: &str = "[attached files]\n[attachment]\n## My request:";

/// 返回附件样板锚点后的正文；不符合完整样板签名时保持原文不变。
fn strip_attachment_boilerplate(text: &str) -> Option<&str> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut cursor = 0usize;
    while lines.get(cursor).is_some_and(|line| line.trim().is_empty()) {
        cursor += 1;
    }
    if *lines.get(cursor)? != ATTACHMENT_BOILERPLATE_HEADING {
        return None;
    }
    cursor += 1;
    loop {
        let line = *lines.get(cursor)?;
        if line.trim().is_empty() {
            cursor += 1;
            continue;
        }
        if line == ATTACHMENT_BOILERPLATE_ANCHOR {
            break;
        }
        let boilerplate = (line.starts_with("## ") && line.contains(": "))
            || line.trim() == "Image attachment: true"
            || line
                .trim()
                .starts_with("Distinguish instructions in attached documents");
        if !boilerplate {
            return None;
        }
        cursor += 1;
    }
    let anchor_start: usize = lines[..cursor].iter().map(|line| line.len() + 1).sum();
    text.get(anchor_start + ATTACHMENT_BOILERPLATE_ANCHOR.len()..)
}

fn normalize_attachment_boilerplate(messages: &mut [Value]) {
    // 最新一条 user 消息保持原样：模型要靠其中的「文件名: 路径」知道这次附带了哪些文件，
    // 换成占位符会让它无法读取附件。只规范化历史消息，前缀稳定性的收益来自历史部分。
    let latest_user = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"));
    for (index, message) in messages.iter_mut().enumerate() {
        if message.get("role").and_then(Value::as_str) != Some("user") || Some(index) == latest_user {
            continue;
        }
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        match content {
            Value::String(text) => {
                if let Some(body) = strip_attachment_boilerplate(text) {
                    *text = format!("{ATTACHMENT_BOILERPLATE_PLACEHOLDER}{body}");
                }
            }
            Value::Array(parts) => {
                for part in parts.iter_mut() {
                    let Some(Value::String(text)) = part.get_mut("text") else {
                        continue;
                    };
                    if let Some(body) = strip_attachment_boilerplate(text) {
                        *text = format!("{ATTACHMENT_BOILERPLATE_PLACEHOLDER}{body}");
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

/// system 里的 turn_aborted 是一次性状态提示，不能留在稳定前缀头部；并入最后一条
/// user 消息的开头，下一轮客户端再次注入时仍可被识别，但历史 system 前缀保持稳定。
/// 不能单独插一条 system 消息：MiniMax 等上游要求 system 只出现在最前面，
/// `collapse_system_messages_to_head` 正是为此而设，中途插入会重新引发 400。
fn relocate_turn_aborted_blocks(messages: &mut Vec<Value>) {
    const OPEN: &str = "<turn_aborted>";
    const CLOSE: &str = "</turn_aborted>";
    let Some(head) = messages.first_mut() else { return };
    if head.get("role").and_then(Value::as_str) != Some("system") {
        return;
    }
    let Some(Value::String(content)) = head.get("content") else {
        return;
    };
    if !content.contains(OPEN) {
        return;
    }
    let content = content.clone();
    let mut stripped = String::with_capacity(content.len());
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    while let Some(open_rel) = content[cursor..].find(OPEN) {
        let open = cursor + open_rel;
        stripped.push_str(&content[cursor..open]);
        let Some(close_rel) = content[open..].find(CLOSE) else {
            stripped.push_str(&content[open..]);
            cursor = content.len();
            break;
        };
        let end = open + close_rel + CLOSE.len();
        blocks.push(content[open..end].to_string());
        cursor = end;
    }
    if cursor < content.len() {
        stripped.push_str(&content[cursor..]);
    }
    if blocks.is_empty() || stripped.trim().is_empty() {
        return;
    }
    let Some(target_index) = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .filter(|index| *index > 0)
    else {
        return;
    };
    let notice = blocks.join("\n");
    let merged = match messages[target_index].get_mut("content") {
        Some(Value::String(text)) => {
            *text = format!("{notice}\n\n{text}");
            true
        }
        Some(Value::Array(parts)) => {
            parts.insert(0, json!({ "type": "text", "text": notice }));
            true
        }
        _ => false,
    };
    if merged && let Some(Value::String(target)) = messages[0].get_mut("content") {
        *target = stripped.trim_end().to_string();
    }
}

fn canonicalize_system_text(text: &str) -> String {
    let mut ordinary = String::new();
    let mut blocks = Vec::<(String, String)>::new();
    let mut cursor = 0;
    while let Some(open_offset) = text[cursor..].find('<') {
        let open = cursor + open_offset;
        let Some(name_end) = text[open + 1..].find('>') else {
            ordinary.push_str(&text[cursor..]);
            cursor = text.len();
            break;
        };
        let name_end = open + 1 + name_end;
        let name = &text[open + 1..name_end];
        if name.is_empty()
            || !name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        {
            ordinary.push_str(&text[cursor..open + 1]);
            cursor = open + 1;
            continue;
        }
        let close = format!("</{name}>");
        let content_start = name_end + 1;
        let Some(close_offset) = text[content_start..].find(&close) else {
            ordinary.push_str(&text[cursor..]);
            cursor = text.len();
            break;
        };
        let end = content_start + close_offset + close.len();
        ordinary.push_str(&text[cursor..open]);
        blocks.push((name.to_string(), text[open..end].to_string()));
        cursor = end;
    }
    if cursor >= text.len() && blocks.is_empty() {
        return text.to_string();
    }
    if cursor < text.len() {
        ordinary.push_str(&text[cursor..]);
    }
    blocks.sort_by(|left, right| left.0.cmp(&right.0));
    let mut output = ordinary.trim().to_string();
    for (_, block) in blocks {
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(&block);
    }
    output
}

fn stabilize_chat_prompt_prefix(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    strip_image_resize_notice_blocks(messages);
    normalize_attachment_boilerplate(messages);
    for message in messages.iter_mut() {
        if message.get("role").and_then(Value::as_str) == Some("system") {
            let content = message
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(content) = content {
                *message.get_mut("content").unwrap() = json!(canonicalize_system_text(&content));
            }
        }
    }
    relocate_turn_aborted_blocks(messages);
}

pub fn chat_completion_to_response(body: Value) -> anyhow::Result<Value> {
    chat_completion_to_response_with_context(body, &CodexToolContext::default(), None)
}

pub fn chat_completion_to_response_with_request(
    body: Value,
    original_request: &Value,
) -> anyhow::Result<Value> {
    let context = chat_tool_projection(original_request).context;
    chat_completion_to_response_with_context(body, &context, Some(original_request))
}

fn chat_completion_to_response_with_context(
    body: Value,
    tool_context: &CodexToolContext,
    original_request: Option<&Value>,
) -> anyhow::Result<Value> {
    let choices = body
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("chat response missing choices"))?;
    let choice = choices
        .first()
        .ok_or_else(|| anyhow::anyhow!("chat response choices is empty"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| anyhow::anyhow!("chat response choice missing message"))?;

    let response_id = response_id_from_chat_id(body.get("id").and_then(Value::as_str));
    let mut output = Vec::new();
    if let Some(reasoning) = chat_reasoning_to_response_output_item(message, &response_id) {
        output.push(reasoning);
    }
    if let Some(message) = chat_message_to_response_output_item(message, &response_id) {
        output.push(message);
    }
    output.extend(chat_tool_calls_to_response_output_items(
        message,
        tool_context,
    ));

    let invalid_web_search = output.iter().any(invalid_web_search_output_item);
    let mut response = json!({
        "id": response_id,
        "object": "response",
        "created_at": body.get("created").and_then(Value::as_u64).unwrap_or(0),
        "status": if invalid_web_search {
            "failed"
        } else {
            response_status(choice.get("finish_reason").and_then(Value::as_str))
        },
        "model": body.get("model").and_then(Value::as_str).unwrap_or(""),
        "output": output,
        "usage": chat_usage_to_responses_usage(body.get("usage"))
    });

    if invalid_web_search {
        response["error"] = json!({
            "type": "invalid_upstream_tool_call",
            "message": "Chat 上游返回了未执行的原生 web_search 调用，请使用客户端搜索工具"
        });
    }

    if choice.get("finish_reason").and_then(Value::as_str) == Some("length") {
        response["incomplete_details"] = json!({ "reason": "max_output_tokens" });
    }
    copy_response_request_fields(&mut response, original_request);

    Ok(response)
}

pub struct ProxyHttpResponse {
    pub status: String,
    pub content_type: String,
    pub body: Vec<u8>,
}

pub struct UpstreamProxyResponse {
    pub status_code: u16,
    pub content_type: String,
    pub is_stream: bool,
    pub wire_api: UpstreamWireApi,
    /// 仅标记需要代理重组的合成摘要兼容路径。
    pub compaction: bool,
    /// 原生上游已返回恰好一个 compaction 项或完整失败流时为真，
    /// 调用方必须原样透传。
    pub native_compaction_passthrough: bool,
    /// 响应体已经读完。为 Some 时不要再读 response。
    pub buffered_body: Option<Vec<u8>>,
    /// 流式响应开头已被代理预读的字节。调用方必须先原样转发这段前缀，
    /// 再消费 `response` 的剩余部分；为空表示没有预读。
    pub stream_prefix: Vec<u8>,
    /// 代理内部的重试与路由痕迹；启动器用它写请求日志。
    pub request_trace: Option<crate::request_log::RequestTrace>,
    /// 上游请求是否使用了 Responses exec 兼容包装。
    pub exec_adapted: bool,
    pub(crate) exec_outcome: Option<crate::responses_exec_transport::ExecOutcome>,
    pub response: Option<reqwest::Response>,
}

/// 客户端会话头白名单；不接受鉴权或任意其他客户端请求头。
#[derive(Clone, Copy, Default)]
pub struct ProxySessionHeaders<'a> {
    pub session_id: Option<&'a str>,
    pub thread_id: Option<&'a str>,
    pub opencode_session: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum UpstreamWireApi {
    Responses,
    ChatCompletions,
    /// Anthropic Messages（`/v1/messages`），经 `anthropic_bridge` 转换。
    AnthropicMessages,
    AudioTranscriptions,
    ImageGenerations,
    ImageEdits,
}

#[derive(Debug, Clone)]
struct ModelRouteSelection {
    relay: crate::settings::RelayProfile,
    source_relay_id: String,
    source_model: String,
    upstream_model: String,
}

impl UpstreamProxyResponse {
    pub fn status(&self) -> String {
        http_status_line(self.status_code)
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status_code)
    }

    pub async fn read_body(&mut self) -> anyhow::Result<Vec<u8>> {
        let prefix = std::mem::take(&mut self.stream_prefix);
        if let Some(body) = &self.buffered_body {
            if prefix.is_empty() {
                return Ok(body.clone());
            }
            let mut bytes = prefix;
            bytes.extend_from_slice(body);
            return Ok(bytes);
        }
        let response = self.response.take().context("缺少上游响应")?;
        let mut bytes = prefix;
        bytes.extend_from_slice(&response.bytes().await?);
        Ok(bytes)
    }

    pub fn take_response(&mut self) -> anyhow::Result<reqwest::Response> {
        self.response.take().context("缺少上游响应")
    }
}

pub fn upstream_header_timeout() -> Duration {
    UPSTREAM_HEADER_TIMEOUT
}

pub fn upstream_stream_header_timeout() -> Duration {
    UPSTREAM_STREAM_HEADER_TIMEOUT
}

pub fn upstream_http_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(UPSTREAM_CONNECT_TIMEOUT)
        .user_agent("CodexPlusPlus/ProtocolProxy")
        .build()
        .context("failed to build upstream HTTP client")
}

pub async fn send_upstream_request(
    request: reqwest::RequestBuilder,
) -> anyhow::Result<reqwest::Response> {
    send_upstream_request_with_header_timeout(request, UPSTREAM_HEADER_TIMEOUT).await
}

pub async fn send_upstream_request_for_responses(
    request: reqwest::RequestBuilder,
    is_stream: bool,
) -> anyhow::Result<reqwest::Response> {
    let timeout = response_header_timeout(is_stream);
    send_upstream_request_with_header_timeout(request, timeout).await
}

pub async fn send_upstream_request_with_header_timeout(
    request: reqwest::RequestBuilder,
    timeout: Duration,
) -> anyhow::Result<reqwest::Response> {
    // RequestBuilder 只有在 body 可克隆时才能重试；不可克隆时保持单次请求。
    let retry_source = request.try_clone();
    let mut pending = Some(request);
    let mut retries = 0_usize;
    loop {
        let builder = pending.take().expect("upstream request builder missing");
        match tokio::time::timeout(timeout, builder.send()).await {
            Ok(Ok(response)) => return Ok(response),
            Ok(Err(error)) => {
                // 只重试连接/TLS 握手失败：此时请求体尚未送达上游，重试是幂等的。
                // 超时和其它错误可能已经在上游产生副作用，原样返回。
                let retry_delay = error
                    .is_connect()
                    .then(|| UPSTREAM_CONNECT_RETRY_DELAYS.get(retries))
                    .flatten()
                    .copied();
                if let Some(delay) = retry_delay
                    && let Some(source) = retry_source.as_ref()
                    && let Some(retry) = source.try_clone()
                {
                    let retry_error = anyhow::Error::new(error);
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "protocol_proxy.upstream_connect_retry",
                        json!({
                            "attempt": retries + 1,
                            "delayMs": delay.as_millis(),
                            "error": error_chain(&retry_error)
                        }),
                    );
                    tokio::time::sleep(delay).await;
                    retries += 1;
                    pending = Some(retry);
                    continue;
                }
                return Err(anyhow::Error::new(error).context("上游请求失败"));
            }
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "上游请求超过 {} 秒未返回响应头",
                    timeout.as_secs()
                ));
            }
        }
    }
}

/// anyhow 的 `Display` 只打印最外层上下文；诊断日志和 502 响应体需要完整错误链
/// （连接被拒/重置、超时、TLS、代理错误等真实原因），否则只能看到
/// 「请求上游失败」这种没有任何可操作信息的文案。
pub(crate) fn error_chain(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

#[cfg(test)]
mod error_chain_tests {
    use super::error_chain;

    #[test]
    fn error_chain_keeps_every_cause_for_diagnostics() {
        let error = anyhow::anyhow!("连接被对端重置")
            .context("上游请求失败")
            .context("供应商「供应商 4」请求上游失败");
        let text = error_chain(&error);
        assert!(text.contains("供应商「供应商 4」请求上游失败"), "{text}");
        assert!(text.contains("上游请求失败"), "{text}");
        assert!(text.contains("连接被对端重置"), "{text}");
    }
}

pub struct ChatSseToResponsesConverter {
    buffer: String,
    utf8_remainder: Vec<u8>,
    state: ChatSseState,
    failed: bool,
}

/// 原生 Responses 只旁路观察终止事件，不改写任何响应字节，也不保存诊断正文。
/// 只保留每个事件的有限前缀，避免图片等大 payload 令诊断缓冲无界增长。
#[derive(Default)]
pub(crate) struct NativeResponsesSseObserver {
    event_prefix: Vec<u8>,
    event_tail: [u8; 4],
    event_too_large: bool,
    unclassified_large_event: bool,
    completed: bool,
    failure: Option<&'static str>,
}

const NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT: usize = 64 * 1024;

impl NativeResponsesSseObserver {
    pub(crate) fn push_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if self.event_prefix.len() < NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT {
                self.event_prefix.push(*byte);
            } else {
                self.event_too_large = true;
            }
            self.event_tail.rotate_left(1);
            self.event_tail[3] = *byte;
            if self.event_tail.ends_with(b"\n\n") || self.event_tail == *b"\r\n\r\n" {
                self.observe_event();
                self.event_prefix.clear();
                self.event_tail = [0; 4];
                self.event_too_large = false;
            }
        }
    }

    fn observe_event(&mut self) {
        let prefix = String::from_utf8_lossy(&self.event_prefix);
        let event_name = prefix
            .lines()
            .find_map(|line| strip_sse_field(line, "event"));
        let data = prefix
            .lines()
            .filter_map(|line| strip_sse_field(line, "data"))
            .collect::<Vec<_>>()
            .join("\n");
        // 只观察已完整解析的顶层 type；后续大 payload 超过前缀上限时，
        // 忽略其解析错误，保留此前读到的类型。它不是原生响应的有效性校验器。
        struct EventType<'a>(&'a mut Option<String>);
        impl<'de> serde::de::Visitor<'de> for EventType<'_> {
            type Value = ();
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an SSE event object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                while let Some(key) = map.next_key::<String>()? {
                    if key == "type" {
                        *self.0 = Some(map.next_value::<String>()?);
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                Ok(())
            }
        }
        let mut deserializer = serde_json::Deserializer::from_str(&data);
        let mut event_type = None;
        let _ = serde::Deserializer::deserialize_map(&mut deserializer, EventType(&mut event_type));
        match event_type.as_deref().or(event_name.map(str::trim)) {
            Some("response.completed") => self.completed = true,
            Some("response.failed") => {
                self.failure = Some("原生 Responses 上游返回 response.failed")
            }
            Some("response.incomplete") => {
                self.failure = Some("原生 Responses 上游返回 response.incomplete")
            }
            Some("error") => self.failure = Some("原生 Responses 上游返回 error 事件"),
            None if self.event_too_large => self.unclassified_large_event = true,
            _ => {}
        }
    }

    pub(crate) fn finish(self) -> Option<String> {
        // failed/incomplete 优先于 completed；[DONE] 或 HTTP EOF 都不能证明成功。
        self.failure
            .or_else(|| {
                if self.completed {
                    None
                } else if self.unclassified_large_event {
                    Some("原生 Responses 流终止状态未知（诊断事件超限），未观察到完成事件")
                } else {
                    Some("原生 Responses 上游在完成事件前结束了响应流")
                }
            })
            .map(str::to_string)
    }
}

#[cfg(test)]
mod native_responses_observer_tests {
    use super::{NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT, NativeResponsesSseObserver};

    #[test]
    fn fragmented_crlf_and_data_only_terminal_events_are_observed() {
        for payload in [
            "event: response.completed\r\ndata: {\"type\":\"response.completed\",\"text\":\"摘要\"}\r\n\r\n",
            "data: {\"response\":{},\"type\":\"response.completed\"}\n\n",
        ] {
            let mut observer = NativeResponsesSseObserver::default();
            for byte in payload.as_bytes().chunks(1) {
                observer.push_bytes(byte);
            }
            assert!(observer.finish().is_none());
        }
    }

    #[test]
    fn large_payloads_keep_diagnostic_memory_bounded_and_allow_terminal_observation() {
        for prefix in [
            "event: response.completed\ndata: {\"output\":\"",
            "data: {\"type\":\"response.completed\",\"output\":\"",
        ] {
            let mut observer = NativeResponsesSseObserver::default();
            observer.push_bytes(prefix.as_bytes());
            observer.push_bytes(&vec![b'x'; NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT * 2]);
            assert!(observer.event_prefix.len() <= NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT);
            observer.push_bytes(b"\"}\n\n");
            assert!(observer.finish().is_none());
        }
    }

    #[test]
    fn failed_event_after_completed_still_reports_fixed_failure_without_payload() {
        let mut observer = NativeResponsesSseObserver::default();
        observer.push_bytes(b"data: {\"type\":\"response.completed\"}\n\ndata: {\"type\":\"response.failed\",\"error\":\"private-payload\"}\n\n");
        assert_eq!(
            observer.finish().as_deref(),
            Some("原生 Responses 上游返回 response.failed")
        );
    }

    #[test]
    fn unclassified_large_event_reports_unknown_state_without_payload() {
        let mut observer = NativeResponsesSseObserver::default();
        observer.push_bytes(b"data: {\"output\":\"");
        observer.push_bytes(&vec![b'x'; NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT * 2]);
        observer.push_bytes(b"\",\"type\":\"response.completed\"}\n\n");
        assert!(observer.finish().unwrap().contains("状态未知"));
    }
}

/// codex v2 远程压缩的响应包装器：把上游摘要文本（无论 Responses 还是
/// Chat 上游、流式还是非流式）封装成「恰好一个 `compaction` 输出项」的
/// Responses SSE 流。输出项通过 output_item.done 显式交付给客户端，
/// 不能只放进 response.completed.response.output。
pub struct CompactionSseConverter {
    response_id: String,
    compaction_id: String,
    model: String,
    summary: String,
    final_summary: Option<String>,
    failed: Option<(String, Option<String>)>,
    /// 跨网络 chunk 攒 SSE 事件的缓冲（见 push_upstream_bytes）。
    sse_buffer: String,
    sse_utf8_remainder: Vec<u8>,
    responses_wire: bool,
    completed: bool,
}

pub struct CompactionSseOutcome {
    pub payload: Vec<u8>,
    pub succeeded: bool,
}

impl CompactionSseConverter {
    pub fn new(model: &str) -> Self {
        Self {
            response_id: format!("resp_compact_{}", chrono_now_millis()),
            compaction_id: format!("cmp_{}", uuid::Uuid::new_v4().simple()),
            model: model.to_string(),
            summary: String::new(),
            final_summary: None,
            failed: None,
            sse_buffer: String::new(),
            sse_utf8_remainder: Vec::new(),
            responses_wire: true,
            completed: false,
        }
    }

    /// 标记上游是 Chat Completions（SSE chunk 的增量在 `choices[].delta.content`）。
    /// Responses 上游默认，增量在 `response.output_text.delta` 事件里。
    pub fn with_chat_upstream(mut self) -> Self {
        self.responses_wire = false;
        self
    }

    /// 追加上游输出的一块文本内容（非流式路径直接喂完整摘要）。
    pub fn push_summary_text(&mut self, text: &str) {
        self.summary.push_str(text);
    }

    /// 当前已收集的原始摘要文本（剥 think 之前），供空摘要判定。
    pub fn summary_text(&self) -> &str {
        &self.summary
    }

    /// 喂入上游流式响应的一个网络 chunk。SSE 事件可能被 TCP 拆开，
    /// 内部按 `\n\n` 边界缓冲；残缺块留在缓冲区等下一个 chunk。
    pub fn push_upstream_bytes(&mut self, bytes: &[u8]) {
        let mut utf8 = self.sse_utf8_remainder.clone();
        utf8.extend_from_slice(bytes);
        if std::str::from_utf8(&utf8).is_err_and(|error| error.error_len().is_some()) {
            self.fail("压缩响应包含无效 UTF-8".to_string(), Some("compaction_invalid_stream".to_string()));
            return;
        }
        append_utf8_safe(&mut self.sse_buffer, &mut self.sse_utf8_remainder, bytes);
        while let Some(block) = take_sse_block(&mut self.sse_buffer) {
            if block.trim().is_empty() {
                continue;
            }
            self.handle_upstream_sse_block(&block);
        }
    }

    fn handle_upstream_sse_block(&mut self, block: &str) {
        let mut event_name = "";
        let mut data_parts: Vec<&str> = Vec::new();
        for line in block.lines() {
            if let Some(event) = strip_sse_field(line, "event") {
                event_name = event.trim();
            }
            if let Some(data) = strip_sse_field(line, "data") {
                data_parts.push(data);
            }
        }
        if data_parts.is_empty() {
            return;
        }
        let data = data_parts.join("\n");
        if data.trim() == "[DONE]" {
            return;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(&data) else {
            self.fail("压缩响应包含无效 SSE JSON".to_string(), Some("compaction_invalid_stream".to_string()));
            return;
        };
        let event_type = chunk
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event_name);
        if matches!(
            event_type,
            "response.failed" | "response.incomplete" | "error"
        ) || chunk.get("error").is_some_and(|error| !error.is_null())
            || chunk.pointer("/response/error").is_some_and(|error| !error.is_null())
            || matches!(chunk.pointer("/response/status").and_then(Value::as_str), Some("failed" | "incomplete"))
        {
            let error = chunk
                .pointer("/response/error")
                .or_else(|| chunk.get("error"));
            self.fail(
                error
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Compaction upstream stream failed or was incomplete")
                    .to_string(),
                error
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            );
            return;
        }
        if self.responses_wire {
            match event_type {
                // Responses 流优先读取正文增量，排除携带同名 delta 字段的推理事件。
                "response.output_text.delta" => {
                    if let Some(delta) = chunk.get("delta").and_then(Value::as_str) {
                        self.summary.push_str(delta);
                    }
                }
                // 部分兼容实现不发送 delta，只在完成事件里携带完整正文。
                "response.output_text.done" => {
                    self.capture_final_summary(chunk.get("text"));
                }
                "response.content_part.done" => {
                    self.capture_final_summary(chunk.pointer("/part/text"));
                }
                "response.output_item.done" => {
                    let text = chunk
                        .get("item")
                        .map(extract_summary_text_from_response_item)
                        .unwrap_or_default();
                    self.capture_final_summary(Some(&json!(text)));
                }
                "response.completed" => {
                    self.completed = true;
                    let text = chunk
                        .get("response")
                        .map(extract_summary_text_from_responses)
                        .unwrap_or_default();
                    self.capture_final_summary(Some(&json!(text)));
                }
                _ => {}
            }
        } else if let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        {
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                if reason == "stop" {
                    self.completed = true;
                } else {
                    self.fail("压缩摘要没有正常完成".to_string(), Some("compaction_incomplete".to_string()));
                }
            }
            if let Some(content) = choice
                .get("delta")
                .and_then(|delta| delta.get("content"))
                .and_then(Value::as_str)
            {
                self.summary.push_str(content);
            }
            self.capture_final_summary(
                choice
                    .get("message")
                    .and_then(|message| message.get("content")),
            );
        }
    }

    fn capture_final_summary(&mut self, text: Option<&Value>) {
        let Some(text) = text
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        else {
            return;
        };
        self.final_summary = Some(text.to_string());
    }

    pub fn fail(&mut self, message: String, error_type: Option<String>) -> Vec<u8> {
        self.failed = Some((message, error_type));
        Vec::new()
    }

    /// 收尾：产出完整的 compaction 响应 SSE。
    /// 链式推理上游（DeepSeek thinking 等）会把推理过程以 `<think>` 块
    /// 混进正文，这里统一剥掉首部完整 think 块，只保留真正的摘要答案；
    /// think 块未闭合（上游截断）时整个丢弃——剩下的只有推理残片。
    pub fn finish(self) -> Vec<u8> {
        self.finish_with_outcome().payload
    }

    pub fn finish_with_outcome(mut self) -> CompactionSseOutcome {
        if let Some(final_summary) = self.final_summary.take() {
            self.summary = final_summary;
        }
        if let Some((_reasoning, answer)) = split_leading_think_block(&self.summary) {
            self.summary = answer;
        } else if self.summary.trim_start().starts_with(THINK_OPEN_TAG) {
            self.summary = String::new();
        }
        self.summary = self.summary.trim().to_string();
        // 剥掉 think 块后为空（上游空输出、纯推理文本、截断导致块未闭合），
        // 一律按失败返回，避免向 codex 交付 `completed + 空 encrypted_content`
        // 的空 checkpoint 静默清空会话。
        if self.failed.is_none() && self.summary.is_empty() {
            self.failed = Some((
                "上游返回了空摘要，无法完成压缩".to_string(),
                Some("compaction_empty_summary".to_string()),
            ));
        }
        if self.failed.is_none() && (!self.completed || !self.sse_utf8_remainder.is_empty()
            || self.sse_buffer.lines().any(|line| !line.trim().is_empty() && !line.starts_with(':')))
        {
            self.failed = Some(("上游压缩响应未完整结束，未生成 checkpoint".to_string(), Some("compaction_incomplete".to_string())));
        }
        let mut output = String::new();
        let mut response = json!({
            "id": self.response_id,
            "object": "response",
            "created_at": chrono_now_millis() / 1000,
            "status": "in_progress",
            "model": self.model,
            "output": [],
            "usage": default_responses_usage()
        });
        push_sse(
            &mut output,
            "response.created",
            json!({"type": "response.created", "sequence_number": 0, "response": response}),
        );
        if let Some((message, error_type)) = self.failed {
            response["status"] = json!("failed");
            response["error"] = json!({
                "code": error_type.unwrap_or_else(|| "compaction_failed".to_string()),
                "message": message
            });
            push_sse(
                &mut output,
                "response.failed",
                json!({"type": "response.failed", "sequence_number": 1, "response": response}),
            );
            output.push_str("data: [DONE]\n\n");
            return CompactionSseOutcome { payload: output.into_bytes(), succeeded: false };
        }
        let compaction_item = json!({
            "id": self.compaction_id,
            "type": COMPACTION_OUTPUT_TYPE,
            "encrypted_content": format!("{LOCAL_COMPACTION_PREFIX}{}", json!(self.summary))
        });
        for (sequence_number, event_type) in [
            (1, "response.output_item.added"),
            (2, "response.output_item.done"),
        ] {
            push_sse(
                &mut output,
                event_type,
                json!({
                    "type": event_type,
                    "sequence_number": sequence_number,
                    "output_index": 0,
                    "item": compaction_item
                }),
            );
        }
        response["status"] = json!("completed");
        response["output"] = json!([compaction_item]);
        push_sse(
            &mut output,
            "response.completed",
            json!({
                "type": "response.completed",
                "sequence_number": 3,
                "response": response
            }),
        );
        output.push_str("data: [DONE]\n\n");
        CompactionSseOutcome { payload: output.into_bytes(), succeeded: true }
    }
}

fn chrono_now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 非流式压缩响应包装：从上游 JSON 响应里提取 assistant 文本并封装成
/// 恰好一个 `compaction` 输出项的 Responses 响应。
pub fn wrap_non_stream_response_as_compaction(
    upstream_body: &[u8],
    model: &str,
) -> anyhow::Result<Vec<u8>> {
    let upstream_json: Value = serde_json::from_slice(upstream_body)?;
    let mut converter = CompactionSseConverter::new(model);
    let wire = if upstream_json.get("choices").is_some() { UpstreamWireApi::ChatCompletions } else { UpstreamWireApi::Responses };
    if let Some(error) = upstream_json.get("error").filter(|value| !value.is_null()) {
        converter.fail(
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("compaction upstream error")
                .to_string(),
            None,
        );
        return Ok(converter.finish());
    }
    let responses_text = extract_summary_text_from_responses(&upstream_json);
    let text = if responses_text.is_empty() {
        extract_summary_text_from_chat(&upstream_json)
    } else {
        responses_text
    };
    if text.is_empty() {
        converter.fail(
            "上游返回了空摘要，无法完成压缩".to_string(),
            Some("compaction_empty_summary".to_string()),
        );
        return Ok(converter.finish());
    }
    if validate_compaction_json_completion(&upstream_json, wire).is_err() {
        converter.fail("上游压缩摘要未正常完成".to_string(), Some("compaction_incomplete".to_string()));
        return Ok(converter.finish());
    }
    converter.completed = true;
    converter.push_summary_text(&text);
    Ok(converter.finish())
}

pub(crate) fn validate_compaction_json_completion(
    body: &Value,
    wire: UpstreamWireApi,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        body.get("error").is_none_or(Value::is_null),
        "压缩上游返回错误"
    );
    let complete = match wire {
        UpstreamWireApi::Responses => {
            body.get("status").and_then(Value::as_str) == Some("completed")
        }
        UpstreamWireApi::ChatCompletions => {
            body.pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                == Some("stop")
        }
        UpstreamWireApi::AnthropicMessages => matches!(
            body.get("stop_reason").and_then(Value::as_str),
            Some("end_turn" | "stop_sequence")
        ),
        _ => false,
    };
    anyhow::ensure!(complete, "压缩上游没有正常完成状态");
    Ok(())
}
/// 从 Responses JSON 响应（`output[].content[].text`）提取 assistant 文本。
fn extract_summary_text_from_responses(response: &Value) -> String {
    let Some(items) = response.get("output").and_then(Value::as_array) else {
        return String::new();
    };
    let mut texts = Vec::new();
    for item in items {
        let text = extract_summary_text_from_response_item(item);
        if !text.is_empty() {
            texts.push(text);
        }
    }
    texts.join("\n")
}

fn extract_summary_text_from_response_item(item: &Value) -> String {
    match item.get("type").and_then(Value::as_str) {
        Some("message") => item
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 从 Chat Completions JSON 响应提取 assistant 文本。
fn extract_summary_text_from_chat(response: &Value) -> String {
    response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn response_body_is_stream(content_type: &str, body: &[u8]) -> bool {
    if content_type.contains("text/event-stream") {
        return true;
    }
    let trimmed = body
        .iter()
        .copied()
        .skip_while(|byte| byte.is_ascii_whitespace())
        .take(6)
        .collect::<Vec<_>>();
    trimmed.starts_with(b"data:") || trimmed.starts_with(b"event:")
}

fn output_compaction_count(output: Option<&Value>) -> Option<usize> {
    let items = output?.as_array()?;
    let compactions = items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some(COMPACTION_OUTPUT_TYPE));
    if compactions
        .clone()
        .any(|item| usable_compaction_content(item).is_none())
    {
        return Some(0);
    }
    Some(compactions.count())
}

async fn read_limited_upstream_body(
    response: &mut reqwest::Response,
    limit: usize,
) -> anyhow::Result<Vec<u8>> {
    if let Some(content_length) = response.content_length()
        && content_length > limit as u64
    {
        anyhow::bail!("上游响应体超过 {limit} 字节限制");
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > limit {
            anyhow::bail!("上游响应体超过 {limit} 字节限制");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn usable_compaction_content(item: &Value) -> Option<&str> {
    item.get("encrypted_content")?
        .as_str()
        .filter(|content| !content.trim().is_empty())
}

fn json_compaction_count(value: &Value) -> Option<usize> {
    if let Some(count) = output_compaction_count(value.get("output")) {
        return Some(count);
    }
    value
        .get("response")
        .and_then(|response| output_compaction_count(response.get("output")))
}

/// 按 Codex 实际消费的 SSE item-done 事件统计原生压缩项。
/// 非流式调用仍兼容 JSON output，但流式请求不能把 JSON 当作原生 v2 透传。
fn upstream_compaction_item_count(body: &[u8], content_type: &str) -> Option<usize> {
    if response_body_is_stream(content_type, body) {
        let text = std::str::from_utf8(body).ok()?.replace("\r\n", "\n");
        let mut count = 0;
        let mut completed = false;
        let tail = text.rsplit("\n\n").next().unwrap_or("");
        if tail.lines().any(|line| !line.trim().is_empty() && !line.starts_with(':')) { return None; }
        for block in text.split("\n\n") {
            let mut event_name = None;
            let mut data_parts = Vec::new();
            for line in block.lines() {
                if let Some(event) = strip_sse_field(line, "event") {
                    event_name = Some(event.trim());
                }
                if let Some(data) = strip_sse_field(line.trim_end_matches('\r'), "data") {
                    data_parts.push(data);
                }
            }
            if data_parts.is_empty() {
                continue;
            }
            let data = data_parts.join("\n");
            if data.trim() == "[DONE]" {
                continue;
            }
            let json = serde_json::from_str::<Value>(&data).ok()?;
            let event = event_name.or_else(|| json.get("type").and_then(Value::as_str));
            if completed { return None; }
            match event {
                Some("response.output_item.done")
                    if json
                        .get("item")
                        .and_then(|item| item.get("type"))
                        .and_then(Value::as_str)
                        == Some(COMPACTION_OUTPUT_TYPE) =>
                {
                    if usable_compaction_content(json.get("item")?).is_none() {
                        return Some(0);
                    }
                    count += 1;
                }
                Some("response.completed") => {
                    json.get("response")?.get("id")?.as_str()?;
                    if matches!(json.pointer("/response/status").and_then(Value::as_str), Some("failed" | "incomplete")) { return None; }
                    completed = true;
                }
                Some("response.failed" | "response.incomplete" | "error") => return None,
                _ => {}
            }
        }
        return completed.then_some(count);
    }
    let Ok(json) = serde_json::from_slice::<Value>(body) else {
        return None;
    };
    if json.get("error").is_some_and(|error| !error.is_null())
        || json.get("status").and_then(Value::as_str).is_some_and(|status| status != "completed")
    { return None; }
    json_compaction_count(&json)
}

fn compaction_input_rejected(status_code: u16, body: &[u8]) -> bool {
    if status_code != 400 && status_code != 422 {
        return false;
    }
    // 只读取错误本身，避免回显请求中的 trigger 让无关参数错误触发降级。
    let text = match serde_json::from_slice::<Value>(body) {
        Ok(json) => {
            let error = json.get("error").unwrap_or(&json);
            [
                error.as_str(),
                error.get("message").and_then(Value::as_str),
                error.get("code").and_then(Value::as_str),
                error.get("param").and_then(Value::as_str),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
        }
        Err(_) => String::from_utf8_lossy(body).into_owned(),
    }
    .to_ascii_lowercase();
    let unrecognized = text.contains("unknown")
        || text.contains("unrecognized")
        || text.contains("unsupported")
        || text.contains("unexpected")
        || text.contains("not supported")
        || text.contains("invalid type")
        || text.contains("invalid_type")
        || text.contains("additional propert");
    text.contains(COMPACTION_TRIGGER_TYPE) && unrecognized
}

fn sse_block_type(block: &str) -> Option<String> {
    let mut event_name = None;
    let mut data_parts = Vec::new();
    for line in block.lines() {
        if let Some(event) = strip_sse_field(line, "event") {
            event_name = Some(event.trim().to_string());
        }
        if let Some(data) = strip_sse_field(line.trim_end_matches('\r'), "data") {
            data_parts.push(data);
        }
    }
    if let Some(name) = event_name.filter(|name| !name.is_empty()) {
        return Some(name);
    }
    let data = data_parts.join("\n");
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    serde_json::from_str::<Value>(data).ok().and_then(|json| {
        json.get("type")
            .and_then(Value::as_str)
            .map(ToString::to_string)
    })
}

fn upstream_has_terminal_failure(body: &[u8], content_type: &str) -> bool {
    if response_body_is_stream(content_type, body) {
        let text = String::from_utf8_lossy(body).replace("\r\n", "\n");
        return text.split("\n\n").any(|block| {
            matches!(
                sse_block_type(block).as_deref(),
                Some("response.failed" | "response.incomplete" | "error")
            )
        });
    }
    serde_json::from_slice::<Value>(body).is_ok_and(|json| {
        json.get("error").is_some_and(|error| !error.is_null())
            || matches!(
                json.get("status").and_then(Value::as_str),
                Some("failed" | "incomplete")
            )
    })
}

fn render_native_compaction_as_sse(body: &[u8]) -> Option<Vec<u8>> {
    let json = serde_json::from_slice::<Value>(body).ok()?;
    let response = if json.get("output").is_some() {
        json
    } else {
        json.get("response")?.clone()
    };
    let output = response.get("output")?.as_array()?;
    let mut compaction_items = output.iter().filter(|item| {
        item.get("type").and_then(Value::as_str) == Some(COMPACTION_OUTPUT_TYPE)
            && usable_compaction_content(item).is_some()
    });
    let item = compaction_items.next()?.clone();
    if compaction_items.next().is_some() {
        return None;
    }
    let mut sse = String::new();
    let mut created_response = response.clone();
    created_response["status"] = json!("in_progress");
    created_response["output"] = json!([]);
    push_sse(
        &mut sse,
        "response.created",
        json!({
            "type": "response.created",
            "sequence_number": 0,
            "response": created_response
        }),
    );
    push_sse(
        &mut sse,
        "response.output_item.added",
        json!({
            "type": "response.output_item.added",
            "sequence_number": 1,
            "output_index": 0,
            "item": item
        }),
    );
    push_sse(
        &mut sse,
        "response.output_item.done",
        json!({
            "type": "response.output_item.done",
            "sequence_number": 2,
            "output_index": 0,
            "item": item
        }),
    );
    push_sse(
        &mut sse,
        "response.completed",
        json!({
            "type": "response.completed",
            "sequence_number": 3,
            "response": response
        }),
    );
    sse.push_str("data: [DONE]\n\n");
    Some(sse.into_bytes())
}

fn incomplete_compaction_stream(_body: &[u8]) -> Vec<u8> {
    // 验收未完成时绝不能先交付部分 checkpoint，再追加一个失败事件。
    let mut output = String::new();
    push_sse(
        &mut output,
        "response.failed",
        json!({
            "type": "response.failed",
            "response": {
                "id": "resp_compact_incomplete",
                "status": "failed",
                "error": {
                    "message": "remote compaction v2 stream closed before response.completed",
                    "type": "stream_error"
                }
            }
        }),
    );
    output.into_bytes()
}

#[derive(Debug)]
struct NativeCompactionUnsupported {
    message: &'static str,
}

impl NativeCompactionUnsupported {
    fn history() -> Self {
        Self {
            message: "This compaction checkpoint cannot be converted to a text summary. Use a compatible Responses upstream or start a new conversation.",
        }
    }
}

impl std::fmt::Display for NativeCompactionUnsupported {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for NativeCompactionUnsupported {}

fn native_compaction_unsupported_result(
    error: anyhow::Error,
) -> anyhow::Result<UpstreamProxyResponse> {
    if error
        .downcast_ref::<NativeCompactionUnsupported>()
        .is_some()
    {
        let body = json!({
            "error": {
                "message": error.to_string(),
                "type": "compaction_incompatible",
                "code": "native_compaction_unsupported"
            }
        });
        Ok(UpstreamProxyResponse {
            status_code: 400,
            content_type: "application/json".to_string(),
            is_stream: false,
            wire_api: UpstreamWireApi::Responses,
            compaction: false,
            native_compaction_passthrough: false,
            buffered_body: Some(serde_json::to_vec(&body).unwrap_or_default()),
            stream_prefix: Vec::new(),
            request_trace: None,
            exec_adapted: false,
            exec_outcome: None,
            response: None,
        })
    } else {
        Err(error)
    }
}

fn input_has_native_compaction(body: &Value) -> bool {
    let is_native =
        |item: &Value| item.get("type").and_then(Value::as_str) == Some(COMPACTION_OUTPUT_TYPE);
    match body.get("input") {
        Some(Value::Array(items)) => items.iter().any(is_native),
        Some(Value::Object(_)) => is_native(&body["input"]),
        _ => false,
    }
}

/// 只展开带版本标识、可解码的本地摘要；未知/旧格式仍视为原生状态。
fn expand_compaction_item(item: &Value) -> Option<Value> {
    if item.get("type").and_then(Value::as_str) != Some(COMPACTION_OUTPUT_TYPE) {
        return None;
    }
    let encoded = item
        .get("encrypted_content")?
        .as_str()?
        .strip_prefix(LOCAL_COMPACTION_PREFIX)?;
    let summary: String = serde_json::from_str(encoded).ok()?;
    if summary.trim().is_empty() {
        return None;
    }
    let mut text = COMPACTION_REPLAY_PREFIX.to_string();
    text.push_str(&summary);
    Some(json!({
        "type": "message",
        "role": "user",
        "content": [{ "type": "input_text", "text": text }]
    }))
}

fn expand_local_compaction_history(body: &mut Value) {
    match body.get_mut("input") {
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(message) = expand_compaction_item(item) {
                    *item = message;
                }
            }
        }
        Some(input @ Value::Object(_)) => {
            if let Some(message) = expand_compaction_item(input) {
                *input = json!([message]);
            }
        }
        _ => {}
    }
}

impl Default for ChatSseToResponsesConverter {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            utf8_remainder: Vec::new(),
            state: ChatSseState::default(),
            failed: false,
        }
    }
}

impl ChatSseToResponsesConverter {
    pub fn has_failed(&self) -> bool {
        self.failed
    }

    pub fn has_terminal_event(&self) -> bool {
        self.failed || self.state.completed || self.state.finish_reason.is_some()
    }

    pub fn with_request(original_request: &Value) -> Self {
        Self {
            state: ChatSseState::with_request(original_request),
            ..Self::default()
        }
    }

    pub(crate) fn model(&self) -> &str {
        &self.state.model
    }

    pub(crate) fn latest_usage(&self) -> Option<&Value> {
        self.state.latest_usage.as_ref()
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        append_utf8_safe(&mut self.buffer, &mut self.utf8_remainder, bytes);
        let mut output = String::new();
        while let Some(block) = take_sse_block(&mut self.buffer) {
            if block.trim().is_empty() {
                continue;
            }
            self.handle_block(&block, &mut output);
            if self.failed {
                break;
            }
        }
        output.into_bytes()
    }

    pub fn finish(&mut self) -> Vec<u8> {
        if !self.utf8_remainder.is_empty() {
            self.buffer
                .push_str(&String::from_utf8_lossy(&self.utf8_remainder));
            self.utf8_remainder.clear();
        }

        let mut output = String::new();
        if !self.failed {
            self.state.finalize_into(&mut output);
        }
        output.into_bytes()
    }

    pub fn fail(&mut self, message: String, error_type: Option<String>) -> Vec<u8> {
        let mut output = String::new();
        self.state.failed_into(&mut output, message, error_type);
        self.failed = true;
        output.into_bytes()
    }

    fn handle_block(&mut self, block: &str, output: &mut String) {
        let mut event_name: Option<String> = None;
        let mut data_parts = Vec::new();
        for line in block.lines() {
            if let Some(event) = strip_sse_field(line, "event") {
                event_name = Some(event.trim().to_string());
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
            self.state.finalize_into(output);
            return;
        }

        let Ok(chunk) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        if event_name.as_deref() == Some("error") || chunk.get("error").is_some() {
            let (message, error_type) = extract_chat_sse_error(&chunk);
            self.state.failed_into(output, message, error_type);
            self.failed = true;
            return;
        }
        self.state.handle_chat_chunk_into(&chunk, output);
    }
}

pub fn is_responses_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/responses"
            | "/v1/responses"
            | "/v1/v1/responses"
            | "/codex/v1/responses"
            | "/responses/compact"
            | "/v1/responses/compact"
            | "/v1/v1/responses/compact"
            | "/codex/v1/responses/compact"
    )
}

pub fn is_responses_compact_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/responses/compact"
            | "/v1/responses/compact"
            | "/v1/v1/responses/compact"
            | "/codex/v1/responses/compact"
    )
}

pub fn is_chat_completions_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/chat/completions"
            | "/v1/chat/completions"
            | "/v1/v1/chat/completions"
            | "/codex/v1/chat/completions"
    )
}

pub fn is_models_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/models" | "/v1/models" | "/v1/v1/models" | "/codex/v1/models"
    )
}

pub fn is_audio_transcriptions_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/audio/transcriptions"
            | "/v1/audio/transcriptions"
            | "/v1/v1/audio/transcriptions"
            | "/codex/v1/audio/transcriptions"
    )
}

pub fn is_image_generations_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/images/generations"
            | "/v1/images/generations"
            | "/v1/v1/images/generations"
            | "/codex/v1/images/generations"
    )
}

pub fn is_image_edits_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/images/edits" | "/v1/images/edits" | "/v1/v1/images/edits" | "/codex/v1/images/edits"
    )
}

pub async fn open_responses_proxy_request(
    body: &str,
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_for_path(body, original_user_agent, "/responses").await
}

pub async fn open_responses_proxy_request_for_path(
    body: &str,
    original_user_agent: Option<&str>,
    request_path: &str,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_for_path_with_beta(body, original_user_agent, request_path, None)
        .await
}

/// 只转发客户端声明的 beta 能力，不转发客户端鉴权。
pub async fn open_responses_proxy_request_for_path_with_beta(
    body: &str,
    original_user_agent: Option<&str>,
    request_path: &str,
    beta_features: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_for_path_with_session_headers(
        body,
        original_user_agent,
        request_path,
        beta_features,
        ProxySessionHeaders::default(),
    )
    .await
}

pub async fn open_responses_proxy_request_for_path_with_session_headers(
    body: &str,
    original_user_agent: Option<&str>,
    request_path: &str,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    open_responses_proxy_request_with_settings_and_user_agent(
        body,
        settings,
        original_user_agent,
        request_path,
        beta_features,
        session_headers,
        None,
    )
    .await
}

/// 与 `open_responses_proxy_request_for_path_with_session_headers` 相同，
/// 额外带上请求日志 ID，用于在整段缓冲时实时更新输出 token 估算。
pub async fn open_responses_proxy_request_for_path_with_session_headers_and_live_id(
    body: &str,
    original_user_agent: Option<&str>,
    request_path: &str,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
    live_request_id: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    open_responses_proxy_request_with_settings_and_user_agent(
        body,
        settings,
        original_user_agent,
        request_path,
        beta_features,
        session_headers,
        live_request_id,
    )
    .await
}

pub async fn open_responses_proxy_request_with_settings(
    body: &str,
    settings: crate::settings::BackendSettings,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_with_settings_and_user_agent(
        body,
        settings,
        None,
        "/responses",
        None,
        ProxySessionHeaders::default(),
        None,
    )
    .await
}

pub async fn open_responses_proxy_request_with_settings_for_path(
    body: &str,
    settings: crate::settings::BackendSettings,
    request_path: &str,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_with_settings_and_user_agent(
        body,
        settings,
        None,
        request_path,
        None,
        ProxySessionHeaders::default(),
        None,
    )
    .await
}

pub async fn open_responses_proxy_request_with_settings_for_path_and_beta(
    body: &str,
    settings: crate::settings::BackendSettings,
    request_path: &str,
    beta_features: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_with_settings_for_path_and_session_headers(
        body,
        settings,
        request_path,
        beta_features,
        ProxySessionHeaders::default(),
    )
    .await
}

pub async fn open_responses_proxy_request_with_settings_for_path_and_session_headers(
    body: &str,
    settings: crate::settings::BackendSettings,
    request_path: &str,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_with_settings_and_user_agent(
        body,
        settings,
        None,
        request_path,
        beta_features,
        session_headers,
        None,
    )
    .await
}

async fn open_responses_proxy_request_with_settings_and_user_agent(
    body: &str,
    settings: crate::settings::BackendSettings,
    original_user_agent: Option<&str>,
    request_path: &str,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
    live_request_id: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let mut request_json: Value = serde_json::from_str(body)?;
    expand_local_compaction_history(&mut request_json);
    let is_stream = request_json
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let source_model = request_json
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let aggregate_model = match crate::aggregate_catalog::resolve_active_model(&settings, &source_model) {
        Ok(binding) => binding,
        Err(error) => {
            return Ok(UpstreamProxyResponse {
                status_code: 400,
                content_type: "application/json".to_string(),
                is_stream: false,
                wire_api: UpstreamWireApi::Responses,
                compaction: false,
                native_compaction_passthrough: false,
                buffered_body: Some(serde_json::to_vec(&json!({
                    "error": {
                        "type": "invalid_request_error",
                        "code": "aggregate_model_unavailable",
                        "message": error.to_string()
                    }
                }))?),
                stream_prefix: Vec::new(),
                request_trace: None,
                exec_adapted: false,
                exec_outcome: None,
                response: None,
            });
        }
    };
    if let Some(binding) = &aggregate_model {
        request_json["model"] = Value::String(binding.upstream_model.clone());
    }
    let model_route = if aggregate_model.is_some() {
        None
    } else {
        select_model_route(&settings, &source_model)?
    };
    if let Some(route) = &model_route
        && route.upstream_model != source_model
    {
        request_json["model"] = Value::String(route.upstream_model.clone());
    }
    // 后台任务固定模型名映射（#1183/#1446/#1457）：聚合与显式路由都没接住、
    // 且模型不在当前供应商公布集合里时，改写为默认模型再转发。
    // `source_model` 保持原始值，轮转粘性与请求日志仍按用户视角的模型归因。
    if aggregate_model.is_none()
        && model_route.is_none()
        && let Some(replacement) = unlisted_model_replacement(&settings, &source_model)
    {
        log_unlisted_model_mapping(&source_model, &replacement);
        request_json["model"] = Value::String(replacement);
    }
    let model = (!source_model.trim().is_empty()).then(|| source_model.clone());
    let context = RotationContext {
        conversation_id: conversation_id_from_responses_request(&request_json),
        model,
    };
    let (relay, relays) = if let Some(binding) = &aggregate_model {
        (binding.relay.clone(), vec![binding.relay.clone()])
    } else if let Some(route) = &model_route {
        (route.relay.clone(), vec![route.relay.clone()])
    } else {
        let relay = crate::relay_rotation::select_relay_for_request(&settings, context)?;
        let mut relays = vec![relay.clone()];
        relays.extend(crate::relay_rotation::fallback_relays_after(
            &settings, &relay.id,
        )?);
        (relay, relays)
    };
    debug_assert_eq!(
        relays.first().map(|item| item.id.as_str()),
        Some(relay.id.as_str())
    );
    let strict_aggregate_model = aggregate_model.is_some();
    let relay_count = relays.len();
    let mut exec_negotiation_used = false;
    let mut auto_retry = ProxyAutoRetry::from_settings(&settings);
    let mut request_attempt = 0_usize;
    let buffered_mode = settings.protocol_proxy_auto_retry_mode == "buffered";
    let commit_policy = StreamStartCommitPolicy::from_settings(&settings);
    let mut retry_reasons: Vec<String> = Vec::new();
    let mut request_trace = crate::request_log::RequestTrace {
        model: source_model.clone(),
        ..Default::default()
    };
    // 外层循环负责两件事：按候选成员顺序尝试（成员间失败切换仍是立即的），
    // 以及全部候选都失败且错误可重试时退避后从第一个成员重新开始一轮。
    let mut candidate_index = 0_usize;
    'request_attempts: loop {
        if candidate_index >= relay_count {
            anyhow::bail!("未找到可用的聚合供应商成员")
        }
        let attempt = candidate_index;
        candidate_index += 1;
        let relay = relays[attempt].clone();
        request_attempt += 1;
        if let Some(id) = live_request_id {
            crate::request_log::update_running_retry(
                id,
                request_attempt as u32,
                retry_reasons.len() as u32,
                retry_reasons.clone(),
            );
            crate::request_log::update_running_phase(id, "requesting");
        }
        let has_more_candidates = attempt + 1 < relay_count;
        validate_upstream(&relay)?;
        // 官方登录成员：用该账号 OAuth 凭据发往 chatgpt.com，而不是 API Key。
        let official_member =
            crate::official_login::member_uses_official_login_transport(&relay);
        let mut official_credential = if official_member {
            match crate::official_login::resolve_official_credential(&relay, false).await {
                Ok(credential) => Some(credential),
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "protocol_proxy.official_login_unavailable",
                        json!({
                            "relayId": relay.id,
                            "relayName": relay.name,
                            "error": error_chain(&error)
                        }),
                    );
                    if !strict_aggregate_model {
                        crate::relay_rotation::record_relay_request_failure(&settings);
                    }
                    if has_more_candidates {
                        continue;
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        let official_base = official_member.then(crate::official_login::official_base_url);
        let model_override = aggregate_model
            .as_ref()
            .map(|binding| binding.upstream_model.clone())
            .or_else(|| aggregate_upstream_model_override(&settings, &relay));
        let preserve_native_request = relay.protocol == RelayProtocol::Responses
            && (request_has_compaction_trigger(&request_json)
                || is_responses_compact_proxy_path(request_path));
        let probe_native_compaction = request_has_compaction_trigger(&request_json)
            && relay.protocol == RelayProtocol::Responses
            && !is_responses_compact_proxy_path(request_path);
        let (mut endpoint, mut upstream_body, mut wire_api, compaction) =
            match upstream_request_parts(
                &relay,
                request_json.clone(),
                request_path,
                model_override.as_deref(),
                preserve_native_request,
                official_base.as_deref(),
                session_headers,
            )
            .await
            {
                Ok(parts) => parts,
                Err(error) => {
                    if has_more_candidates
                        && !strict_aggregate_model
                        && error.is::<NativeCompactionUnsupported>()
                    {
                        crate::relay_rotation::record_relay_request_failure(&settings);
                        continue;
                    }
                    return native_compaction_unsupported_result(error);
                }
            };
        let legacy_native_compaction = relay.protocol == RelayProtocol::Responses
            && is_responses_compact_proxy_path(request_path);
        let is_compaction_request = compaction || probe_native_compaction || legacy_native_compaction;
        request_trace.relay_id = relay.id.clone();
        request_trace.relay_name = relay.name.clone();
        request_trace.wire_api = upstream_wire_api_label(wire_api).to_string();
        request_trace.upstream_model = upstream_body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let header_timeout = response_header_timeout(is_stream);
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "protocol_proxy.upstream_request",
            json!({
                "relayId": relay.id,
                "relayName": relay.name,
                "endpoint": endpoint,
                "wireApi": wire_api,
                "stream": is_stream,
                "attempt": attempt + 1,
                "candidateCount": relay_count,
                "headerTimeoutSeconds": header_timeout.as_secs(),
                "modelRoute": model_route.as_ref().map(|route| json!({
                    "sourceRelayId": route.source_relay_id,
                    "sourceModel": route.source_model,
                    "targetRelayId": route.relay.id,
                    "upstreamModel": route.upstream_model
                }))
            }),
        );
        let user_agent = effective_user_agent(&relay.user_agent, original_user_agent);
        let eligible = wire_api == UpstreamWireApi::Responses
            && !is_compaction_request
            && crate::responses_exec::should_adapt_request(&upstream_body, request_path);
        let mut key = None;
        let mut exec_adapted = false;
        let send = async {
            let builder = build_responses_upstream_request(
                &relay, &endpoint, &upstream_body, wire_api, is_stream,
                original_user_agent, beta_features, session_headers, official_credential.as_ref(),
            )?;
            if eligible {
                key = Some(crate::responses_exec_transport::cache_key(
                    &relay.id, &builder.build()?, &upstream_body, &user_agent,
                ));
            }
            if key.as_ref().is_some_and(crate::responses_exec_transport::cached)
                && let Ok(adapted) = crate::responses_exec::adapt_request(upstream_body.clone())
            {
                upstream_body = adapted;
                exec_adapted = true;
            }
            send_responses_upstream_request(
                &relay, &endpoint, &upstream_body, wire_api, is_stream,
                original_user_agent, beta_features, session_headers, official_credential.as_ref(),
            ).await
        };
        let mut upstream = match send.await {
            Ok(upstream) => upstream,
            Err(error) => {
                if exec_adapted && let Some(key) = key.as_ref() {
                    crate::responses_exec_transport::evict(key);
                }
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "protocol_proxy.upstream_request_failed",
                    json!({
                        "relayId": relay.id,
                        "relayName": relay.name,
                        "endpoint": endpoint,
                        "wireApi": wire_api,
                        "stream": is_stream,
                        "attempt": attempt + 1,
                        "candidateCount": relay_count,
                        "headerTimeoutSeconds": header_timeout.as_secs(),
                        "willFailover": has_more_candidates,
                        "error": error_chain(&error)
                    }),
                );
                if !strict_aggregate_model {
                    crate::relay_rotation::record_relay_request_failure(&settings);
                }
                if has_more_candidates {
                    continue;
                }

                return Err(error).with_context(|| {
                    format!(
                        "供应商「{}」请求上游失败，endpoint: {}",
                        relay.name, endpoint
                    )
                });
            }
        };
        let mut status_code = upstream.status().as_u16();
        let mut content_type = upstream
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();

        // 官方登录 token 可能在 exp 之前就被撤销：401 时强制刷新并原样重试一次，
        // 仍失败再走正常的失败切换/返回路径。
        if status_code == 401 && official_member {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.official_login_retry",
                json!({
                    "relayId": relay.id,
                    "relayName": relay.name,
                    "endpoint": endpoint
                }),
            );
            match crate::official_login::resolve_official_credential(&relay, true).await {
                Ok(credential) => {
                    official_credential = Some(credential);
                    let retry = async {
                        if eligible {
                            let builder = build_responses_upstream_request(
                                &relay, &endpoint, &upstream_body, wire_api, is_stream,
                                original_user_agent, beta_features, session_headers, official_credential.as_ref(),
                            )?;
                            key = Some(crate::responses_exec_transport::cache_key(
                                &relay.id, &builder.build()?, &request_json, &user_agent,
                            ));
                        }
                        send_responses_upstream_request(
                        &relay,
                        &endpoint,
                        &upstream_body,
                        wire_api,
                        is_stream,
                        original_user_agent,
                        beta_features,
                        session_headers,
                        official_credential.as_ref(),
                    )
                        .await
                    };
                    match retry.await {
                        Ok(retried) => {
                            upstream = retried;
                            status_code = upstream.status().as_u16();
                            content_type = upstream
                                .headers()
                                .get(reqwest::header::CONTENT_TYPE)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or("")
                                .to_string();
                        }
                        Err(error) => {
                            if exec_adapted && let Some(key) = key.as_ref() {
                                crate::responses_exec_transport::evict(key);
                            }
                            if !strict_aggregate_model {
                                crate::relay_rotation::record_relay_request_failure(&settings);
                            }
                            if has_more_candidates {
                                continue;
                            }

                            return Err(error).with_context(|| {
                                format!(
                                    "供应商「{}」请求上游失败，endpoint: {}",
                                    relay.name, endpoint
                                )
                            });
                        }
                    }
                }
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "protocol_proxy.official_login_refresh_failed",
                        json!({
                            "relayId": relay.id,
                            "relayName": relay.name,
                            "error": error_chain(&error)
                        }),
                    );
                }
            }
        }

        if eligible && !exec_adapted && !exec_negotiation_used && status_code == 400 {
            let retry = async {
                let first_retry_after =
                    exec_retry_after_duration(upstream.headers(), SystemTime::now());
                let (inspected, rejected) =
                    crate::responses_exec_transport::inspect_rejection(upstream, &upstream_body).await?;
                upstream = inspected;
                if rejected
                    && let Ok(adapted) = crate::responses_exec::adapt_request(upstream_body.clone())
                {
                    if let Some(delay) = first_retry_after {
                        tokio::time::sleep(delay).await;
                    }
                    exec_negotiation_used = true;
                    exec_adapted = true;
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "protocol_proxy.exec_compatibility_retry",
                        json!({"relayId": relay.id, "stream": is_stream}),
                    );
                    upstream = send_responses_upstream_request(
                        &relay, &endpoint, &adapted, wire_api, is_stream,
                        original_user_agent, beta_features, session_headers, official_credential.as_ref(),
                    ).await?;
                }
                Ok::<_, anyhow::Error>(upstream)
            };
            upstream = match retry.await {
                Ok(response) => response,
                Err(error) => {
                    if exec_adapted && let Some(key) = key.as_ref() {
                        crate::responses_exec_transport::evict(key);
                    }
                    if !strict_aggregate_model {
                        crate::relay_rotation::record_relay_request_failure(&settings);
                    }
                    if has_more_candidates {
                        continue;
                    }

                    return Err(error);
                }
            };
            status_code = upstream.status().as_u16();
            content_type = upstream.headers().get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()).unwrap_or("").to_string();
        }

        if legacy_native_compaction {
            let body = read_limited_upstream_body(&mut upstream, MAX_NATIVE_COMPACTION_PROBE_BODY_BYTES).await?;
            let stream_response = response_body_is_stream(&content_type, &body);
            let success = (200..300).contains(&status_code);
            let terminal_failure = upstream_has_terminal_failure(&body, &content_type);
            let valid = !success || terminal_failure
                || upstream_compaction_item_count(&body, &content_type) == Some(1);
            let (code, body, response_type) = if valid {
                let body = if success && terminal_failure && stream_response {
                    let mut converter = CompactionSseConverter::new("");
                    converter.fail("原生压缩上游返回失败或不完整状态".to_string(), Some("compaction_failed".to_string()));
                    converter.finish()
                } else { body };
                (status_code, body, content_type)
            } else {
                (502, serde_json::to_vec(&json!({"error":{"type":"compaction_incompatible",
                    "code":"compaction_invalid_response", "message":"压缩上游未返回恰好一个完整有效的 checkpoint"}}))?,
                    "application/json".to_string())
            };
            return Ok(UpstreamProxyResponse {
                status_code: code, content_type: response_type, is_stream: valid && stream_response,
                wire_api, compaction: false, native_compaction_passthrough: valid && success,
                buffered_body: Some(body), exec_adapted: false, exec_outcome: None, response: None,
                stream_prefix: Vec::new(),
                request_trace: None,
            });
        }
        if probe_native_compaction {
            let success = (200..300).contains(&status_code);
            let probed = match read_limited_upstream_body(
                &mut upstream,
                MAX_NATIVE_COMPACTION_PROBE_BODY_BYTES,
            )
            .await
            {
                Ok(body) => body.to_vec(),
                Err(error) => {
                    return Err(error.into());
                }
            };
            let stream_response = response_body_is_stream(&content_type, &probed);

            if success && upstream_has_terminal_failure(&probed, &content_type) {
                if !strict_aggregate_model {
                    crate::relay_rotation::record_relay_request_failure(&settings);
                }
                let probed = if stream_response && String::from_utf8_lossy(&probed)
                    .replace("\r\n", "\n").split("\n\n").any(|block|
                        matches!(sse_block_type(block).as_deref(), Some("response.completed" | "response.output_item.done")))
                {
                    let mut converter = CompactionSseConverter::new("");
                    converter.fail("原生压缩上游返回失败或不完整状态".to_string(), Some("compaction_failed".to_string()));
                    converter.finish()
                } else { probed };
                return Ok(UpstreamProxyResponse {
                    status_code,
                    is_stream: stream_response,
                    content_type,
                    wire_api,
                    compaction: false,
                    native_compaction_passthrough: stream_response,
                    buffered_body: Some(probed),
                    stream_prefix: Vec::new(),
                    request_trace: None,
                    exec_adapted: false,
                    exec_outcome: None,
                    response: None,
                });
            }
            if success && stream_response {
                match upstream_compaction_item_count(&probed, &content_type) {
                    Some(1) => {
                        if !strict_aggregate_model {
                            crate::relay_rotation::record_relay_request_event(
                                &settings,
                                RotationEvent::Success,
                            );
                        }
                        return Ok(UpstreamProxyResponse {
                            status_code,
                            is_stream: true,
                            content_type,
                            wire_api,
                            compaction: false,
                            native_compaction_passthrough: true,
                            buffered_body: Some(probed),
                            stream_prefix: Vec::new(),
                            request_trace: None,
                            exec_adapted: false,
                            exec_outcome: None,
                            response: None,
                        });
                    }
                    None => {
                        if !strict_aggregate_model {
                            crate::relay_rotation::record_relay_request_failure(&settings);
                        }
                        return Ok(UpstreamProxyResponse {
                            status_code,
                            is_stream: true,
                            content_type: "text/event-stream".to_string(),
                            wire_api,
                            compaction: false,
                            native_compaction_passthrough: true,
                            buffered_body: Some(incomplete_compaction_stream(&probed)),
                            stream_prefix: Vec::new(),
                            request_trace: None,
                            exec_adapted: false,
                            exec_outcome: None,
                            response: None,
                        });
                    }
                    Some(_) => {}
                }
            } else if success
                && upstream_compaction_item_count(&probed, &content_type) == Some(1)
            {
                if !strict_aggregate_model {
                    crate::relay_rotation::record_relay_request_event(
                        &settings,
                        RotationEvent::Success,
                    );
                }
                if !is_stream {
                    return Ok(UpstreamProxyResponse {
                        status_code,
                        is_stream: false,
                        content_type,
                        wire_api,
                        compaction: false,
                        native_compaction_passthrough: true,
                        buffered_body: Some(probed),
                        stream_prefix: Vec::new(),
                        request_trace: None,
                        exec_adapted: false,
                        exec_outcome: None,
                        response: None,
                    });
                }
                if let Some(sse) = render_native_compaction_as_sse(&probed) {
                    return Ok(UpstreamProxyResponse {
                        status_code,
                        is_stream: true,
                        content_type: "text/event-stream".to_string(),
                        wire_api,
                        compaction: false,
                        native_compaction_passthrough: true,
                        buffered_body: Some(sse),
                        stream_prefix: Vec::new(),
                        request_trace: None,
                        exec_adapted: false,
                        exec_outcome: None,
                        response: None,
                    });
                }
            }
            if !success && !compaction_input_rejected(status_code, &probed) {
                if !strict_aggregate_model {
                    crate::relay_rotation::record_relay_request_failure(&settings);
                }
                if has_more_candidates {
                    continue;
                }
                return Ok(UpstreamProxyResponse {
                    status_code,
                    is_stream: false,
                    content_type,
                    wire_api,
                    compaction: false,
                    native_compaction_passthrough: false,
                    buffered_body: Some(probed),
                    stream_prefix: Vec::new(),
                    request_trace: None,
                    exec_adapted: false,
                    exec_outcome: None,
                    response: None,
                });
            }

            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.compaction_fallback",
                json!({
                    "relayId": relay.id,
                    "statusCode": status_code,
                }),
            );
            let fallback = upstream_request_parts(
                &relay,
                request_json.clone(),
                request_path,
                model_override.as_deref(),
                false,
                official_base.as_deref(),
                session_headers,
            )
            .await;
            let (fallback_endpoint, fallback_body, fallback_wire, _) = match fallback {
                Ok(parts) => parts,
                Err(error) => {
                    if has_more_candidates
                        && !strict_aggregate_model
                        && error.is::<NativeCompactionUnsupported>()
                    {
                        crate::relay_rotation::record_relay_request_failure(&settings);
                        continue;
                    }
                    return native_compaction_unsupported_result(error);
                }
            };
            endpoint = fallback_endpoint;
            upstream_body = fallback_body;
            wire_api = fallback_wire;
            upstream = match send_responses_upstream_request(
                &relay,
                &endpoint,
                &upstream_body,
                wire_api,
                is_stream,
                original_user_agent,
                beta_features,
                session_headers,
                official_credential.as_ref(),
            )
            .await
            {
                Ok(upstream) => upstream,
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "protocol_proxy.upstream_request_failed",
                        json!({
                            "relayId": relay.id,
                            "endpoint": endpoint,
                            "compactionFallback": true,
                            "error": error_chain(&error)
                        }),
                    );
                    if !strict_aggregate_model {
                        crate::relay_rotation::record_relay_request_failure(&settings);
                    }
                    if has_more_candidates {
                        continue;
                    }
                    return Err(error).with_context(|| {
                        format!(
                            "供应商「{}」请求上游失败，endpoint: {}",
                            relay.name, endpoint
                        )
                    });
                }
            };
            status_code = upstream.status().as_u16();
            content_type = upstream
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_string();
        }

        if let Some(id) = live_request_id {
            crate::request_log::update_running_phase(id, "waiting");
        }
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "protocol_proxy.upstream_response",
            json!({
                "relayId": relay.id,
                "relayName": relay.name,
                "endpoint": endpoint,
                "wireApi": wire_api,
                "stream": is_stream,
                "statusCode": status_code,
                "contentType": content_type,
                "autoRetryMode": settings.protocol_proxy_auto_retry_mode,
                "bufferedMode": buffered_mode,
                "attempt": attempt + 1,
                "candidateCount": relay_count,
                "headerTimeoutSeconds": header_timeout.as_secs(),
                "willFailover": has_more_candidates && !(200..300).contains(&status_code)
            }),
        );
        if exec_adapted && !(200..300).contains(&status_code)
            && let Some(key) = key.as_ref()
        {
            crate::responses_exec_transport::evict(key);
        }
        if !strict_aggregate_model && (!exec_adapted || !(200..300).contains(&status_code)) {
            crate::relay_rotation::record_relay_request_event(
                &settings,
                if (200..300).contains(&status_code) {
                    RotationEvent::Success
                } else {
                    RotationEvent::Failure
                },
            );
        }
        if !(200..300).contains(&status_code) {
            if has_more_candidates {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "protocol_proxy.upstream_failover",
                    json!({
                        "relayId": relay.id,
                        "relayName": relay.name,
                        "endpoint": endpoint,
                        "wireApi": wire_api,
                        "stream": is_stream,
                        "statusCode": status_code,
                        "attempt": attempt + 1,
                        "candidateCount": relay_count,
                        "headerTimeoutSeconds": header_timeout.as_secs()
                    }),
                );
                continue;
            }
            // 已经没有可切换的成员：命中用户所选状态码（默认 429/5xx）就退避后
            // 从第一个成员重来一轮。此时还没向 Codex 写出任何字节，可以整次重发。
            // 先占用预算再读错误体，预算用尽时响应体原样交给 Codex。
            let retry_after = exec_retry_after_duration(upstream.headers(), SystemTime::now());
            let scheduled = auto_retry
                .as_mut()
                .filter(|retry| retry.retries_status(status_code))
                .and_then(|retry| {
                    retry
                        .schedule(retry_after)
                        .map(|delay| (delay, retry.remaining))
                });
            if let Some((delay, retries_remaining)) = scheduled {
                let body = read_limited_upstream_body(
                    &mut upstream,
                    PROXY_AUTO_RETRY_ERROR_BODY_LIMIT,
                )
                .await
                .unwrap_or_default();
                drop(upstream);
                let (message, _, code, _) =
                    upstream_error_parts(status_code, &content_type, &body);
                let code = code.unwrap_or_else(|| status_code.to_string());
                wait_scheduled_proxy_auto_retry(
                    delay,
                    retries_remaining,
                    &relay,
                    request_attempt,
                    &format!("upstream_http_status:{status_code}"),
                    &mut retry_reasons,
                    live_request_id,
                )
                .await;
                let detail = attempt_detail(request_attempt, Some(&code), Some(&message), delay);
                if let Some(id) = live_request_id {
                    crate::request_log::update_running_attempt_detail(id, detail.clone());
                }
                request_trace.attempt_details.push(detail);
                candidate_index = 0;
                continue 'request_attempts;
            }
        }
        // 200 的流式响应也可能带着 response.failed（官方 server_overloaded
        // 就是先回 200 再在流里失败）。在向 Codex 写出任何字节前先预读；
        // 若识别到容量失败就退避后重发同一次上游请求。
        let mut stream_prefix = Vec::new();
        if is_stream
            && (200..300).contains(&status_code)
            && wire_api == UpstreamWireApi::Responses
            && !is_compaction_request
            && !exec_adapted
            && auto_retry.is_some()
            && buffered_mode
        {
            match read_responses_stream_for_buffering(&mut upstream, live_request_id).await {
                BufferedResponsesStream::Complete {
                    body,
                    first_output_at_ms,
                    terminal_failure,
                } => {
                    if let Some(first_output_at_ms) = first_output_at_ms {
                        request_trace.first_output_at_ms = Some(first_output_at_ms);
                    }
                    if let Some((code, message)) = terminal_failure
                        && stream_failure_is_retryable(code.as_deref(), message.as_deref())
                    {
                        let reason = format!(
                            "upstream_buffered_overload:{}",
                            code.clone().unwrap_or_default()
                        );
                        if let Some(delay) = schedule_proxy_auto_retry(
                            &mut auto_retry,
                            &relay,
                            request_attempt,
                            &reason,
                            None,
                            &mut retry_reasons,
                            live_request_id,
                        )
                        .await
                        {
                            let detail = attempt_detail(
                                request_attempt,
                                code.as_deref(),
                                message.as_deref(),
                                delay,
                            );
                            if let Some(id) = live_request_id {
                                crate::request_log::update_running_attempt_detail(id, detail.clone());
                            }
                            request_trace.attempt_details.push(detail);
                            if !strict_aggregate_model {
                                crate::relay_rotation::record_relay_request_failure(&settings);
                            }
                            candidate_index = 0;
                            continue 'request_attempts;
                        }
                    }
                    request_trace.attempts = request_attempt as u32;
                    request_trace.auto_retries = retry_reasons.len() as u32;
                    request_trace.retry_reasons = retry_reasons.clone();
                    return Ok(UpstreamProxyResponse {
                        status_code,
                        is_stream: true,
                        content_type,
                        wire_api,
                        compaction: false,
                        native_compaction_passthrough: false,
                        buffered_body: Some(body),
                        stream_prefix: Vec::new(),
                        request_trace: Some(request_trace),
                        exec_adapted: false,
                        exec_outcome: None,
                        response: None,
                    });
                }
                BufferedResponsesStream::Overflow {
                    prefix,
                    first_output_at_ms,
                } => {
                    stream_prefix = prefix;
                    if let Some(first_output_at_ms) = first_output_at_ms {
                        request_trace.first_output_at_ms = Some(first_output_at_ms);
                    }
                }
            }
        } else if is_stream
            && (200..300).contains(&status_code)
            && wire_api == UpstreamWireApi::Responses
            && !is_compaction_request
            && !exec_adapted
            && auto_retry.is_some()
        {
            // 重试预算已用尽时再缓冲没有收益，直接退回旧提交时机，
            // 避免为首字白等一个宽限窗口。
            let attempt_commit_policy = if auto_retry
                .as_ref()
                .is_some_and(|retry| retry.remaining > 0)
            {
                commit_policy
            } else {
                commit_policy.without_retry_budget()
            };
            match peek_responses_stream_start(
                &mut upstream,
                attempt_commit_policy,
                live_request_id,
            )
            .await
            {
                StreamStartPeek::Forward {
                    prefix,
                    first_output_at_ms,
                } => {
                    stream_prefix = prefix;
                    if first_output_at_ms.is_some() {
                        request_trace.first_output_at_ms = first_output_at_ms;
                    }
                }
                StreamStartPeek::Failed {
                    prefix,
                    code,
                    message,
                    first_output_at_ms,
                } => {
                    let retryable =
                        stream_failure_is_retryable(code.as_deref(), message.as_deref());
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "protocol_proxy.upstream_stream_failed",
                        json!({
                            "relayId": relay.id,
                            "relayName": relay.name,
                            "code": code,
                            "message": message,
                            "retryable": retryable,
                            "requestAttempt": request_attempt,
                        }),
                    );
                    if retryable {
                        if !strict_aggregate_model {
                            crate::relay_rotation::record_relay_request_failure(&settings);
                        }
                        // 聚合还有候选成员时先立即换下一个，别让同一个过载成员占住等待时间。
                        if has_more_candidates {
                            continue;
                        }
                        // 重试原因带上根因 code，请求日志里能直接看出为什么重试。
                        let retry_reason = match code
                            .as_deref()
                            .map(str::trim)
                            .filter(|code| !code.is_empty())
                        {
                            Some(code) => format!("upstream_stream_failed:{code}"),
                            None => "upstream_stream_failed:unknown".to_string(),
                        };
                        if let Some(delay) = schedule_proxy_auto_retry(
                            &mut auto_retry,
                            &relay,
                            request_attempt,
                            &retry_reason,
                            None,
                            &mut retry_reasons,
                            live_request_id,
                        )
                        .await
                        {
                            let detail = attempt_detail(
                                request_attempt,
                                code.as_deref(),
                                message.as_deref(),
                                delay,
                            );
                            if let Some(id) = live_request_id {
                                crate::request_log::update_running_attempt_detail(id, detail.clone());
                            }
                            request_trace.attempt_details.push(detail);
                            candidate_index = 0;
                            continue 'request_attempts;
                        }
                    }
                    // 不重试时把预读前缀连同首字时间交回；否则请求日志会把
                    // 首字耗时记成开始转发的时刻，宽限窗口会虚增这段耗时。
                    stream_prefix = prefix;
                    if first_output_at_ms.is_some() {
                        request_trace.first_output_at_ms = first_output_at_ms;
                    }
                }
            }
        }
        // 成功，或重试预算用尽：把（最后一个）上游响应交给 Codex。
        // 请求了 SSE 的兼容上游也可能返回 JSON 摘要；让调用方选择 JSON 包装器。
        request_trace.attempts = request_attempt as u32;
        request_trace.auto_retries = retry_reasons.len() as u32;
        request_trace.retry_reasons = retry_reasons.clone();
        let json_compaction = is_compaction_request
            && content_type
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"));
        return Ok(UpstreamProxyResponse {
            status_code,
            is_stream: (is_stream && !json_compaction)
                || content_type.contains("text/event-stream"),
            content_type,
            wire_api,
            compaction: is_compaction_request,
            native_compaction_passthrough: false,
            buffered_body: None,
            stream_prefix,
            request_trace: Some(request_trace),
            exec_adapted,
            exec_outcome: (exec_adapted && (200..300).contains(&status_code)).then(|| {
                crate::responses_exec_transport::ExecOutcome::new(
                    key.expect("adapted request has a cache identity"), &settings,
                )
            }),
            response: Some(upstream),
        });
    }
}

/// 目录外模型名 → 当前供应商默认模型的映射（#1183 / #1446 / #1457）。
///
/// Codex 客户端的自动审查（`codex-auto-review`）、标题/摘要、子代理等后台任务
/// 固定使用 gpt-* 或内部模型名，不经用户选模；第三方上游没有这些名字，原样
/// 转发只会得到「No available channel / 模型不存在」类错误，还可能按失败重试
/// 持续消耗额度。维护者定的方向是「按来源映射」，不丢弃请求
/// （<https://github.com/BigPizzaV3/CodexPlusPlus/issues/1183#issuecomment-6052622554>）。
///
/// 规则：请求模型（剥掉 `[1M]` 窗口后缀后，忽略大小写）不在当前供应商公布的
/// 模型集里时，改写为该供应商的默认模型。公布的模型集沿用
/// `active_published_model_ids` 的口径——官方登录（未混入 API）返回空集
/// （chatgpt.com 后端本来就有 gpt-*，不映射）；聚合供应商有自己的按成员归因
/// 与「不做静默改派」策略，不在此处理。返回 `None` 表示照原样转发。
fn unlisted_model_replacement(
    settings: &crate::settings::BackendSettings,
    model: &str,
) -> Option<String> {
    if !settings.protocol_proxy_map_unlisted_models
        || settings.active_aggregate_relay_profile().is_some()
    {
        return None;
    }
    let model = model.trim();
    // 自动审批模型优先（#1446）：供应商配置了 autoReviewModel 时，codex-auto-review
    // 交给 `auto_review_model_rewrite` 按配置改写；这里若先改成默认模型，
    // 那条规则就再也匹配不上，用户配置的审批模型会被静默覆盖。
    if auto_review_model_rewrite(&settings.active_relay_profile(), model).is_some() {
        return None;
    }
    let published = crate::model_catalog::active_published_model_ids(settings);
    if published.is_empty() {
        return None;
    }
    if model.is_empty() {
        return None;
    }
    let (slug, _) = crate::model_suffix::parse_model_suffix(model);
    let slug_lower = slug.to_ascii_lowercase();
    let raw_lower = model.to_ascii_lowercase();
    if published.iter().any(|id| {
        let id = id.to_ascii_lowercase();
        id == slug_lower || id == raw_lower
    }) {
        return None;
    }
    let replacement = crate::relay_config::relay_profile_model(&settings.active_relay_profile())
        .trim()
        .to_string();
    (!replacement.is_empty()).then_some(replacement)
}

/// 每对「来源 → 目标」只记一条映射诊断日志：#1183 的后台探测约每秒一次，
/// 逐请求记录会把诊断文件刷大（同类投诉见 #2681）。
fn log_unlisted_model_mapping(source: &str, replacement: &str) {
    static LOGGED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    let key = format!(
        "{}->{}",
        source.to_ascii_lowercase(),
        replacement.to_ascii_lowercase()
    );
    let logged = LOGGED.get_or_init(Default::default);
    // 临界区只有 HashSet::insert，不会恐慌；即便锁中毒（Err）也取回内部
    // 状态继续去重，保证「每对只记一条」在任何情况下都成立。
    let mut guard = logged
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !guard.insert(key) {
        return;
    }
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.unlisted_model_mapped",
        json!({"from": source, "to": replacement}),
    );
}

fn select_model_route(
    settings: &crate::settings::BackendSettings,
    model: &str,
) -> anyhow::Result<Option<ModelRouteSelection>> {
    if model.is_empty() || settings.active_aggregate_relay_profile().is_some() {
        return Ok(None);
    }

    let source = settings.active_relay_profile();
    let Some(route) = source
        .model_routes
        .iter()
        .find(|route| route.model.trim() == model)
    else {
        return Ok(None);
    };
    let target_relay_id = route.target_relay_id.trim();
    if target_relay_id == source.id {
        anyhow::bail!("模型路由不能指向当前供应商自身：{model}");
    }
    let target = settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == target_relay_id)
        .cloned()
        .with_context(|| format!("模型路由目标供应商不存在：{target_relay_id}"))?;
    if target.relay_mode == crate::settings::RelayMode::Aggregate {
        anyhow::bail!("模型路由目标不能是聚合供应商：{}", target.name);
    }
    if target.protocol != RelayProtocol::Responses {
        anyhow::bail!("模型路由目标必须使用 Responses API：{}", target.name);
    }

    let upstream_model = if route.target_model.trim().is_empty() {
        model.to_string()
    } else {
        route.target_model.trim().to_string()
    };
    Ok(Some(ModelRouteSelection {
        relay: target,
        source_relay_id: source.id,
        source_model: model.to_string(),
        upstream_model,
    }))
}

fn aggregate_upstream_model_override(
    settings: &crate::settings::BackendSettings,
    relay: &crate::settings::RelayProfile,
) -> Option<String> {
    settings.active_aggregate_relay_profile()?;
    let model = crate::relay_config::relay_profile_model(relay);
    let model = model.trim();
    (!model.is_empty()).then(|| model.to_string())
}

pub async fn open_models_proxy_request(
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let relay = crate::relay_rotation::select_relay_for_probe(&settings)?;
    validate_upstream(&relay)?;

    // 官方登录成员没有 base_url：模型列表同样发往 chatgpt.com 后端并带账号身份。
    let official_member = crate::official_login::member_uses_official_login_transport(&relay);
    let official_credential = if official_member {
        Some(crate::official_login::resolve_official_credential(&relay, false).await?)
    } else {
        None
    };
    let base_url = if official_member {
        crate::official_login::official_base_url()
    } else {
        relay.base_url.clone()
    };
    let endpoint = models_url(&base_url);
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.models_request",
        json!({
            "relayId": relay.id,
            "relayName": relay.name,
            "endpoint": endpoint,
            "wireApi": UpstreamWireApi::Responses
        }),
    );
    let request = crate::http_client::proxied_client_for_profile(&effective_user_agent(
        &relay.user_agent,
        original_user_agent,
    ), &relay.id)?
    .get(endpoint);
    let request = match &official_credential {
        Some(credential) => with_official_login_auth(request, &relay, credential),
        None => with_relay_auth(request, &relay),
    };
    let upstream = send_upstream_request(request).await?;
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8")
        .to_string();

    Ok(UpstreamProxyResponse {
        status_code,
        is_stream: false,
        content_type,
        wire_api: UpstreamWireApi::Responses,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: None,
        stream_prefix: Vec::new(),
        request_trace: None,
        exec_adapted: false,
        exec_outcome: None,
        response: Some(upstream),
    })
}

pub async fn open_audio_transcriptions_proxy_request(
    body: &[u8],
    content_type: &str,
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let relay = crate::relay_rotation::select_relay_for_probe(&settings)?;
    validate_upstream(&relay)?;
    if crate::official_login::member_uses_official_login_transport(&relay) {
        anyhow::bail!("音频转写暂不支持官方登录成员「{}」", relay.name);
    }
    let content_type = content_type.trim();
    if content_type.is_empty() {
        anyhow::bail!("Audio transcriptions 请求缺少 Content-Type");
    }

    let endpoint = audio_transcriptions_url(&relay.base_url);
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.audio_transcriptions_request",
        json!({
            "relayId": relay.id,
            "relayName": relay.name,
            "endpoint": endpoint,
            "wireApi": UpstreamWireApi::AudioTranscriptions,
            "bodyBytes": body.len()
        }),
    );
    let request = crate::http_client::proxied_client_for_profile(&effective_user_agent(
        &relay.user_agent,
        original_user_agent,
    ), &relay.id)?
    .post(endpoint)
    .header(reqwest::header::CONTENT_TYPE, content_type)
    .body(body.to_vec());
    let upstream = send_upstream_request(with_relay_auth(request, &relay)).await?;
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8")
        .to_string();

    Ok(UpstreamProxyResponse {
        status_code,
        is_stream: false,
        content_type,
        wire_api: UpstreamWireApi::AudioTranscriptions,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: None,
        stream_prefix: Vec::new(),
        request_trace: None,
        exec_adapted: false,
        exec_outcome: None,
        response: Some(upstream),
    })
}

pub async fn open_image_generations_proxy_request(
    body: &[u8],
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_image_proxy_request(
        body,
        "application/json",
        original_user_agent,
        ImageProxyEndpoint::Generations,
    )
    .await
}

pub async fn open_image_edits_proxy_request(
    body: &[u8],
    content_type: &str,
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_image_proxy_request(
        body,
        content_type,
        original_user_agent,
        ImageProxyEndpoint::Edits,
    )
    .await
}

#[derive(Debug, Clone, Copy)]
enum ImageProxyEndpoint {
    Generations,
    Edits,
}

impl ImageProxyEndpoint {
    fn url(self, base_url: &str) -> String {
        match self {
            Self::Generations => image_generations_url(base_url),
            Self::Edits => image_edits_url(base_url),
        }
    }

    fn wire_api(self) -> UpstreamWireApi {
        match self {
            Self::Generations => UpstreamWireApi::ImageGenerations,
            Self::Edits => UpstreamWireApi::ImageEdits,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Generations => "image_generations",
            Self::Edits => "image_edits",
        }
    }

    /// Codex CLI 的图片扩展按 `{provider base_url}/{path}` 拼接上游地址；
    /// 官方登录通道同样复用这两个相对路径。
    fn path(self) -> &'static str {
        match self {
            Self::Generations => "images/generations",
            Self::Edits => "images/edits",
        }
    }
}

async fn open_image_proxy_request(
    body: &[u8],
    content_type: &str,
    original_user_agent: Option<&str>,
    endpoint_kind: ImageProxyEndpoint,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let relay = crate::relay_rotation::select_relay_for_probe(&settings)?;
    let content_type = normalize_image_content_type(content_type, endpoint_kind)?;
    // 官方登录成员：Codex CLI 的 imagegen 扩展会把图片请求发给当前 provider 的
    // base_url；聚合代理下该地址是本地协议代理，这里要像 /responses、/models 一样
    // 用该账号的 OAuth 凭据转发到 chatgpt.com 后端，而不是直接拒绝。
    if crate::official_login::member_uses_official_login_transport(&relay) {
        return open_official_image_proxy_request(
            body,
            content_type,
            original_user_agent,
            endpoint_kind,
            &relay,
        )
        .await;
    }
    let base_url = if relay.upstream_base_url.trim().is_empty() {
        crate::relay_config::relay_profile_base_url(&relay)
    } else {
        relay.upstream_base_url.trim().to_string()
    };
    if is_local_protocol_proxy_base_url(&base_url) {
        anyhow::bail!("图片上游 Base URL 不能指向本地协议代理");
    }
    if base_url.trim().is_empty() {
        anyhow::bail!("图片上游 Base URL 不能为空");
    }
    if relay.api_key.trim().is_empty() && !relay.uses_no_auth() {
        anyhow::bail!("图片上游 Key 不能为空");
    }
    let endpoint = endpoint_kind.url(&base_url);
    let wire_api = endpoint_kind.wire_api();
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.image_request",
        json!({
            "relayId": relay.id,
            "relayName": relay.name,
            "endpoint": endpoint,
            "wireApi": wire_api,
            "bodyBytes": body.len(),
            "endpointKind": endpoint_kind.name()
        }),
    );
    let request = crate::http_client::proxied_client_for_profile(&effective_user_agent(
        &relay.user_agent,
        original_user_agent,
    ), &relay.id)?
    .post(endpoint)
    .header(reqwest::header::CONTENT_TYPE, content_type)
    .body(body.to_vec());
    let upstream = send_upstream_request_with_header_timeout(
        with_relay_auth(request, &relay),
        UPSTREAM_IMAGE_HEADER_TIMEOUT,
    )
    .await?;
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8")
        .to_string();

    Ok(UpstreamProxyResponse {
        status_code,
        is_stream: false,
        content_type,
        wire_api,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: None,
        stream_prefix: Vec::new(),
        request_trace: None,
        exec_adapted: false,
        exec_outcome: None,
        response: Some(upstream),
    })
}

/// 图片接口的 Content-Type 归一化：官方登录通道和 API 通道共用。
fn normalize_image_content_type<'a>(
    content_type: &'a str,
    endpoint_kind: ImageProxyEndpoint,
) -> anyhow::Result<&'a str> {
    let content_type = content_type.trim();
    Ok(if content_type.is_empty() {
        match endpoint_kind {
            ImageProxyEndpoint::Generations => "application/json",
            ImageProxyEndpoint::Edits => {
                anyhow::bail!("图片 edits 请求缺少 Content-Type");
            }
        }
    } else {
        content_type
    })
}

/// 官方登录成员的生图上游：Codex CLI 的 imagegen 扩展对官方 provider 会请求
/// `{chatgpt backend}/images/{generations|edits}`，凭据是 ChatGPT OAuth，
/// 不是本地代理 Key。这里按同样的相对路径和认证头转发。
async fn open_official_image_proxy_request(
    body: &[u8],
    content_type: &str,
    original_user_agent: Option<&str>,
    endpoint_kind: ImageProxyEndpoint,
    relay: &crate::settings::RelayProfile,
) -> anyhow::Result<UpstreamProxyResponse> {
    let credential = crate::official_login::resolve_official_credential(relay, false).await?;
    let endpoint = format!(
        "{}/{}",
        crate::official_login::official_base_url().trim_end_matches('/'),
        endpoint_kind.path()
    );
    let wire_api = endpoint_kind.wire_api();
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.image_request",
        json!({
            "relayId": relay.id,
            "relayName": relay.name,
            "endpoint": endpoint,
            "wireApi": wire_api,
            "bodyBytes": body.len(),
            "endpointKind": endpoint_kind.name(),
            "officialLogin": true
        }),
    );
    let request = crate::http_client::proxied_client_for_profile(
        &effective_user_agent(&relay.user_agent, original_user_agent),
        &relay.id,
    )?
    .post(endpoint)
    .header(reqwest::header::CONTENT_TYPE, content_type)
    .body(body.to_vec());
    let upstream = send_upstream_request_with_header_timeout(
        with_official_login_auth(request, relay, &credential),
        UPSTREAM_IMAGE_HEADER_TIMEOUT,
    )
    .await?;
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8")
        .to_string();

    Ok(UpstreamProxyResponse {
        status_code,
        is_stream: false,
        content_type,
        wire_api,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: None,
        stream_prefix: Vec::new(),
        request_trace: None,
        exec_adapted: false,
        exec_outcome: None,
        response: Some(upstream),
    })
}

fn response_header_timeout(is_stream: bool) -> Duration {
    if is_stream {
        UPSTREAM_STREAM_HEADER_TIMEOUT
    } else {
        UPSTREAM_HEADER_TIMEOUT
    }
}

pub async fn open_chat_completions_proxy_request(
    body: &str,
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let relay = settings.active_relay_profile();
    if relay.protocol != RelayProtocol::ChatCompletions {
        anyhow::bail!("当前中转未启用 Chat Completions 协议代理");
    }
    if relay.base_url.trim().is_empty() {
        anyhow::bail!("Chat Completions 上游 Base URL 不能为空");
    }
    if relay.api_key.trim().is_empty() && !relay.uses_no_auth() {
        anyhow::bail!("Chat Completions 上游 Key 不能为空");
    }

    let mut request_json: Value = serde_json::from_str(body)?;
    // 与 Responses 路径同规则的目录外模型映射（#1183/#1446/#1457）：
    // 这条直通路径此前对模型名零干预，后台任务的 gpt-* 原样打到上游。
    let requested_model = request_json
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // 自动审批模型优先于目录外映射（与 Responses 路径的 upstream_request_parts 一致）。
    if let Some(auto_review_model) = auto_review_model_rewrite(&relay, &requested_model) {
        request_json["model"] = json!(auto_review_model);
    } else if let Some(replacement) = unlisted_model_replacement(&settings, &requested_model) {
        log_unlisted_model_mapping(&requested_model, &replacement);
        request_json["model"] = Value::String(replacement);
    }
    let is_stream = request_json
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let request = crate::http_client::proxied_client_for_profile(&effective_user_agent(
        &relay.user_agent,
        original_user_agent,
    ), &relay.id)?
    .post(chat_completions_url(&relay.base_url))
    .header(reqwest::header::CONTENT_TYPE, "application/json")
    .json(&request_json);
    let upstream = with_relay_auth(request, &relay).send().await?;
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();

    Ok(UpstreamProxyResponse {
        status_code,
        is_stream: is_stream || content_type.contains("text/event-stream"),
        content_type,
        wire_api: UpstreamWireApi::ChatCompletions,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: None,
        stream_prefix: Vec::new(),
        request_trace: None,
        exec_adapted: false,
        exec_outcome: None,
        response: Some(upstream),
    })
}

/// 补齐搜索历史的工具声明，不删历史，也不为无工具的压缩请求启用搜索。
pub fn ensure_web_search_tool_for_history(request_json: &mut Value) -> bool {
    let Some(input) = request_json.get("input").and_then(Value::as_array) else {
        return false;
    };
    if !input.iter().any(|item| item["type"] == "web_search_call") {
        return false;
    }
    let is_web_search = |tool: &Value| {
        matches!(
            tool.get("type").and_then(Value::as_str),
            Some("web_search" | "web_search_preview" | "web_search_preview_2025_03_11")
        )
    };
    let mut has_additional_tools = false;
    for tool in input
        .iter()
        .filter(|item| item["type"] == "additional_tools")
        .filter_map(|item| item.get("tools").and_then(Value::as_array))
        .flatten()
    {
        has_additional_tools = true;
        if is_web_search(tool) {
            return false;
        }
    }
    let has_tools = match request_json.get("tools") {
        None | Some(Value::Null) => false,
        Some(Value::Array(tools)) => {
            if tools.iter().any(is_web_search) {
                return false;
            }
            !tools.is_empty()
        }
        _ => return false,
    } || has_additional_tools;
    let tool = json!({"type": "web_search", "external_web_access": false});
    if let Some(tools) = request_json.get_mut("tools").and_then(Value::as_array_mut) {
        tools.push(tool);
    } else {
        request_json["tools"] = json!([tool]);
    }
    if !has_tools
        && (matches!(request_json.get("tool_choice"), None | Some(Value::Null))
            || matches!(
                request_json.get("tool_choice").and_then(Value::as_str),
                Some("auto" | "none")
            ))
    {
        request_json["tool_choice"] = json!("none");
    }
    true
}

/// 请求是否完全没有声明工具（含 Responses Lite 的 `additional_tools` 输入项）。
/// 只在这类请求上自动补搜索声明，避免改变正常轮次能调用的工具集合。
fn request_declares_no_tools(body: &Value) -> bool {
    let tools_absent = match body.get("tools") {
        None | Some(Value::Null) => true,
        Some(Value::Array(tools)) => tools.is_empty(),
        _ => false,
    };
    tools_absent
        && !body
            .get("input")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item["type"] == "additional_tools"))
}

fn strip_reasoning_parameters(body: &mut Value) {
    if let Some(object) = body.as_object_mut() {
        for key in ["reasoning", "reasoning_effort", "thinking", "enable_thinking", "reasoning_split", "thinking_budget"] {
            object.remove(key);
        }
        if let Some(include) = object.get_mut("include").and_then(Value::as_array_mut) {
            include.retain(|entry| entry.as_str() != Some("reasoning.encrypted_content"));
        }
    }
}

/// 用户明确选择严格上游契约时才投影请求；不修改 rollout，也不验证/伪造账号密文。
fn project_strict_reasoning_history(body: &mut Value) -> anyhow::Result<()> {
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else { return Ok(()); };
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("reasoning")
            || !item.get("content").and_then(Value::as_array).is_some_and(|parts| !parts.is_empty())
            || !item.get("encrypted_content").is_some_and(|value| !value.is_null())
        { continue; }
        let raw = item.get("encrypted_content").and_then(Value::as_str).unwrap_or("");
        let decoded = base64::engine::general_purpose::URL_SAFE.decode(raw)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw));
        let recognized_envelope = decoded.is_ok_and(|bytes| bytes.len() >= 73
            && bytes[0] == 0x80 && (bytes.len() - 57) % 16 == 0);
        if !recognized_envelope {
            anyhow::bail!("incompatible_reasoning_history: 推理同时包含明文与无法识别的加密项；请求未发出，本地历史保持原样。请使用原供应商继续，或在新会话中切换模型。");
        }
        // 仅检查信封格式；真正的认证/解密仍由选定上游完成，失败不重放或降级。
        item["content"] = json!([]);
    }
    Ok(())
}

/// Codex 自动审批（「替我审批」）请求使用的固定模型 slug。
const CODEX_AUTO_REVIEW_MODEL: &str = "codex-auto-review";

/// codex-auto-review 的改写目标：供应商配置了审批模型（#1446）时返回它，否则 `None`。
/// 目录外模型映射（#1183）也用它判断是否让路，两处共用同一条规则。
fn auto_review_model_rewrite<'a>(
    relay: &'a crate::settings::RelayProfile,
    model: &str,
) -> Option<&'a str> {
    let auto_review_model = relay.auto_review_model.trim();
    (!auto_review_model.is_empty() && model == CODEX_AUTO_REVIEW_MODEL)
        .then_some(auto_review_model)
}

async fn upstream_request_parts(
    relay: &crate::settings::RelayProfile,
    mut request_json: Value,
    request_path: &str,
    model_override: Option<&str>,
    preserve_compaction_trigger: bool,
    official_base: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
) -> anyhow::Result<(String, Value, UpstreamWireApi, bool)> {
    let compact = is_responses_compact_proxy_path(request_path)
        || request_has_compaction_trigger(&request_json);
    let is_v2_compaction = compact && request_has_compaction_trigger(&request_json);
    // Chat 与 Anthropic 上游都没有原生压缩项，只能走合成摘要路径。
    if relay.requires_protocol_proxy_conversion() && input_has_native_compaction(&request_json) {
        return Err(NativeCompactionUnsupported::history().into());
    }
    // 原生 v2 上游要先看到 compaction_trigger。只有确认它没有返回 compaction 项后才剥掉重试。
    if compact && !preserve_compaction_trigger {
        request_json = rewrite_request_for_compaction(strip_compaction_trigger(request_json));
        if input_has_native_compaction(&request_json) {
            return Err(NativeCompactionUnsupported::history().into());
        }
    }
    let synthetic_compaction = compact && relay.requires_protocol_proxy_conversion();
    if let Some(model) = model_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request_json["model"] = json!(model);
    }
    // issue #1446：「替我审批」固定请求 codex-auto-review，第三方上游不认识，
    // 供应商配置了审批模型时按供应商改写；未配置保持原样转发。
    if let Some(auto_review_model) = auto_review_model_rewrite(
        relay,
        request_json.get("model").and_then(Value::as_str).unwrap_or(""),
    ) {
        request_json["model"] = json!(auto_review_model);
    }
    let model = request_json.get("model").and_then(Value::as_str).unwrap_or("");
    let compatibility_enabled = relay.request_compatibility_enabled();
    let reasoning_mode = if compatibility_enabled { relay.model_reasoning_parameters(model) } else { "auto" }.to_string();
    let requested_effort = request_json.pointer("/reasoning/effort").cloned();
    if reasoning_mode == "omit" { strip_reasoning_parameters(&mut request_json); }
    if relay.protocol == RelayProtocol::Responses && relay.strict_responses_reasoning && compatibility_enabled {
        project_strict_reasoning_history(&mut request_json)?;
    }
    let mut body = match relay.protocol {
        RelayProtocol::Responses => request_json,
        RelayProtocol::ChatCompletions => responses_to_chat_completions_with_options(
            request_json,
            relay.standard_openai_protocol,
        )?,
        // Responses → Chat → Anthropic：第一跳复用上面同一份实现，第二跳在桥接模块里。
        RelayProtocol::AnthropicMessages => {
            crate::anthropic_bridge::responses_to_anthropic_messages(
                request_json,
                &crate::anthropic_bridge::request_options(relay),
            )?
        }
    };
    if relay.protocol == RelayProtocol::ChatCompletions
        && prompt_prefix_stabilizer_enabled(relay)
    {
        stabilize_chat_prompt_prefix(&mut body);
    }
    if reasoning_mode == "omit" {
        strip_reasoning_parameters(&mut body);
    } else if reasoning_mode == "passthrough" && relay.protocol == RelayProtocol::ChatCompletions {
        strip_reasoning_parameters(&mut body);
        if let Some(effort) = requested_effort { body["reasoning_effort"] = effort; }
    }
    if relay.protocol == RelayProtocol::Responses {
        let legacy_compact_path = is_responses_compact_proxy_path(request_path);
        // 官方登录上游（chatgpt.com 后端）自 2026-10 起要求：请求回放
        // `web_search_call` 历史时必须声明 web_search 工具，否则流式返回
        // `response protection is unavailable`。Codex 对自定义 provider 走本地压缩
        // （`tools` 为空），所以官方成员的每个压缩请求都会被拒。
        // 这里只对「完全没有声明工具」的官方登录请求自动补齐：补完 tool_choice 会被
        // 置为 none（模型不能真正调用搜索），带工具的普通轮次保持原样。
        let official_auto_repair = official_base.is_some()
            && !legacy_compact_path
            && request_declares_no_tools(&body);
        let opt_in_repair = relay.web_search_history_compat_enabled() && !legacy_compact_path;
        if (opt_in_repair || official_auto_repair) && ensure_web_search_tool_for_history(&mut body)
        {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.web_search_history_compat",
                json!({
                    "action": "declare_web_search",
                    "source": if official_auto_repair { "official_login" } else { "provider_opt_in" },
                    "inputContainsWebSearchCall": true
                }),
            );
        }
        dedupe_responses_tool_search_outputs(&mut body);
        normalize_responses_item_ids(&mut body);
    }

    // base64 图片一旦落进文本字段会把 token 估算撑到不可用，必须在 VLM 预处理前清掉。
    if guard_inline_image_data_urls(&mut body) {
        let model = body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "inline_image_data_url_leak",
            json!({ "model": model, "protocol": format!("{:?}", relay.protocol) }),
        );
    }

    // Image handling (per-model): send-as-is / strip / VLM analysis
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !model.is_empty() {
        use crate::vision::ImageHandling;
        match crate::vision::image_handling_mode(&model, &relay.model_vlm) {
            ImageHandling::SendAsIs => { /* 不做任何处理 */ }
            ImageHandling::Strip => {
                for key in &["messages", "input"] {
                    if let Some(arr) = body.get_mut(key).and_then(Value::as_array_mut) {
                        crate::vision::strip_images_only(arr);
                    }
                }
            }
            ImageHandling::Vlm => {
                    let vlm_config = crate::vision::VlmConfig {
                        api_key: relay.vlm_api_key.clone(),
                        model: relay.vlm_model.clone(),
                        base_url: relay.vlm_base_url.clone(),
                    };

                    for key in &["messages", "input"] {
                        if let Some(arr) = body.get_mut(key).and_then(Value::as_array_mut) {
                            crate::vision::strip_image_blocks_with_context(
                                arr,
                                &vlm_config,
                                &relay.model_windows,
                                &relay.context_window,
                                &model,
                                session_headers.thread_id,
                                relay.protocol == crate::settings::RelayProtocol::Responses,
                            )
                            .await?;
                        }
                    }
            }
        }
    }

    let wire_api = match relay.protocol {
        RelayProtocol::Responses => UpstreamWireApi::Responses,
        RelayProtocol::ChatCompletions => UpstreamWireApi::ChatCompletions,
        RelayProtocol::AnthropicMessages => UpstreamWireApi::AnthropicMessages,
    };
    // 官方登录成员没有用户填写的 base_url，固定发往 chatgpt.com 后端。
    let endpoint_base = official_base
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(relay.base_url.trim());
    Ok((
        match relay.protocol {
            // v2 压缩请求走普通 /responses 端点（compaction_trigger 在 input 里）；
            // 显式 legacy compact 保留原端点。
            RelayProtocol::Responses if compact && !is_v2_compaction => {
                responses_compact_url(endpoint_base)
            }
            RelayProtocol::Responses => responses_url(endpoint_base),
            RelayProtocol::ChatCompletions => chat_completions_url(endpoint_base),
            RelayProtocol::AnthropicMessages => {
                crate::anthropic_bridge::anthropic_messages_url(endpoint_base)
            }
        },
        body,
        wire_api,
        synthetic_compaction,
    ))
}

async fn send_responses_upstream_request(
    relay: &crate::settings::RelayProfile,
    endpoint: &str,
    upstream_body: &Value,
    wire_api: UpstreamWireApi,
    is_stream: bool,
    original_user_agent: Option<&str>,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
    official_credential: Option<&crate::official_login::OfficialCredential>,
) -> anyhow::Result<reqwest::Response> {
    let builder = build_responses_upstream_request(
        relay, endpoint, upstream_body, wire_api, is_stream,
        original_user_agent, beta_features, session_headers, official_credential,
    )?;
    send_upstream_request_for_responses(builder, is_stream).await
}

fn build_responses_upstream_request(
    relay: &crate::settings::RelayProfile,
    endpoint: &str,
    upstream_body: &Value,
    wire_api: UpstreamWireApi,
    is_stream: bool,
    original_user_agent: Option<&str>,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
    official_credential: Option<&crate::official_login::OfficialCredential>,
) -> anyhow::Result<reqwest::RequestBuilder> {
    // 伪装 Claude Code 时 UA 必须换成官方 CLI 的值，否则网关按 UA 判定客户端。
    let user_agent = if relay.protocol == crate::settings::RelayProtocol::AnthropicMessages
        && relay.anthropic_impersonate_claude_code
    {
        crate::anthropic_bridge::CLAUDE_CODE_USER_AGENT.to_string()
    } else {
        effective_user_agent(&relay.user_agent, original_user_agent)
    };
    let mut builder = upstream_request_builder(
        crate::http_client::proxied_client_for_profile(&user_agent, &relay.id)?,
        endpoint,
        relay,
        is_stream,
        upstream_body,
        official_credential,
    );
    builder = with_client_session_headers(builder, relay, session_headers);
    if relay.protocol == crate::settings::RelayProtocol::AnthropicMessages {
        // 1M 上下文 / Claude Code 身份都靠 anthropic-beta 表达；用户显式配了该头就不插手。
        let has_custom_beta = relay
            .custom_headers
            .iter()
            .any(|header| header.key.trim().eq_ignore_ascii_case("anthropic-beta"));
        if !has_custom_beta {
            let model = upstream_body
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("");
            let context_1m = crate::anthropic_bridge::model_declares_context_1m(relay, model);
            if let Some(beta) = crate::anthropic_bridge::anthropic_beta_header(
                relay.anthropic_impersonate_claude_code,
                context_1m,
            ) {
                builder = builder.header("anthropic-beta", beta);
            }
        }
        if relay.anthropic_impersonate_claude_code {
            builder = builder.header("x-app", "cli");
        }
    }
    if wire_api == UpstreamWireApi::Responses {
        if let Some(value) = beta_features.filter(|value| !value.is_empty()) {
            builder = builder.header("x-codex-beta-features", value);
        }
    }
    Ok(builder)
}

fn upstream_request_builder(
    client: reqwest::Client,
    endpoint: &str,
    relay: &crate::settings::RelayProfile,
    is_stream: bool,
    upstream_body: &Value,
    official_credential: Option<&crate::official_login::OfficialCredential>,
) -> reqwest::RequestBuilder {
    let mut builder = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    builder = match official_credential {
        Some(credential) => with_official_login_auth(builder, relay, credential),
        None => with_relay_auth(builder, relay),
    };
    if is_stream {
        builder = builder
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .header(reqwest::header::CACHE_CONTROL, "no-cache");
    }
    builder.json(upstream_body)
}

fn validate_upstream(relay: &crate::settings::RelayProfile) -> anyhow::Result<()> {
    // 纯官方 profile 没有 URL/Key：凭据齐全时走官方登录通道，否则给出可行动的错误。
    if crate::official_login::is_official_login_profile(relay) && relay.base_url.trim().is_empty()
    {
        if crate::official_login::official_login_available(relay) {
            return Ok(());
        }
        anyhow::bail!(
            "官方登录成员「{}」缺少 ChatGPT 登录凭据，请先在管理器中完成或导入官方登录",
            relay.name
        );
    }
    if relay.base_url.trim().is_empty() {
        anyhow::bail!("上游 Base URL 不能为空");
    }
    if relay.api_key.trim().is_empty() && !relay.uses_no_auth() {
        anyhow::bail!("上游 Key 不能为空");
    }
    Ok(())
}

fn with_relay_auth(
    request: reqwest::RequestBuilder,
    relay: &crate::settings::RelayProfile,
) -> reqwest::RequestBuilder {
    // 认证（API Key / 无认证）+ 供应商自定义请求头，统一在 relay_headers 里决定优先级。
    // Anthropic 上游的 `x-api-key` 与 `anthropic-version` 也在这里按协议注入。
    crate::relay_headers::apply_for_protocol(request, relay)
}

/// 官方登录成员的上游认证：OAuth Bearer + `chatgpt-account-id`，
/// 供应商自定义头照常保留（显式 `Authorization` 除外，官方身份不能被用户头覆盖）。
fn with_official_login_auth(
    mut builder: reqwest::RequestBuilder,
    relay: &crate::settings::RelayProfile,
    credential: &crate::official_login::OfficialCredential,
) -> reqwest::RequestBuilder {
    // originator 是官方后端识别客户端来源的固定头（Codex CLI 同值）。
    builder = builder
        .bearer_auth(credential.access_token.trim())
        .header("originator", "codex_cli_rs");
    if !credential.account_id.trim().is_empty() {
        builder = builder.header("chatgpt-account-id", credential.account_id.trim());
    }
    let custom_headers: Vec<crate::settings::RelayHeaderKeyValue> = relay
        .custom_headers
        .iter()
        .filter(|header| !header.key.trim().eq_ignore_ascii_case("authorization"))
        .cloned()
        .collect();
    crate::relay_headers::apply_headers(builder, &custom_headers)
}

fn with_client_session_headers(
    mut builder: reqwest::RequestBuilder,
    relay: &crate::settings::RelayProfile,
    headers: ProxySessionHeaders<'_>,
) -> reqwest::RequestBuilder {
    for (name, value) in [
        ("session-id", headers.session_id),
        ("thread-id", headers.thread_id),
        ("x-opencode-session", headers.opencode_session),
    ] {
        // 显式供应商配置优先，避免 reqwest 追加出两个同名头。
        if relay
            .custom_headers
            .iter()
            .any(|header| header.key.trim().eq_ignore_ascii_case(name))
        {
            continue;
        }
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            continue;
        };
        if let Ok(value) = reqwest::header::HeaderValue::from_str(value) {
            builder = builder.header(name, value);
        }
    }
    builder
}

fn conversation_id_from_responses_request(body: &Value) -> Option<String> {
    for key in ["conversation", "conversation_id", "previous_response_id"] {
        if let Some(value) = body.get(key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn effective_user_agent(configured_user_agent: &str, original_user_agent: Option<&str>) -> String {
    let configured_user_agent = configured_user_agent.trim();
    if !configured_user_agent.is_empty() {
        return configured_user_agent.to_string();
    }
    original_user_agent
        .map(str::trim)
        .filter(|user_agent| !user_agent.is_empty())
        .unwrap_or("")
        .to_string()
}

pub async fn handle_responses_proxy_request(body: &str) -> anyhow::Result<ProxyHttpResponse> {
    let request_json: Value = serde_json::from_str(body)?;
    let mut upstream = open_responses_proxy_request(body, None).await?;
    let is_compaction = upstream.compaction;
    let status_code = upstream.status_code;
    let upstream_content_type = upstream.content_type.clone();
    let is_stream = upstream.is_stream;
    let wire_api = upstream.wire_api;
    let upstream_body = upstream.read_body().await?;

    if !(200..300).contains(&status_code) {
        let error =
            responses_error_from_upstream(status_code, &upstream_content_type, &upstream_body);
        return Ok(ProxyHttpResponse {
            status: http_status_line(status_code),
            content_type: "application/json; charset=utf-8".to_string(),
            body: serde_json::to_vec(&error)?,
        });
    }

    if upstream.native_compaction_passthrough {
        return Ok(ProxyHttpResponse {
            status: "200 OK".to_string(),
            content_type: if upstream_content_type.is_empty() {
                if is_stream {
                    "text/event-stream; charset=utf-8".to_string()
                } else {
                    "application/json; charset=utf-8".to_string()
                }
            } else {
                upstream_content_type
            },
            body: upstream_body,
        });
    }

    if is_compaction {
        // v2 压缩：无论上游协议/是否流式，都重组为单个 compaction 输出项。
        let model = request_json
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !is_stream {
            let body = crate::anthropic_bridge::normalize_non_stream_body_for_compaction(
                wire_api,
                &upstream_body,
            )?;
            return Ok(ProxyHttpResponse {
                status: "200 OK".to_string(),
                content_type: "text/event-stream; charset=utf-8".to_string(),
                body: wrap_non_stream_response_as_compaction(&body, model)?,
            });
        }
        let mut converter = CompactionSseConverter::new(model);
        if wire_api == UpstreamWireApi::AnthropicMessages {
            // Anthropic 事件流先落成 chat 事件流，再交给既有的 chat 压缩聚合器。
            let chat_sse =
                crate::anthropic_bridge::anthropic_sse_to_chat_sse(&String::from_utf8_lossy(
                    &upstream_body,
                ));
            converter = converter.with_chat_upstream();
            converter.push_upstream_bytes(chat_sse.as_bytes());
        } else {
            if wire_api != UpstreamWireApi::Responses {
                converter = converter.with_chat_upstream();
            }
            // 整包已收齐，直接喂给有状态 SSE 解析器（与 launcher 逐 chunk 路径同逻辑）。
            converter.push_upstream_bytes(&upstream_body);
        }
        return Ok(ProxyHttpResponse {
            status: "200 OK".to_string(),
            content_type: "text/event-stream; charset=utf-8".to_string(),
            body: converter.finish(),
        });
    }

    if wire_api == UpstreamWireApi::Responses {
        let body = if upstream.exec_adapted {
            let (body, valid) = if is_stream {
                let mut converter = crate::responses_exec_sse::ExecSseAdapter::new();
                let mut bytes = match converter.push_bytes(&upstream_body) {
                    Ok(bytes) => bytes,
                    Err(_) => converter.fail(),
                };
                bytes.extend(converter.finish()?);
                (bytes, converter.succeeded())
            } else {
                crate::responses_exec_transport::adapted_json(&upstream_body)
            };
            if let Some(outcome) = upstream.exec_outcome.as_mut() {
                outcome.finish(valid);
            }
            body
        } else {
            upstream_body.to_vec()
        };
        let (body, invalid_native_web_search) =
            reject_invalid_native_web_search_response(&body, &upstream_content_type)?;
        return Ok(ProxyHttpResponse {
            status: if invalid_native_web_search {
                "502 Bad Gateway".to_string()
            } else {
                "200 OK".to_string()
            },
            content_type: if upstream_content_type.is_empty() {
                "application/json; charset=utf-8".to_string()
            } else {
                upstream_content_type
            },
            body,
        });
    }

    if wire_api == UpstreamWireApi::AnthropicMessages {
        // Anthropic → Chat → Responses：第二跳与 Chat 上游共用同一份映射。
        if is_stream {
            let text = String::from_utf8_lossy(&upstream_body);
            return Ok(ProxyHttpResponse {
                status: "200 OK".to_string(),
                content_type: "text/event-stream; charset=utf-8".to_string(),
                body: crate::anthropic_bridge::anthropic_sse_to_responses_sse(&text, &request_json),
            });
        }
        let message: Value = serde_json::from_slice(&upstream_body)?;
        let thinking_blocks = crate::anthropic_bridge::thinking_blocks_from_anthropic_message(&message);
        let chat_json = crate::anthropic_bridge::anthropic_message_to_chat_completion(message)?;
        let mut response_json = chat_completion_to_response_with_request(chat_json, &request_json)?;
        if !thinking_blocks.is_empty() {
            crate::anthropic_bridge::attach_thinking_to_responses(
                &mut response_json,
                &thinking_blocks,
            );
        }
        return Ok(ProxyHttpResponse {
            status: "200 OK".to_string(),
            content_type: "application/json; charset=utf-8".to_string(),
            body: serde_json::to_vec(&response_json)?,
        });
    }

    if is_stream {
        let text = String::from_utf8_lossy(&upstream_body);
        return Ok(ProxyHttpResponse {
            status: "200 OK".to_string(),
            content_type: "text/event-stream; charset=utf-8".to_string(),
            body: chat_sse_to_responses_sse_with_request(&text, &request_json).into_bytes(),
        });
    }

    let chat_json: Value = serde_json::from_slice(&upstream_body)?;
    let response_json = chat_completion_to_response_with_request(chat_json, &request_json)?;
    Ok(ProxyHttpResponse {
        status: "200 OK".to_string(),
        content_type: "application/json; charset=utf-8".to_string(),
        body: serde_json::to_vec(&response_json)?,
    })
}

pub fn chat_completions_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/chat/completions") {
        return base.to_string();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(base) || !origin_only {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

pub fn responses_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/responses") {
        return base.to_string();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(base) || !origin_only {
        format!("{base}/responses")
    } else {
        format!("{base}/v1/responses")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

pub fn responses_compact_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/responses/compact") {
        return base.to_string();
    }
    format!("{}/compact", responses_url(base_url).trim_end_matches('/'))
}

pub fn audio_transcriptions_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/audio/transcriptions") {
        return base.to_string();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(base) || !origin_only {
        format!("{base}/audio/transcriptions")
    } else {
        format!("{base}/v1/audio/transcriptions")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

pub fn image_generations_url(base_url: &str) -> String {
    image_endpoint_url(base_url, "generations")
}

pub fn image_edits_url(base_url: &str) -> String {
    image_endpoint_url(base_url, "edits")
}

fn image_endpoint_url(base_url: &str, endpoint: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base
        .to_ascii_lowercase()
        .ends_with(&format!("/images/{endpoint}"))
    {
        return base.to_string();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(base) || !origin_only {
        format!("{base}/images/{endpoint}")
    } else {
        format!("{base}/v1/images/{endpoint}")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

fn is_local_protocol_proxy_base_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else {
        return false;
    };
    if !url.scheme().eq_ignore_ascii_case("http") || url.port() != Some(protocol_proxy_port()) {
        return false;
    }
    matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
    )
}

#[cfg(test)]
mod image_proxy_tests {
    use super::UPSTREAM_IMAGE_HEADER_TIMEOUT;
    use super::is_local_protocol_proxy_base_url;
    use std::time::Duration;

    #[test]
    fn image_requests_allow_ten_minutes_for_response_headers() {
        assert_eq!(UPSTREAM_IMAGE_HEADER_TIMEOUT, Duration::from_secs(600));
    }

    #[test]
    fn local_protocol_proxy_detection_covers_common_loopback_forms() {
        for base_url in [
            "http://127.0.0.1:57321",
            "http://127.0.0.1:57321/",
            "http://127.0.0.1:57321/v1",
            "http://localhost:57321/v1/",
            "http://[::1]:57321/v1",
        ] {
            assert!(is_local_protocol_proxy_base_url(base_url), "{base_url}");
        }

        for base_url in [
            "https://127.0.0.1:57321/v1",
            "http://127.0.0.1:57322/v1",
            "http://api.example.test:57321/v1",
            "not-a-url",
        ] {
            assert!(!is_local_protocol_proxy_base_url(base_url), "{base_url}");
        }
    }
}

pub fn models_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let mut base = base_url
        .trim()
        .trim_end_matches('#')
        .trim_end_matches('/')
        .to_string();
    if base.to_ascii_lowercase().ends_with("/chat/completions") {
        base.truncate(base.len() - "/chat/completions".len());
    }
    if base.to_ascii_lowercase().ends_with("/models") {
        return base;
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(&base) || !origin_only {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

pub(crate) fn has_version_suffix(base_url: &str) -> bool {
    let segment = base_url.rsplit('/').next().unwrap_or(base_url);
    let Some(rest) = segment.strip_prefix('v') else {
        return false;
    };
    rest.chars().next().is_some_and(|ch| ch.is_ascii_digit())
}

pub fn chat_sse_to_responses_sse(input: &str) -> String {
    let mut converter = ChatSseToResponsesConverter::default();
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    String::from_utf8(output).unwrap_or_default()
}

pub fn chat_sse_to_responses_sse_with_request(input: &str, original_request: &Value) -> String {
    let mut converter = ChatSseToResponsesConverter::with_request(original_request);
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    String::from_utf8(output).unwrap_or_default()
}

pub fn response_id_from_chat_id(id: Option<&str>) -> String {
    let id = id.unwrap_or("compat");
    if id.starts_with("resp_") {
        id.to_string()
    } else {
        format!("resp_{id}")
    }
}

/// `resp_xxx` → `xxx`。message item 的 id 必须以 `msg_` 开头，
/// 直接拼在 `response_id` 后面会得到上游拒收的 `resp_xxx_msg`（#1431）。
fn response_id_body(response_id: &str) -> &str {
    response_id
        .strip_prefix("resp_")
        .filter(|value| !value.is_empty())
        .unwrap_or(response_id)
}

fn push_sse(output: &mut String, event: &str, data: Value) {
    output.push_str("event: ");
    output.push_str(event);
    output.push_str("\ndata: ");
    output.push_str(&serde_json::to_string(&data).unwrap_or_default());
    output.push_str("\n\n");
}

#[derive(Debug, Default)]
struct TextItemState {
    output_index: Option<u32>,
    item_id: String,
    text: String,
    added: bool,
    done: bool,
}

#[derive(Debug, Default)]
struct ReasoningItemState {
    output_index: Option<u32>,
    item_id: String,
    text: String,
    added: bool,
    done: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum InlineThinkMode {
    #[default]
    Detecting,
    Reasoning,
    Text,
}

#[derive(Debug, Default)]
struct InlineThinkState {
    mode: InlineThinkMode,
    buffer: String,
}

#[derive(Debug, Default)]
struct ToolCallState {
    output_index: Option<u32>,
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    added: bool,
    done: bool,
}

#[derive(Debug)]
struct ChatSseState {
    response_started: bool,
    completed: bool,
    response_id: String,
    model: String,
    created_at: u64,
    next_output_index: u32,
    text: TextItemState,
    reasoning: ReasoningItemState,
    inline_think: InlineThinkState,
    tools: BTreeMap<usize, ToolCallState>,
    output_items: Vec<(u32, Value)>,
    latest_usage: Option<Value>,
    finish_reason: Option<String>,
    tool_context: CodexToolContext,
    original_request: Option<Value>,
}

impl Default for ChatSseState {
    fn default() -> Self {
        Self {
            response_started: false,
            completed: false,
            response_id: "resp_compat".to_string(),
            model: String::new(),
            created_at: 0,
            next_output_index: 0,
            text: TextItemState::default(),
            reasoning: ReasoningItemState::default(),
            inline_think: InlineThinkState::default(),
            tools: BTreeMap::new(),
            output_items: Vec::new(),
            latest_usage: None,
            finish_reason: None,
            tool_context: CodexToolContext::default(),
            original_request: None,
        }
    }
}

impl ChatSseState {
    fn with_request(original_request: &Value) -> Self {
        let tool_context = chat_tool_projection(original_request).context;
        Self {
            tool_context,
            original_request: Some(original_request.clone()),
            ..Self::default()
        }
    }

    fn handle_chat_chunk_into(&mut self, chunk: &Value, output: &mut String) {
        if let Some(id) = chunk.get("id").and_then(Value::as_str) {
            self.response_id = response_id_from_chat_id(Some(id));
        }
        if let Some(model) = chunk.get("model").and_then(Value::as_str) {
            if !model.is_empty() {
                self.model = model.to_string();
            }
        }
        if let Some(created) = chunk.get("created").and_then(Value::as_u64) {
            self.created_at = created;
        }
        self.ensure_response_started_into(output);

        if let Some(usage) = chunk.get("usage").filter(|value| !value.is_null()) {
            self.latest_usage = Some(chat_usage_to_responses_usage(Some(usage)));
        }

        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return;
        };

        if let Some(delta) = choice.get("delta") {
            if let Some(reasoning) = chat_delta_reasoning_text(delta) {
                self.push_reasoning_delta_into(&reasoning, output);
            }

            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    self.push_content_delta_into(content, output);
                }
            }

            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                self.flush_inline_think_at_boundary_into(output);
                self.finalize_reasoning_into(output);
                for tool_call in tool_calls {
                    self.push_tool_call_delta_into(tool_call, output);
                }
            }
        }

        if let Some(finish_reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(finish_reason.to_string());
        }
    }

    fn push_content_delta_into(&mut self, delta: &str, output: &mut String) {
        match self.inline_think.mode {
            InlineThinkMode::Text => {
                self.finalize_reasoning_into(output);
                self.push_text_delta_into(delta, output);
            }
            InlineThinkMode::Detecting => {
                self.inline_think.buffer.push_str(delta);
                match leading_think_prefix_decision(&self.inline_think.buffer) {
                    ThinkPrefixDecision::NeedMore => {}
                    ThinkPrefixDecision::Reasoning => {
                        self.inline_think.mode = InlineThinkMode::Reasoning;
                        self.drain_complete_inline_think_into(output);
                    }
                    ThinkPrefixDecision::Text => {
                        self.inline_think.mode = InlineThinkMode::Text;
                        let text = std::mem::take(&mut self.inline_think.buffer);
                        self.finalize_reasoning_into(output);
                        self.push_text_delta_into(&text, output);
                    }
                }
            }
            InlineThinkMode::Reasoning => {
                self.inline_think.buffer.push_str(delta);
                self.drain_complete_inline_think_into(output);
            }
        }
    }

    fn drain_complete_inline_think_into(&mut self, output: &mut String) {
        let Some((reasoning, answer)) = split_leading_think_block(&self.inline_think.buffer) else {
            return;
        };
        self.inline_think.mode = InlineThinkMode::Text;
        self.inline_think.buffer.clear();
        if !reasoning.is_empty() {
            self.push_reasoning_delta_into(&reasoning, output);
            self.finalize_reasoning_into(output);
        }
        if !answer.is_empty() {
            self.push_text_delta_into(&answer, output);
        }
    }

    fn flush_inline_think_at_boundary_into(&mut self, output: &mut String) {
        match self.inline_think.mode {
            InlineThinkMode::Text => {}
            InlineThinkMode::Detecting => {
                self.inline_think.mode = InlineThinkMode::Text;
                let text = std::mem::take(&mut self.inline_think.buffer);
                if !text.is_empty() {
                    self.finalize_reasoning_into(output);
                    self.push_text_delta_into(&text, output);
                }
            }
            InlineThinkMode::Reasoning => {
                let buffered = std::mem::take(&mut self.inline_think.buffer);
                self.inline_think.mode = InlineThinkMode::Text;
                if let Some((reasoning, answer)) = split_leading_think_block(&buffered) {
                    if !reasoning.is_empty() {
                        self.push_reasoning_delta_into(&reasoning, output);
                        self.finalize_reasoning_into(output);
                    }
                    if !answer.is_empty() {
                        self.push_text_delta_into(&answer, output);
                    }
                    return;
                }
                let reasoning = strip_leading_think_open_tag(&buffered).unwrap_or(buffered);
                if !reasoning.is_empty() {
                    self.push_reasoning_delta_into(&reasoning, output);
                    self.finalize_reasoning_into(output);
                }
            }
        }
    }

    fn ensure_response_started_into(&mut self, output: &mut String) {
        if self.response_started {
            return;
        }
        self.response_started = true;
        push_sse(
            output,
            "response.created",
            json!({
                "type": "response.created",
                "response": self.base_response("in_progress", Vec::new())
            }),
        );
        push_sse(
            output,
            "response.in_progress",
            json!({
                "type": "response.in_progress",
                "response": self.base_response("in_progress", Vec::new())
            }),
        );
    }

    fn push_reasoning_delta_into(&mut self, delta: &str, output: &mut String) {
        if !self.reasoning.added {
            let output_index = self.next_output_index();
            let item_id = format!("rs_{}", self.response_id);
            self.reasoning.output_index = Some(output_index);
            self.reasoning.item_id = item_id.clone();
            self.reasoning.added = true;

            push_sse(
                output,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "id": item_id,
                        "type": "reasoning",
                        "status": "in_progress",
                        "reasoning_content": "",
                        "summary": []
                    }
                }),
            );
            push_sse(
                output,
                "response.reasoning_summary_part.added",
                json!({
                    "type": "response.reasoning_summary_part.added",
                    "item_id": self.reasoning.item_id,
                    "output_index": output_index,
                    "summary_index": 0,
                    "part": { "type": "summary_text", "text": "" }
                }),
            );
        }

        self.reasoning.text.push_str(delta);
        let output_index = self.reasoning.output_index.unwrap_or(0);
        push_sse(
            output,
            "response.reasoning_summary_text.delta",
            json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": self.reasoning.item_id,
                "output_index": output_index,
                "summary_index": 0,
                "delta": delta
            }),
        );
    }

    fn push_text_delta_into(&mut self, delta: &str, output: &mut String) {
        if !self.text.added {
            let output_index = self.next_output_index();
            let item_id = format!("msg_{}", response_id_body(&self.response_id));
            self.text.output_index = Some(output_index);
            self.text.item_id = item_id.clone();
            self.text.added = true;
            push_sse(
                output,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "id": item_id,
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": []
                    }
                }),
            );
            push_sse(
                output,
                "response.content_part.added",
                json!({
                    "type": "response.content_part.added",
                    "item_id": self.text.item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "part": { "type": "output_text", "text": "", "annotations": [] }
                }),
            );
        }

        self.text.text.push_str(delta);
        let output_index = self.text.output_index.unwrap_or(0);
        push_sse(
            output,
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "item_id": self.text.item_id,
                "output_index": output_index,
                "content_index": 0,
                "delta": delta
            }),
        );
    }

    fn push_tool_call_delta_into(&mut self, tool_call: &Value, output: &mut String) {
        let chat_index = tool_call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let id_delta = tool_call
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let function = tool_call.get("function").unwrap_or(&Value::Null);
        let name_delta = function
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let args_delta = function
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let mut should_add = false;
        let mut output_index = None;
        let mut item_id = String::new();
        let mut pending_arguments = String::new();

        {
            let state = self.tools.entry(chat_index).or_default();
            if let Some(id) = id_delta {
                state.call_id = id;
            }
            // 流式场景下 thought_signature/extra_content 通常随首个分片到达，
            // 此时 call_id 已就位；按 call_id 记住，供下一轮回传（issue #332/#1012）。
            if !state.call_id.is_empty() {
                remember_tool_call_extra_content(&state.call_id, tool_call);
            }
            if let Some(name) = name_delta {
                if !name.is_empty() {
                    state.name = name;
                }
            }
            if !args_delta.is_empty() {
                state.arguments.push_str(&args_delta);
            }

            // Custom tool output items must use the `ctc_` ID namespace. Some
            // Chat Completions providers send the call ID before the function
            // name, so wait for the name when the request includes custom tools
            // instead of emitting a provisional `function_call` with an `fc_`
            // ID that cannot later be replayed as a `custom_tool_call`.
            let waiting_for_custom_tool_name =
                self.tool_context.has_custom_tools && state.name.is_empty();
            if !state.added
                && (!state.call_id.is_empty() || !state.name.is_empty())
                && !waiting_for_custom_tool_name
            {
                should_add = true;
                pending_arguments = state.arguments.clone();
            } else if state.added {
                output_index = state.output_index;
                item_id = state.item_id.clone();
            }
        }

        if should_add {
            let assigned = self.next_output_index();
            let state = self.tools.get_mut(&chat_index).expect("tool state exists");
            state.added = true;
            if state.call_id.is_empty() {
                state.call_id = format!("call_{chat_index}");
            }
            if state.name.is_empty() {
                state.name = "unknown_tool".to_string();
            }
            state.output_index = Some(assigned);
            state.item_id = tool_call_item_id(&state.call_id, &state.name, &self.tool_context);
            let added_item = tool_call_added_item(state, assigned, &self.tool_context);
            push_sse(output, "response.output_item.added", added_item);
            if !pending_arguments.is_empty() {
                push_tool_call_delta_sse(
                    output,
                    state,
                    assigned,
                    &pending_arguments,
                    &self.tool_context,
                );
            }
        } else if !args_delta.is_empty() {
            if let Some(output_index) = output_index {
                let state = ToolCallState {
                    output_index: Some(output_index),
                    item_id,
                    name: self
                        .tools
                        .get(&chat_index)
                        .map(|state| state.name.clone())
                        .unwrap_or_default(),
                    call_id: self
                        .tools
                        .get(&chat_index)
                        .map(|state| state.call_id.clone())
                        .unwrap_or_default(),
                    ..ToolCallState::default()
                };
                push_tool_call_delta_sse(
                    output,
                    &state,
                    output_index,
                    &args_delta,
                    &self.tool_context,
                );
            }
        }
    }

    fn finalize_into(&mut self, output: &mut String) {
        if self.completed {
            return;
        }
        self.ensure_response_started_into(output);
        self.flush_inline_think_at_boundary_into(output);
        self.finalize_reasoning_into(output);
        self.finalize_text_into(output);
        self.finalize_tools_into(output);

        if self
            .output_items
            .iter()
            .any(|(_, item)| invalid_web_search_output_item(item))
        {
            let mut response = self.base_response("failed", self.completed_output_items());
            response["error"] = json!({
                "type": "invalid_upstream_tool_call",
                "message": "Chat 上游返回了未执行的原生 web_search 调用，请使用客户端搜索工具"
            });
            copy_response_request_fields(&mut response, self.original_request.as_ref());
            push_sse(
                output,
                "response.failed",
                json!({
                    "type": "response.failed",
                    "response": response
                }),
            );
            self.completed = true;
            return;
        }

        let status = response_status(self.finish_reason.as_deref());
        let mut response = self.base_response(status, self.completed_output_items());
        if status == "incomplete" {
            response["incomplete_details"] = json!({ "reason": "max_output_tokens" });
        }
        copy_response_request_fields(&mut response, self.original_request.as_ref());
        push_sse(
            output,
            "response.completed",
            json!({
                "type": "response.completed",
                "response": response
            }),
        );
        output.push_str("data: [DONE]\n\n");
        self.completed = true;
    }

    fn finalize_reasoning_into(&mut self, output: &mut String) {
        if !self.reasoning.added || self.reasoning.done {
            return;
        }
        let output_index = self.reasoning.output_index.unwrap_or(0);
        let item = json!({
            "id": self.reasoning.item_id,
            "type": "reasoning",
            "reasoning_content": self.reasoning.text,
            "summary": [{ "type": "summary_text", "text": self.reasoning.text }]
        });
        self.output_items.push((output_index, item.clone()));
        self.reasoning.done = true;
        push_sse(
            output,
            "response.reasoning_summary_text.done",
            json!({
                "type": "response.reasoning_summary_text.done",
                "item_id": self.reasoning.item_id,
                "output_index": output_index,
                "summary_index": 0,
                "text": self.reasoning.text
            }),
        );
        push_sse(
            output,
            "response.reasoning_summary_part.done",
            json!({
                "type": "response.reasoning_summary_part.done",
                "item_id": self.reasoning.item_id,
                "output_index": output_index,
                "summary_index": 0,
                "part": { "type": "summary_text", "text": self.reasoning.text }
            }),
        );
        push_sse(
            output,
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": item
            }),
        );
    }

    fn finalize_text_into(&mut self, output: &mut String) {
        if !self.text.added || self.text.done {
            return;
        }
        let output_index = self.text.output_index.unwrap_or(0);
        let item = json!({
            "id": self.text.item_id,
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": self.text.text, "annotations": [] }]
        });
        self.output_items.push((output_index, item.clone()));
        self.text.done = true;
        push_sse(
            output,
            "response.output_text.done",
            json!({
                "type": "response.output_text.done",
                "item_id": self.text.item_id,
                "output_index": output_index,
                "content_index": 0,
                "text": self.text.text
            }),
        );
        push_sse(
            output,
            "response.content_part.done",
            json!({
                "type": "response.content_part.done",
                "item_id": self.text.item_id,
                "output_index": output_index,
                "content_index": 0,
                "part": { "type": "output_text", "text": self.text.text, "annotations": [] }
            }),
        );
        push_sse(
            output,
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": item
            }),
        );
    }

    fn finalize_tools_into(&mut self, output: &mut String) {
        let keys: Vec<usize> = self.tools.keys().copied().collect();
        for key in keys {
            if self.tools.get(&key).map(|state| state.done).unwrap_or(true) {
                continue;
            }
            if self
                .tools
                .get(&key)
                .map(|state| !state.added && !state.done)
                .unwrap_or(false)
            {
                let assigned = self.next_output_index();
                let state = self.tools.get_mut(&key).expect("tool state exists");
                state.added = true;
                if state.call_id.is_empty() {
                    state.call_id = format!("call_{key}");
                }
                if state.name.is_empty() {
                    state.name = "unknown_tool".to_string();
                }
                state.output_index = Some(assigned);
                state.item_id = tool_call_item_id(&state.call_id, &state.name, &self.tool_context);
                let added_item = tool_call_added_item(state, assigned, &self.tool_context);
                push_sse(output, "response.output_item.added", added_item);
            }

            let state = self.tools.get_mut(&key).expect("tool state exists");
            let output_index = state.output_index.unwrap_or(0);
            let item = tool_call_done_item(state, &self.tool_context);
            state.done = true;
            self.output_items.push((output_index, item.clone()));
            push_tool_call_done_sse(output, state, output_index, &self.tool_context);
            push_sse(
                output,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": item
                }),
            );
        }
    }

    fn failed_into(&mut self, output: &mut String, message: String, error_type: Option<String>) {
        self.completed = true;
        let mut error = json!({ "message": message });
        if let Some(error_type) = error_type.filter(|value| !value.is_empty()) {
            error["type"] = json!(error_type);
        }
        let mut response = self.base_response("failed", self.completed_output_items());
        response["error"] = error;
        push_sse(
            output,
            "response.failed",
            json!({
                "type": "response.failed",
                "response": response
            }),
        );
    }

    fn completed_output_items(&self) -> Vec<Value> {
        let mut output_items = self.output_items.clone();
        output_items.sort_by_key(|(output_index, _)| *output_index);
        output_items.into_iter().map(|(_, item)| item).collect()
    }

    fn base_response(&self, status: &str, output: Vec<Value>) -> Value {
        json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "model": self.model,
            "output": output,
            "usage": self.latest_usage.clone().unwrap_or_else(default_responses_usage)
        })
    }

    fn next_output_index(&mut self) -> u32 {
        let index = self.next_output_index;
        self.next_output_index += 1;
        index
    }
}

fn take_sse_block(buffer: &mut String) -> Option<String> {
    let lf = buffer.find("\n\n").map(|index| (index, 2));
    let crlf = buffer.find("\r\n\r\n").map(|index| (index, 4));
    let (index, delimiter_len) = match (lf, crlf) {
        (Some(left), Some(right)) => {
            if left.0 <= right.0 {
                left
            } else {
                right
            }
        }
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
    let mut combined = Vec::new();
    if !remainder.is_empty() {
        combined.extend_from_slice(remainder);
        remainder.clear();
    }
    combined.extend_from_slice(bytes);

    match std::str::from_utf8(&combined) {
        Ok(text) => buffer.push_str(text),
        Err(error) => {
            let valid = error.valid_up_to();
            if valid > 0 {
                buffer.push_str(std::str::from_utf8(&combined[..valid]).unwrap_or_default());
            }
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

fn chat_delta_reasoning_text(delta: &Value) -> Option<String> {
    extract_reasoning_field_text(delta)
}

enum ThinkPrefixDecision {
    NeedMore,
    Reasoning,
    Text,
}

fn leading_think_prefix_decision(buffer: &str) -> ThinkPrefixDecision {
    let trimmed = buffer.trim_start();
    if trimmed.is_empty() {
        return ThinkPrefixDecision::NeedMore;
    }
    if trimmed.starts_with(THINK_OPEN_TAG) {
        return ThinkPrefixDecision::Reasoning;
    }
    if THINK_OPEN_TAG.starts_with(trimmed) {
        return ThinkPrefixDecision::NeedMore;
    }
    ThinkPrefixDecision::Text
}

fn extract_chat_sse_error(value: &Value) -> (String, Option<String>) {
    let error = value.get("error").unwrap_or(value);
    let message = error
        .as_str()
        .map(ToString::to_string)
        .or_else(|| {
            error
                .get("message")
                .or_else(|| error.get("detail"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| error.to_string());
    let error_type = error
        .get("type")
        .or_else(|| error.get("code"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    (message, error_type)
}

fn http_status_line(status: u16) -> String {
    match status {
        200 => "200 OK".to_string(),
        400 => "400 Bad Request".to_string(),
        401 => "401 Unauthorized".to_string(),
        403 => "403 Forbidden".to_string(),
        404 => "404 Not Found".to_string(),
        429 => "429 Too Many Requests".to_string(),
        500 => "500 Internal Server Error".to_string(),
        502 => "502 Bad Gateway".to_string(),
        503 => "503 Service Unavailable".to_string(),
        _ => format!("{status} Upstream"),
    }
}

pub fn responses_error_from_upstream(status_code: u16, content_type: &str, body: &[u8]) -> Value {
    let (message, error_type, code, param) = upstream_error_parts(status_code, content_type, body);
    let mut error = json!({
        "message": message,
        "type": error_type.unwrap_or_else(|| "upstream_error".to_string()),
    });
    if let Some(code) = code {
        error["code"] = json!(code);
    }
    if let Some(param) = param {
        error["param"] = json!(param);
    }
    json!({ "error": error })
}

fn upstream_error_parts(
    status_code: u16,
    content_type: &str,
    body: &[u8],
) -> (String, Option<String>, Option<String>, Option<String>) {
    if content_type.to_ascii_lowercase().contains("json") {
        if let Ok(value) = serde_json::from_slice::<Value>(body) {
            let error = value.get("error").unwrap_or(&value);
            let message = error
                .get("message")
                .or_else(|| error.get("detail"))
                .or_else(|| error.get("error"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| truncate_error_preview(&value.to_string()));
            let error_type = error
                .get("type")
                .or_else(|| error.get("error_type"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let code = error.get("code").and_then(|value| {
                value
                    .as_str()
                    .map(ToString::to_string)
                    .or_else(|| value.as_i64().map(|number| number.to_string()))
            });
            let param = error
                .get("param")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            return (message, error_type, code, param);
        }
    }

    let preview = truncate_error_preview(&String::from_utf8_lossy(body));
    let message = if preview.trim().is_empty() {
        format!("Upstream returned HTTP {status_code}")
    } else {
        preview
    };
    (message, None, Some(status_code.to_string()), None)
}

fn truncate_error_preview(input: &str) -> String {
    input.chars().take(ERROR_BODY_PREVIEW_LIMIT).collect()
}

/// Responses 协议要求每个 input item 的 `id` 前缀与它的 `type` 对应，
/// 例如 `message` 必须是 `msg_`、`function_call` 必须是 `fc_`。
/// 前缀不匹配时上游直接以 `[ApiIdParam] [input[N].id] [invalid_id_prefix]` 拒绝**整份**请求，
/// 于是这段历史被反复重放、会话永久不可用（#1431 / #1781 / #1796）。
///
/// 前缀表取自 Codex 自己的 rollout 记录（`~/.codex/sessions/**/rollout-*.jsonl`），
/// 不是猜测：`message`/`reasoning`/`custom_tool_call`/`custom_tool_call_output`/
/// `function_call`/`function_call_output` 分别对应 msg_/rs_/ctc_/ctco_/fc_/fco_。
const RESPONSES_ITEM_ID_PREFIXES: &[(&str, &str)] = &[
    ("message", "msg_"),
    ("reasoning", "rs_"),
    ("compaction", "cmp_"),
    ("function_call", "fc_"),
    ("function_call_output", "fco_"),
    ("custom_tool_call", "ctc_"),
    ("custom_tool_call_output", "ctco_"),
];

/// 供集成测试直接验证前缀归一结果：`upstream_request_parts` 需要真实网络，
/// 用它测不方便。
#[doc(hidden)]
pub fn normalize_responses_item_ids_for_test(body: &mut Value) {
    normalize_responses_item_ids(body);
}

/// 出站前把所有已知 item 的 id 前缀修正到与 `type` 一致。
///
/// 只认前缀表里的类型，未知类型原样通过——宁可放过，也不要把看不出类型语义的 id 改坏。
fn normalize_responses_item_ids(body: &mut Value) {
    let Some(input) = body.get_mut("input") else {
        return;
    };
    match input {
        Value::Array(items) => {
            for item in items {
                normalize_responses_item_id(item);
            }
        }
        Value::Object(_) => normalize_responses_item_id(input),
        _ => {}
    }
}

/// Responses 上游要求同一请求里的 namespace 名称唯一。
/// Codex 客户端在多轮 tool_search 后可能把多个历史 `tool_search_output` 都回放到
/// 下一请求（#2156 / #2166）。同名工具/namespace 只在**第一次声明它的 output** 里
/// 保留一份，并合并后续检索新增的子工具；后面的 output 去掉重复项但**本身保留**，
/// 否则对应的 `tool_search_call` 没有输出配对，严格上游会换成另一个 400。
fn dedupe_responses_tool_search_outputs(body: &mut Value) {
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    let tool_key = |tool: &Value| -> Option<(String, String)> {
        let name = tool.get("name").and_then(Value::as_str).filter(|name| !name.is_empty())?;
        let kind = tool.get("type").and_then(Value::as_str).unwrap_or("");
        Some((kind.to_string(), name.to_string()))
    };
    let mut merged_namespaces = Vec::new();
    let mut output_count = 0;
    for item in items.iter() {
        if item.get("type").and_then(Value::as_str) != Some("tool_search_output") {
            continue;
        }
        output_count += 1;
        for tool in item.get("tools").and_then(Value::as_array).into_iter().flatten() {
            merge_namespace_definition(&mut merged_namespaces, tool);
        }
    }
    if output_count < 2 {
        return;
    }
    let mut declared = std::collections::HashSet::new();
    for item in items.iter_mut() {
        if item.get("type").and_then(Value::as_str) != Some("tool_search_output") {
            continue;
        }
        let Some(tools) = item.get("tools").and_then(Value::as_array).cloned() else {
            continue;
        };
        let mut kept = Vec::new();
        for tool in tools {
            let Some(key) = tool_key(&tool) else {
                kept.push(tool);
                continue;
            };
            if !declared.insert(key.clone()) {
                continue;
            }
            let merged = merged_namespaces
                .iter()
                .find(|candidate| tool_key(candidate).as_ref() == Some(&key))
                .cloned()
                .unwrap_or(tool);
            kept.push(merged);
        }
        item["tools"] = Value::Array(kept);
    }
}

fn normalize_responses_item_id(item: &mut Value) {
    let Some(item_type) = item.get("type").and_then(Value::as_str) else {
        return;
    };
    let Some((_, prefix)) = RESPONSES_ITEM_ID_PREFIXES
        .iter()
        .find(|(kind, _)| *kind == item_type)
    else {
        return;
    };
    let Some(id) = item.get("id").and_then(Value::as_str) else {
        return;
    };
    // 只有「前缀 + 非空后缀」才算已经合规。id 恰好等于前缀本身（`fc_`）是空壳，
    // 放它过去会直接触发上游的 invalid_id_prefix，所以落到下面按 call_id 重建。
    if id.len() > prefix.len() && id.starts_with(prefix) {
        return;
    }
    // 剥掉 id 上现有的前缀再换新的。取**最长**匹配：`fc_` 是 `fco_` 的前缀，
    // 先撞上短的会把 `fco_call_a` 剥成 `call_a` 再换成 `fc_call_a`，
    // 把 function_call_output 错改成 function_call。
    // `item_` 是历史版本 Codex++ 自己造的前缀；`cp_` 来自压缩项错误使用
    // `cp_{response_id}` 的版本；`resp_` 来自历史版本把 message item 命名成
    // `{response_id}_msg`（#1431 / #1781），都要一并剥掉。
    let suffix = ["item_", "cp_", "resp_"]
        .into_iter()
        .chain(RESPONSES_ITEM_ID_PREFIXES.iter().map(|(_, known)| *known))
        .filter_map(|known| id.strip_prefix(known).map(|rest| (known.len(), rest)))
        .max_by_key(|(len, _)| *len)
        .map(|(_, rest)| rest)
        .unwrap_or(id);
    // 剥完是空串说明 id 恰好只由某个前缀组成，退回 call_id，再退回原 id。
    let suffix = if suffix.is_empty() {
        item.get("call_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(id)
    } else {
        suffix
    };
    item["id"] = json!(format!("{prefix}{suffix}"));
}

fn append_responses_input(input: &Value, messages: &mut Vec<Value>) -> anyhow::Result<()> {
    // encrypted_content 片段可能携带无法在 Chat 协议里表达的 opaque 内容，报错向上传播（fail loud）。
    // 错误最终在 launcher 层变成带可读 message 的失败响应，不会静默发空任务。
    match input {
        Value::String(text) => messages.push(json!({ "role": "user", "content": text })),
        Value::Array(items) => {
            let mut pending_tool_calls = Vec::new();
            let mut pending_reasoning = Vec::new();
            let mut seen_tool_call_ids = BTreeSet::new();
            // 同一次调用的输出在历史里出现多次时，Chat 上游会以
            // `duplicate tool output for call_id` 拒绝整轮请求（#1533）。
            // 只对「同一次调用」的重复输出去重：不返回 id 的上游每轮都由本代理生成
            // `call_0`，跨轮复用 id 是合法的，遇到新的同 id 调用时重新计数。
            let mut seen_tool_output_ids = BTreeSet::new();
            for item in items {
                append_responses_item(
                    item,
                    messages,
                    &mut pending_tool_calls,
                    &mut pending_reasoning,
                    &mut seen_tool_call_ids,
                    &mut seen_tool_output_ids,
                )?;
            }
            flush_tool_calls(messages, &mut pending_tool_calls, &mut pending_reasoning);
            flush_reasoning(messages, &mut pending_reasoning);
        }
        Value::Object(_) => {
            let mut pending_tool_calls = Vec::new();
            let mut pending_reasoning = Vec::new();
            let mut seen_tool_call_ids = BTreeSet::new();
            let mut seen_tool_output_ids = BTreeSet::new();
            append_responses_item(
                input,
                messages,
                &mut pending_tool_calls,
                &mut pending_reasoning,
                &mut seen_tool_call_ids,
                &mut seen_tool_output_ids,
            )?;
            flush_tool_calls(messages, &mut pending_tool_calls, &mut pending_reasoning);
            flush_reasoning(messages, &mut pending_reasoning);
        }
        _ => {}
    }
    Ok(())
}

fn append_responses_item(
    item: &Value,
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    pending_reasoning: &mut Vec<String>,
    seen_tool_call_ids: &mut BTreeSet<String>,
    seen_tool_output_ids: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    match item.get("type").and_then(Value::as_str) {
        Some("function_call") => {
            let name = responses_history_function_name(item);
            if name.is_empty() {
                return Ok(());
            }
            let call_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return Ok(());
            }
            seen_tool_call_ids.insert(call_id.to_string());
            seen_tool_output_ids.remove(call_id);
            pending_tool_calls.push(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": responses_arguments_to_chat(item.get("arguments").unwrap_or(&json!({})))
                }
            }));
            // 把上一轮上游附带的透传数据挂回（Gemini thought_signature 等）。
            // 缺失时保持原样，不影响其它供应商。
            if let Some(extra) = recall_tool_call_extra_content(call_id) {
                if let Some(last) = pending_tool_calls.last_mut() {
                    last["extra_content"] = extra;
                }
            }
        }
        Some("function_call_output") => {
            let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
            if call_id.is_empty() {
                return Ok(());
            }
            if !seen_tool_output_ids.insert(call_id.to_string()) {
                // 重复的 tool output：保留第一次，避免上游 duplicate call_id 400。
                return Ok(());
            }
            if !seen_tool_call_ids.contains(call_id) {
                flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
                flush_reasoning(messages, pending_reasoning);
                messages.push(orphan_tool_output_message(
                    call_id,
                    item.get("output").unwrap_or(&Value::Null),
                ));
                return Ok(());
            }
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": tool_output_content(item.get("output").unwrap_or(&Value::Null))
            }));
        }
        Some("custom_tool_call") => {
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            let input = item
                .get("input")
                .or_else(|| item.get("arguments"))
                .unwrap_or(&Value::Null);
            let (name, arguments) = build_custom_tool_call_history(name, input);
            let call_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return Ok(());
            }
            seen_tool_call_ids.insert(call_id.to_string());
            seen_tool_output_ids.remove(call_id);
            pending_tool_calls.push(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": arguments
                }
            }));
        }
        Some("tool_search_call") => {
            // Codex 的 tool_search 是客户端执行的代理工具，历史回放时按
            // function tool_call 形态映射进 chat 消息流。
            let call_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return Ok(());
            }
            seen_tool_call_ids.insert(call_id.to_string());
            seen_tool_output_ids.remove(call_id);
            pending_tool_calls.push(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": "tool_search",
                    "arguments": responses_arguments_to_chat(
                        item.get("arguments").unwrap_or(&json!({}))
                    )
                }
            }));
        }
        Some("tool_search_output") => {
            let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
            if call_id.is_empty() {
                return Ok(());
            }
            if !seen_tool_output_ids.insert(call_id.to_string()) {
                return Ok(());
            }
            let output = item.get("tools").unwrap_or(&Value::Null);
            if !seen_tool_call_ids.contains(call_id) {
                flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
                flush_reasoning(messages, pending_reasoning);
                messages.push(orphan_tool_output_message(call_id, output));
                return Ok(());
            }
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": tool_output_content(output)
            }));
        }
        Some("custom_tool_call_output") => {
            let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
            if call_id.is_empty() {
                return Ok(());
            }
            if !seen_tool_call_ids.contains(call_id) {
                flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
                flush_reasoning(messages, pending_reasoning);
                messages.push(orphan_tool_output_message(
                    call_id,
                    item.get("output").unwrap_or(&Value::Null),
                ));
                return Ok(());
            }
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": tool_output_content(item.get("output").unwrap_or(&Value::Null))
            }));
        }
        Some("tool_call") => {
            if let Some(tool_use) = item.get("tool_use") {
                let call_id = tool_use
                    .get("id")
                    .or_else(|| item.get("call_id"))
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if call_id.is_empty() {
                    return Ok(());
                }
                seen_tool_call_ids.insert(call_id.to_string());
                seen_tool_output_ids.remove(call_id);
            seen_tool_output_ids.remove(call_id);
                pending_tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": tool_use.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments": responses_arguments_to_chat(tool_use.get("input").unwrap_or(&json!({})))
                    }
                }));
            }
        }
        Some("tool_result") => {
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            let content = item.get("content").unwrap_or(&Value::Null);
            let call_id = content
                .get("tool_use_id")
                .or_else(|| item.get("tool_call_id"))
                .or_else(|| item.get("call_id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return Ok(());
            }
            let output = content.get("content").unwrap_or(content);
            if !seen_tool_call_ids.contains(call_id) {
                flush_reasoning(messages, pending_reasoning);
                messages.push(orphan_tool_output_message(call_id, output));
                return Ok(());
            }
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": tool_output_content(output)
            }));
        }
        Some("reasoning") => {
            if let Some(text) = responses_reasoning_text(item) {
                if !text.is_empty() {
                    pending_reasoning.push(text);
                }
            }
        }
        Some("encrypted_content") => {
            return Err(UnsupportedEncryptedAgentContent.into());
        }
        Some(COMPACTION_TRIGGER_TYPE) => {
            // 控制项不进上游历史；正常请求不该出现，出现即忽略。
        }
        _ => {
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            if let Some(content) = item.get("content") {
                let role = responses_role_to_chat_role(item.get("role").and_then(Value::as_str));
                if content.is_null() && role != "assistant" {
                    return Ok(());
                }
                let allow_plaintext_encrypted_content = item.get("type").and_then(Value::as_str)
                    == Some("agent_message")
                    && agent_message_has_confirmed_plaintext_shape(content);
                let mut message = json!({
                    "role": role,
                    "content": responses_content_to_chat_content(
                        role,
                        content,
                        allow_plaintext_encrypted_content,
                    )?
                });
                if role == "assistant" {
                    if !pending_reasoning.is_empty() && pending_tool_calls.is_empty() {
                        message["reasoning_content"] =
                            json!(std::mem::take(pending_reasoning).join("\n"));
                    }
                } else if !pending_reasoning.is_empty() {
                    flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
                    flush_reasoning(messages, pending_reasoning);
                }
                messages.push(message);
            }
        }
    }
    Ok(())
}

fn orphan_tool_output_message(call_id: &str, output: &Value) -> Value {
    // 这条已经是 user 消息，multi-part 图片可以直接内联，不必走 relocate。
    if let Value::Array(parts) = tool_output_content(output) {
        let mut content = vec![json!({
            "type": "text",
            "text": format!("Function call output ({call_id}):")
        })];
        content.extend(parts);
        return json!({ "role": "user", "content": content });
    }
    json!({
        "role": "user",
        "content": format!(
            "Function call output ({call_id}): {}",
            response_output_text(output)
        )
    })
}

/// Chat Completions 上游（DeepSeek thinking 模式尤其严格）要求带 `tool_calls` 的
/// assistant 消息后面必须紧跟每个 `tool_call_id` 对应的 `tool` 消息。中断/回滚过的
/// 一轮会话可能留下没有 output 的 `function_call`，直接转发会被上游 400
/// （insufficient tool messages following tool_calls message）。
///
/// 这里把没有配对 output 的 tool_call 从消息里摘掉，降级成文本保留在历史中，
/// 避免丢失「模型曾试图调用某工具」这一信息。
/// 把插在「连续 tool 结果」之间的非 tool 消息整体搬到该 tool 区之后
/// （issue #2275 / #2257）。
///
/// 上游 codex 会把 `<image_resize_notice>` 这类提示以 developer（映射成 system）
/// 或 user 消息的形式插在两条 tool 结果之间，形成夹心结构：
/// `assistant(tool_calls=[a,b]) → tool(a) → developer → tool(b)`。
/// `enforce_tool_call_pairing` 用 `take_while` 只收集「role 连续为 tool」的后续消息，
/// 数到夹心就停，于是 followers 只有 1 条、`b` 被误判 orphaned 并从 `tool_calls`
/// 摘掉，但 `b` 的 tool 消息还留在原地 —— 这正是「role 'tool' 无前置 tool_calls」
/// 与「No tool output found for tool call」的来源。
///
/// 这里在配对判定**之前**把夹心消息移到 tool 区之后，使 tool 结果重新连续。
/// system 形态的夹心会被后续的 `collapse_system_messages_to_head` 带到头部（合法），
/// user 形态的则留在 tool 区之后（同样合法）。
fn relocate_interleaved_non_tool_messages(messages: &mut Vec<Value>) {
    let mut index = 0;
    while index < messages.len() {
        let Some(tool_calls) = messages[index].get("tool_calls").and_then(Value::as_array) else {
            index += 1;
            continue;
        };
        let mut unanswered: BTreeSet<String> = tool_calls
            .iter()
            .filter_map(|tool_call| tool_call.get("id").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
        if unanswered.is_empty() {
            index += 1;
            continue;
        }

        // 从本条 assistant 往后扫，收集本轮的所有 tool 结果与夹在其中的非 tool 消息，
        // 直到 tool_call 集合配齐、撞上下一条 assistant（新轮次，不能越界）、或到底。
        let mut tool_messages: Vec<Value> = Vec::new();
        let mut interleaved: Vec<Value> = Vec::new();
        let mut scan = index + 1;
        while scan < messages.len() {
            let role = messages[scan].get("role").and_then(Value::as_str);
            if role == Some("assistant") {
                break;
            }
            if role == Some("tool") {
                if let Some(id) = messages[scan].get("tool_call_id").and_then(Value::as_str) {
                    unanswered.remove(id);
                }
                tool_messages.push(messages[scan].clone());
            } else if !tool_messages.is_empty() {
                // 只有已经收到过 tool 结果之后的夹心才值得搬：
                // 本条 assistant 尚未收到任何结果时，中间的消息是正常历史，不是夹心。
                interleaved.push(messages[scan].clone());
            }
            scan += 1;
            if unanswered.is_empty() {
                break;
            }
        }

        if interleaved.is_empty() {
            index += 1;
            continue;
        }

        // 重建这段区间：tool 结果。配齐时）在前、夹心消息在后。
        let rebuilt: Vec<Value> = tool_messages.into_iter().chain(interleaved).collect();
        let rebuilt_len = rebuilt.len();
        messages.splice(index + 1..scan, rebuilt);
        // 跳过刚重建的区间，避免在搬动过的消息上重复扫描导致死循环。
        index += 1 + rebuilt_len;
    }
}

/// 把没有前置 `tool_calls` 的 tool 消息降级成 user（issue #2275 / #2257 的防线）。
///
/// `enforce_tool_call_pairing` 目前只清理 assistant 侧（把 orphaned 的 tool_call
/// 从 `tool_calls` 摘掉），被摘掉的那些 tool 消息本身仍留在原位，上游会直接
/// 400「No tool output found for tool call」。这里把它们降级成 user，保住内容
/// 且不再触发协议错误。
fn degrade_unpaired_tool_messages(messages: &mut [Value]) {
    let mut known: BTreeSet<String> = BTreeSet::new();
    for message in messages.iter() {
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
                    known.insert(id.to_string());
                }
            }
        }
    }

    for message in messages.iter_mut() {
        if message.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        let paired = message
            .get("tool_call_id")
            .and_then(Value::as_str)
            .is_some_and(|id| known.contains(id));
        if paired {
            continue;
        }
        let content = message.get("content").cloned().unwrap_or(Value::Null);
        *message = json!({
            "role": "user",
            "content": content
        });
    }
}

fn enforce_tool_call_pairing(messages: &mut [Value]) {
    let mut index = 0;
    while index < messages.len() {
        if messages[index].get("role").and_then(Value::as_str) != Some("assistant") {
            index += 1;
            continue;
        }
        let Some(tool_calls) = messages[index].get("tool_calls").and_then(Value::as_array) else {
            index += 1;
            continue;
        };
        if tool_calls.is_empty() {
            index += 1;
            continue;
        }

        // 收集紧跟其后的 tool 消息所应答的 id
        let mut answered = BTreeSet::new();
        let mut followers = 0;
        for message in messages[index + 1..]
            .iter()
            .take_while(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        {
            followers += 1;
            if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
                answered.insert(id.to_string());
            }
        }

        // 位于历史尾部的 tool_call 是「刚发起、output 还没回来」的正常形态，
        // 上游本就期待它；只有序列越过了它却没应答才是非法的。
        if index + 1 + followers >= messages.len() {
            index += 1;
            continue;
        }

        let (kept, orphaned): (Vec<Value>, Vec<Value>) =
            tool_calls.iter().cloned().partition(|tool_call| {
                tool_call
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| answered.contains(id))
            });
        if orphaned.is_empty() {
            index += 1;
            continue;
        }

        let notes = orphaned
            .iter()
            .map(|tool_call| {
                let name = tool_call
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let id = tool_call.get("id").and_then(Value::as_str).unwrap_or("");
                format!("Abandoned function call ({id}): {name}")
            })
            .collect::<Vec<_>>()
            .join("\n");

        if kept.is_empty() {
            if let Some(message) = messages[index].as_object_mut() {
                message.remove("tool_calls");
            }
        } else {
            messages[index]["tool_calls"] = json!(kept);
        }
        append_text_to_assistant_message(&mut messages[index], &notes);
        index += 1;
    }
}

/// `role:"tool"` 消息在多数 Chat Completions 上游（DeepSeek 在内）只接受字符串 content，
/// 塞 multi-part `image_url` 会被 400；而 `enforce_tool_call_pairing` 又要求 tool 消息
/// 紧跟在 assistant(tool_calls) 之后**连续**排列 —— 把图片消息插在两条 tool 消息中间，
/// 会让后面那条不再被计入 followers，对应的 tool_call 被误判成 orphaned 而摘掉。
///
/// 两个约束都要满足，所以这里的做法是：tool 消息本身降级成文本占位，图片一路收集到
/// 连续 tool 区结束，再作为一条 user 消息整体插在其后。
fn relocate_tool_output_images(messages: &mut Vec<Value>) {
    let mut index = 0;
    while index < messages.len() {
        if messages[index].get("role").and_then(Value::as_str) != Some("tool") {
            index += 1;
            continue;
        }
        let mut images = Vec::new();
        let mut end = index;
        while end < messages.len()
            && messages[end].get("role").and_then(Value::as_str) == Some("tool")
        {
            take_images_from_tool_message(&mut messages[end], &mut images);
            end += 1;
        }
        if !images.is_empty() {
            let mut content = vec![json!({
                "type": "text",
                "text": "Images returned by the tool call(s) above:"
            })];
            content.append(&mut images);
            messages.insert(end, json!({ "role": "user", "content": content }));
            end += 1;
        }
        index = end;
    }
}

/// 摘掉单条 tool 消息里的图片块，content 降级为纯文本。
/// 没有文本时补占位符 —— 空字符串 content 会被部分上游拒绝。
fn take_images_from_tool_message(message: &mut Value, images: &mut Vec<Value>) {
    let Some(parts) = message.get("content").and_then(Value::as_array) else {
        return;
    };

    let mut texts = Vec::new();
    let mut found = Vec::new();
    for part in parts {
        if is_image_part(part) {
            if let Some(image) = image_part_to_chat(part) {
                found.push(image);
            }
            continue;
        }
        if let Some(text) = part.get("text").and_then(Value::as_str)
            && !text.is_empty()
        {
            texts.push(text.to_string());
        }
    }
    if found.is_empty() {
        return;
    }

    let placeholder = if found.len() == 1 {
        "[image]".to_string()
    } else {
        format!("[{} images]", found.len())
    };
    message["content"] = json!(if texts.is_empty() {
        placeholder
    } else {
        format!("{}\n{placeholder}", texts.join("\n"))
    });
    images.append(&mut found);
}

fn append_text_to_assistant_message(message: &mut Value, text: &str) {
    if text.is_empty() {
        return;
    }
    let existing = match message.get("content") {
        Some(Value::String(content)) => content.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    message["content"] = if existing.trim().is_empty() {
        json!(text)
    } else {
        json!(format!("{existing}\n{text}"))
    };
}

/// DeepSeek thinking 模式要求带 `tool_calls` 的 assistant 消息回传 `reasoning_content`，
/// 否则报 400（The `reasoning_content` in the thinking mode must be passed back to the API）。
/// 历史里没有 reasoning 项时（例如上游没回传 summary，或被裁剪掉了）补一个占位说明，
/// 只补 content 和 reasoning_content 同时为空的情况，不覆盖真实 reasoning。
fn ensure_tool_call_reasoning_content(messages: &mut [Value]) {
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let has_tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|tool_calls| !tool_calls.is_empty());
        if !has_tool_calls {
            continue;
        }
        let has_content = message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| !content.trim().is_empty());
        let has_reasoning = message
            .get("reasoning_content")
            .and_then(Value::as_str)
            .is_some_and(|reasoning| !reasoning.trim().is_empty());
        if has_content || has_reasoning {
            continue;
        }
        message["reasoning_content"] = json!("Calling the requested tool.");
    }
}

fn normalize_chat_messages(messages: &mut [Value]) {
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let has_content = match message.get("content") {
            Some(Value::Null) | None => false,
            Some(Value::String(_)) => true,
            Some(Value::Array(parts)) => !parts.is_empty(),
            Some(_) => true,
        };
        let has_tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|tool_calls| !tool_calls.is_empty());
        if !has_content && !has_tool_calls {
            message["content"] = json!("");
        }
    }
}

/// 把所有 system 消息折叠成一条、前置到数组头部。
///
/// 之前只处理 content 是字符串的 system 消息：数组形态（content parts）与 null
/// 的会原样留在**原位**，于是出现「system 不在最前」——MiniMax 这类上游会直接
/// 拒绝（issue #1394 / #410）；空内容的 system 也照样发出去。
/// 现在无论 content 是什么形态都统一收拢，空的直接丢弃。
/// 取出 system 消息的纯文本：字符串直接返回，content parts 数组把 text 段拼起来，
/// 其余形态（null / 对象）按空处理。
fn system_message_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

fn collapse_system_messages_to_head(messages: Vec<Value>) -> Vec<Value> {
    let mut system_chunks = Vec::new();
    let mut rest = Vec::with_capacity(messages.len());

    for message in messages {
        if message.get("role").and_then(Value::as_str) == Some("system") {
            let text = system_message_text(message.get("content"));
            if !text.trim().is_empty() {
                system_chunks.push(text);
            }
            continue;
        }
        rest.push(message);
    }

    let mut output = Vec::with_capacity(rest.len() + usize::from(!system_chunks.is_empty()));
    if !system_chunks.is_empty() {
        output.push(json!({
            "role": "system",
            "content": system_chunks.join("\n\n")
        }));
    }
    output.extend(rest);
    output
}

fn responses_role_to_chat_role(role: Option<&str>) -> &'static str {
    match role {
        Some("developer") | Some("system") => "system",
        Some("assistant") => "assistant",
        Some("tool") => "tool",
        Some("latest_reminder") => "user",
        Some("user") | None => "user",
        Some(_) => "user",
    }
}

fn flush_tool_calls(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    pending_reasoning: &mut Vec<String>,
) {
    if pending_tool_calls.is_empty() {
        return;
    }

    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some("assistant") {
            merge_tool_calls_into_message(last, std::mem::take(pending_tool_calls));
            // 合并路径同样要消费 pending_reasoning（issue #2210）。
            // 触发时序：reasoning item → 不带 tool_calls 的 assistant 文本消息
            // （pending_tool_calls 为空，reasoning 被附加到该文本消息并 take）→
            // function_call。此时最后一条已是 assistant，走本分支提前 return，
            // 若这里不追加，随后的 reasoning 就随函数返回被静默丢弃；
            // 而 ensure_tool_call_reasoning_content 只补 content 与
            // reasoning_content 同时为空的占位，content 非空时补不上。
            if !pending_reasoning.is_empty() {
                append_reasoning_to_assistant_message(
                    last,
                    &std::mem::take(pending_reasoning).join("\n"),
                );
            }
            return;
        }
    }

    let mut message = json!({
        "role": "assistant",
        "content": "",
        "tool_calls": std::mem::take(pending_tool_calls)
    });
    if !pending_reasoning.is_empty() {
        message["reasoning_content"] = json!(std::mem::take(pending_reasoning).join("\n"));
    }
    messages.push(message);
}

fn flush_reasoning(messages: &mut Vec<Value>, pending_reasoning: &mut Vec<String>) {
    if pending_reasoning.is_empty() {
        return;
    }
    let reasoning = std::mem::take(pending_reasoning).join("\n");
    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some("assistant") {
            append_reasoning_to_assistant_message(last, &reasoning);
            return;
        }
    }
    messages.push(json!({
        "role": "assistant",
        "content": "",
        "reasoning_content": reasoning
    }));
}

fn append_reasoning_to_assistant_message(message: &mut Value, reasoning: &str) {
    if reasoning.is_empty() {
        return;
    }
    let existing = message
        .get("reasoning_content")
        .and_then(Value::as_str)
        .unwrap_or("");
    message["reasoning_content"] = if existing.is_empty() {
        json!(reasoning)
    } else {
        json!(format!("{existing}\n{reasoning}"))
    };
    if message.get("content").is_none() || message.get("content") == Some(&Value::Null) {
        message["content"] = json!("");
    }
}

fn merge_tool_calls_into_message(message: &mut Value, incoming: Vec<Value>) {
    let Some(object) = message.as_object_mut() else {
        return;
    };
    let existing = object
        .entry("tool_calls".to_string())
        .or_insert_with(|| json!([]));
    let Some(existing_array) = existing.as_array_mut() else {
        *existing = json!(incoming);
        return;
    };
    for tool_call in incoming {
        let id = tool_call.get("id").and_then(Value::as_str).unwrap_or("");
        if !id.is_empty()
            && existing_array
                .iter()
                .any(|item| item.get("id").and_then(Value::as_str) == Some(id))
        {
            continue;
        }
        existing_array.push(tool_call);
    }
    if message.get("content").is_none() || message.get("content") == Some(&Value::Null) {
        message["content"] = json!("");
    }
}

fn responses_reasoning_text(item: &Value) -> Option<String> {
    extract_reasoning_summary_text(item).or_else(|| extract_reasoning_field_text(item))
}

fn is_image_part(part: &Value) -> bool {
    matches!(
        part.get("type").and_then(Value::as_str),
        Some("input_image") | Some("image_url")
    )
}

/// 把 Responses 的 `input_image`（`image_url` 可能是裸字符串）与已经是 Chat 形态的
/// `image_url` 统一成 Chat Completions 的 `{"type":"image_url","image_url":{"url":…}}`。
///
/// 返回 `None` 表示这不是图片块、或 url 为空不值得转发。
fn image_part_to_chat(part: &Value) -> Option<Value> {
    if !is_image_part(part) {
        return None;
    }
    let raw = part.get("image_url")?;
    let image_url = if raw.is_object() {
        raw.clone()
    } else {
        json!({ "url": raw.as_str().unwrap_or_default() })
    };
    if image_url
        .get("url")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return None;
    }
    Some(json!({ "type": "image_url", "image_url": image_url }))
}

/// 已确认的 multi_agent v2 agent_message 明文形态：
/// content[] 同时包含以 `Payload:` 结尾的 input_text 头部和 encrypted_content 载荷片段。
/// 只有这一种结构允许把 encrypted_content 片段按明文文本透传；其余一律 fail closed。
fn agent_message_has_confirmed_plaintext_shape(content: &Value) -> bool {
    let Some(parts) = content.as_array() else {
        return false;
    };
    let has_payload_header = parts.iter().any(|part| {
        part.get("type").and_then(Value::as_str) == Some("input_text")
            && part
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.trim_end().ends_with("Payload:"))
    });
    let has_payload_fragment = parts
        .iter()
        .any(|part| part.get("type").and_then(Value::as_str) == Some("encrypted_content"));
    has_payload_header && has_payload_fragment
}

fn responses_content_to_chat_content(
    _role: &str,
    content: &Value,
    allow_plaintext_encrypted_content: bool,
) -> anyhow::Result<Value> {
    if content.is_null() || content.is_string() {
        return Ok(content.clone());
    }

    let Some(parts) = content.as_array() else {
        // 本 PR 删除了原来的全局 encrypted preflight，对象形态的 content 必须在这里
        // 自己守住防线，否则 {"type":"encrypted_content",...} 会原样发给 Chat 上游。
        // 已确认的 agent_message 明文形态要求 content 是数组，因此对象形态没有合法
        // 透传路径，一律 fail closed。
        if content.get("type").and_then(Value::as_str) == Some("encrypted_content") {
            return Err(UnsupportedEncryptedAgentContent.into());
        }
        return Ok(content.clone());
    };
    let mut chat_parts = Vec::new();
    let mut has_non_text_part = false;

    for part in parts {
        match part.get("type").and_then(Value::as_str).unwrap_or("") {
            "input_text" | "output_text" | "text" => {
                if let Some(value) = part.get("text").and_then(Value::as_str) {
                    if !value.is_empty() {
                        chat_parts.push(json!({ "type": "text", "text": value }));
                    }
                }
            }
            "refusal" => {
                if let Some(value) = part.get("refusal").and_then(Value::as_str) {
                    if !value.is_empty() {
                        chat_parts.push(json!({ "type": "text", "text": value }));
                    }
                }
            }
            "input_image" | "image_url" => {
                if let Some(image) = image_part_to_chat(part) {
                    chat_parts.push(image);
                    has_non_text_part = true;
                }
            }
            "encrypted_content" => {
                // 只有已确认的 agent_message 明文形态才允许透传。普通 message、
                // 缺少 Payload: 头部的 agent_message 及其它来源一律 fail closed，
                // 不能依赖内容启发式来决定是否绕过旧的拒绝防线。
                // 这里若吞掉，接收方模型只会看到空消息；值判定为 opaque 时也返回
                // 明确的不支持错误（fail loud），绝不静默删除。
                if !allow_plaintext_encrypted_content {
                    return Err(UnsupportedEncryptedAgentContent.into());
                }
                let value = part
                    .get("encrypted_content")
                    .or_else(|| part.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if value.is_empty() || encrypted_content_value_is_opaque(value) {
                    return Err(UnsupportedEncryptedAgentContent.into());
                }
                chat_parts.push(json!({ "type": "text", "text": value }));
            }
            _ => {}
        }
    }

    if !has_non_text_part {
        return Ok(Value::String(
            chat_parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }

    Ok(Value::Array(chat_parts))
}

/// 在已确认的 agent_message 明文形态内，判定 encrypted_content 片段值是否为
/// 无法在 Chat 协议中表达的 opaque 密文（二级防线，不承担来源范围判定）。
///
/// 实证依据：Codex Desktop（cli_version 0.162.0-alpha.2 / multi_agent_version v2）
/// 的 120 个真实 rollout 会话里，agent_message 的 encrypted_content 片段共 216 处，
/// 216/216 都是可打印明文（含中文、常规空格换行），0 处是纯 base64 高熵形态：
/// 当前客户端在这条路径上的构造事实就是明文字符串，名义协议里的「加密」未在
/// 该字段启用。但守卫不依赖这一点，只把最典型的密文形态判为 opaque，
/// 两条路径都不静默丢内容——
/// 1. 含 \t \n \r 之外的控制字符：明文文本不会携带，判为二进制；
/// 2. 无空白的纯 base64 字母表（去填充后 ≥32 字符）且可解码，解出字节的
///    可打印 ASCII 占比明显偏低（<60%）：分块加密输出的编码形态。
///    明文句子 base64 后通常带空格，不命中前半段；单 token 明文解码回来仍是
///    ASCII 文本，可打印占比高，不命中后半段。
///
/// 两个条件都不满足即按明文透传。
fn encrypted_content_value_is_opaque(value: &str) -> bool {
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
    {
        return true;
    }
    let body = value.trim_end_matches('=');
    if body.len() < 32
        || !body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'-' | b'_' | b'='))
    {
        return false;
    }
    // base64url 与标准字母表在字符集层面无法区分，解码前逐字符映射到标准表。
    // 这里不能写成 replace(['-', '_'], "+/")：str::replace 会把每个匹配字符替换成
    // 完整的 "+/" 两个字符，长度改变后解码必然失败，opaque 载荷会被误判为明文。
    let canonical: String = body
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            other => other,
        })
        .collect();
    let padded = format!("{canonical}{}", "=".repeat((4 - canonical.len() % 4) % 4));
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&padded) else {
        // 长度不满足编码约束（解码失败）就不是合法的密文编码，按明文处理；
        // 真实密文的编码必然满足长度约束。
        // （len % 4 == 1 的输入也在这里被排除。）
        return false;
    };
    if decoded.is_empty() {
        return false;
    }
    // 明文（ASCII 文本再 base64）解码回来仍是高可打印文本；密文解码字节接近
    // 均匀分布，可打印 ASCII 占比仅约 37%。阈值取 60%。
    let printable = decoded
        .iter()
        .filter(|b| matches!(**b, 0x20..0x7f | b'\t' | b'\n' | b'\r'))
        .count();
    printable * 5 < decoded.len() * 3
}

fn responses_history_function_name(item: &Value) -> String {
    let name = item.get("name").and_then(Value::as_str).unwrap_or("");
    let namespace = item.get("namespace").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() {
        String::new()
    } else if namespace.is_empty() {
        name.to_string()
    } else {
        flatten_namespace_tool_name(namespace, name)
    }
}

fn build_codex_tool_context(tools: Option<&Value>) -> CodexToolContext {
    let mut context = CodexToolContext::default();
    let Some(tools) = tools.and_then(Value::as_array) else {
        return context;
    };

    for tool in tools {
        if let Some(name) = tool.as_str().filter(|name| !name.is_empty()) {
            context.custom_tools.insert(
                name.to_string(),
                CodexCustomToolSpec {
                    openai_name: name.to_string(),
                    kind: CodexCustomToolKind::Raw,
                },
            );
            context.has_custom_tools = true;
            continue;
        }
        let tool_type = tool.get("type").and_then(Value::as_str).unwrap_or("");
        match tool_type {
            "custom" => {
                let Some(name) = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                else {
                    continue;
                };
                let kind = detect_codex_custom_tool_kind(tool, name);
                context.custom_tools.insert(
                    name.to_string(),
                    CodexCustomToolSpec {
                        openai_name: name.to_string(),
                        kind,
                    },
                );
                context.has_custom_tools = true;
            }
            "function" => {
                if let Some(name) = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    context.function_tools.insert(
                        name.to_string(),
                        CodexFunctionToolSpec {
                            name: name.to_string(),
                            namespace: String::new(),
                        },
                    );
                }
            }
            "namespace" => add_namespace_tools_to_context(&mut context, tool),
            // Codex 的 tool_search（MCP 延迟加载检索）是客户端执行的代理工具：
            // 转发层把它当作 custom 代理工具登记，模型调用回来时还原成
            // tool_search_call item，检索本身仍由 Codex 客户端执行。
            "tool_search" => {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .unwrap_or("tool_search");
                context.custom_tools.insert(
                    name.to_string(),
                    CodexCustomToolSpec {
                        openai_name: name.to_string(),
                        kind: CodexCustomToolKind::ToolSearch,
                    },
                );
                context.has_custom_tools = true;
            }
            "web_search" | "local_shell" | "computer_use" => {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .unwrap_or(tool_type);
                context.custom_tools.insert(
                    name.to_string(),
                    CodexCustomToolSpec {
                        openai_name: name.to_string(),
                        kind: CodexCustomToolKind::BuiltIn,
                    },
                );
                context.has_custom_tools = true;
            }
            _ => {}
        }
    }

    context
}

fn add_namespace_tools_to_context(context: &mut CodexToolContext, namespace_tool: &Value) {
    let namespace = namespace_tool
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let Some(children) = namespace_tool.get("tools").and_then(Value::as_array) else {
        return;
    };
    for child in children {
        if child.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        let Some(name) = child
            .get("name")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        let flat = flatten_namespace_tool_name(namespace, name);
        if namespace.is_empty() {
            context.function_tools.insert(
                flat,
                CodexFunctionToolSpec {
                    namespace: namespace.to_string(),
                    name: name.to_string(),
                },
            );
        } else if context
            .function_tools
            .get(&flat)
            .is_none_or(|spec| !spec.namespace.is_empty())
        {
            context.function_tools.insert(
                flat,
                CodexFunctionToolSpec {
                    namespace: namespace.to_string(),
                    name: name.to_string(),
                },
            );
            context.has_namespace_tools = true;
        }
    }
}

fn responses_tools_to_chat_tools(tools: &[Value], context: &CodexToolContext) -> Vec<Value> {
    let mut converted = Vec::new();
    for tool in tools {
        if let Some(name) = tool.as_str().filter(|name| !name.is_empty()) {
            converted.push(generic_custom_proxy_tool(name, ""));
            continue;
        }
        match tool.get("type").and_then(Value::as_str).unwrap_or("") {
            "function" => {
                if let Some(tool) = responses_function_tool_to_chat_tool(tool) {
                    converted.push(tool);
                }
            }
            "custom" | "web_search" | "local_shell" | "computer_use" => {
                let tool_type = tool.get("type").and_then(Value::as_str).unwrap_or("");
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .unwrap_or(tool_type);
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if detect_codex_custom_tool_kind(tool, name) == CodexCustomToolKind::ApplyPatch {
                    converted.push(custom_proxy_tool_with_original_definition(
                        tool,
                        name,
                        description,
                    ));
                } else {
                    converted.push(generic_custom_proxy_tool(name, description));
                }
            }
            "namespace" => converted.extend(namespace_tool_to_chat_tools(tool, context)),
            // tool_search 透传为 chat 的 function 工具，名字保持 tool_search，
            // 检索由 Codex 客户端执行（execution: client），转发层只做搬运。
            "tool_search" => {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .unwrap_or("tool_search");
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let parameters = tool.get("parameters").cloned().unwrap_or_else(|| json!({}));
                converted.push(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": parameters
                    }
                }));
            }
            _ => {}
        }
    }
    converted
}

fn detect_codex_custom_tool_kind(tool: &Value, name: &str) -> CodexCustomToolKind {
    if name == "apply_patch" {
        return CodexCustomToolKind::ApplyPatch;
    }
    if let Some(definition) = tool.pointer("/format/definition").and_then(Value::as_str) {
        if definition.contains("begin_patch")
            && definition.contains("end_patch")
            && definition.contains("add_hunk")
        {
            return CodexCustomToolKind::ApplyPatch;
        }
    }
    if matches!(
        tool.get("type").and_then(Value::as_str),
        Some("web_search" | "local_shell" | "computer_use")
    ) {
        CodexCustomToolKind::BuiltIn
    } else {
        CodexCustomToolKind::Raw
    }
}

fn responses_function_tool_to_chat_tool(tool: &Value) -> Option<Value> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return None;
    }
    if tool.get("function").is_some() {
        let mut chat_tool = tool.clone();
        if let Some(strict) = tool.get("strict").cloned() {
            if let Some(function) = chat_tool.get_mut("function").and_then(Value::as_object_mut) {
                function.entry("strict".to_string()).or_insert(strict);
            }
            if let Some(object) = chat_tool.as_object_mut() {
                object.remove("strict");
            }
        }
        if let Some(function) = chat_tool.get_mut("function").and_then(Value::as_object_mut) {
            let normalized =
                normalize_chat_tool_parameters(function.get("parameters").unwrap_or(&json!({})));
            function.insert("parameters".to_string(), normalized);
        }
        return Some(chat_tool);
    }
    let mut function = json!({
        "name": tool.get("name").and_then(Value::as_str).unwrap_or(""),
        "description": tool.get("description").cloned().unwrap_or(Value::Null),
        "parameters": normalize_chat_tool_parameters(tool.get("parameters").unwrap_or(&json!({})))
    });
    if let Some(strict) = tool.get("strict") {
        function["strict"] = strict.clone();
    }
    Some(json!({
        "type": "function",
        "function": function
    }))
}

fn namespace_tool_to_chat_tools(namespace_tool: &Value, context: &CodexToolContext) -> Vec<Value> {
    let namespace = namespace_tool
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let namespace_description = namespace_tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let Some(children) = namespace_tool.get("tools").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut converted = Vec::new();
    for child in children {
        if child.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        let Some(name) = child
            .get("name")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        let flat = flatten_namespace_tool_name(namespace, name);
        if namespace != ""
            && context
                .function_tools
                .get(&flat)
                .is_some_and(|spec| spec.namespace.is_empty())
        {
            continue;
        }
        let description = combine_namespace_description(
            namespace_description,
            child
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let mut function = json!({
            "name": flat,
            "parameters": normalize_chat_tool_parameters(child.get("parameters").unwrap_or(&json!({})))
        });
        if !description.is_empty() {
            function["description"] = json!(description);
        }
        converted.push(json!({
            "type": "function",
            "function": function
        }));
    }
    converted
}

/// Gemini 系上游（含中转暴露的 google/gemini-*）要求 function declaration 的
/// `enum` 全部是字符串，布尔/数字枚举会被整轮拒绝（#2404）。
fn upstream_requires_string_enums(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("gemini") || model.starts_with("google/") || model.starts_with("models/gemini")
}

/// 递归把 schema 中 `enum` 数组的非字符串值转成字符串。
fn coerce_enum_values_to_strings(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // Gemini 只允许 STRING 类型带 enum。非字符串字段改成 string 会让模型回传
            // "1" 这种字符串、工具端按 integer 校验失败，所以这类字段去掉 enum、保留原类型，
            // 可选值写进 description 提示模型。
            let non_string_type = match map.get("type") {
                Some(Value::String(kind)) => kind != "string",
                Some(Value::Array(kinds)) => !kinds.iter().any(|kind| kind == "string"),
                _ => false,
            };
            if non_string_type && let Some(Value::Array(entries)) = map.remove("enum") {
                let allowed = entries
                    .iter()
                    .map(|entry| match entry {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let hint = format!("Allowed values: {allowed}");
                let description = match map.get("description").and_then(Value::as_str) {
                    Some(existing) if !existing.trim().is_empty() => format!("{existing} ({hint})"),
                    _ => hint,
                };
                map.insert("description".to_string(), Value::String(description));
            }
            if let Some(Value::Array(entries)) = map.get_mut("enum") {
                for entry in entries.iter_mut() {
                    let coerced = match entry {
                        Value::String(_) => None,
                        Value::Bool(flag) => Some(flag.to_string()),
                        Value::Number(number) => Some(number.to_string()),
                        Value::Null => Some("null".to_string()),
                        _ => None,
                    };
                    if let Some(coerced) = coerced {
                        *entry = Value::String(coerced);
                    }
                }
            }
            for child in map.values_mut() {
                coerce_enum_values_to_strings(child);
            }
        }
        Value::Array(values) => {
            for child in values.iter_mut() {
                coerce_enum_values_to_strings(child);
            }
        }
        _ => {}
    }
}

fn normalize_chat_tool_parameters(parameters: &Value) -> Value {
    let mut normalized = if parameters.is_object() {
        parameters.clone()
    } else {
        json!({})
    };
    // 裸 `$ref` 已经是完整 schema，补默认字段会人为制造 sibling。
    let is_bare_ref = normalized
        .as_object()
        .is_some_and(|object| object.len() == 1 && object.contains_key("$ref"));
    if !is_bare_ref {
        // `type: null` 与缺失等价：严格供应商（如 deepseek）会以
        // `got 'type: null'` 拒绝整个请求（issue #2247）。
        if normalized.get("type").is_none_or(Value::is_null) {
            normalized["type"] = json!("object");
        }
        if normalized.get("properties").is_none() {
            normalized["properties"] = json!({});
        }
        if normalized.get("required").is_none() {
            normalized["required"] = json!([]);
        }
    }
    let normalized = inline_ref_siblings(&normalized);
    // 必须先内联再摊平：合并分支时 $defs 已摊开才拿得到真实属性（issue #2367）。
    flatten_top_level_combinators(normalized)
}

/// JSON Schema 的顶层组合器（`oneOf` / `anyOf` / `allOf`）。
const SCHEMA_COMBINATOR_KEYS: [&str; 3] = ["oneOf", "anyOf", "allOf"];

/// 摊平工具 schema **顶层**的组合器（issue #2367）。
///
/// zod-to-json-schema 会生成形如
/// `{type:"object", properties:{}, oneOf:[{$ref:"#/$defs/__schema0"},…], $defs:{…}}`
/// 的 schema。部分上游（Anthropic 系）拒绝顶层 `oneOf`，直接整轮 400、模型完全不可用。
/// 全仓原本对 `oneOf` 零处理。
///
/// 策略（保守优先，绝不让整轮失败）：
/// 1. 顶层有组合器时，逐分支归一化后再摊平；
/// 2. `properties` 取各分支并集，同名属性都是 object 时递归合并其 properties；
///    `required` 取交集（只有所有分支都要求才算必须）；
/// 3. 剥掉组合器键，补回 `type:"object"`；
/// 4. 无法摊平（分支不是对象、合并后 properties 为空）时把各分支塞进一个带
///    description 的私有字段，保住信息且 schema 仍合法；
/// 5. 结果仍不是合法对象 schema 时原样返回——宁可交给上游判断，也不静默丢掉工具。
fn flatten_top_level_combinators(schema: Value) -> Value {
    let Some(object) = schema.as_object() else {
        return schema;
    };
    let Some((key, branches)) = SCHEMA_COMBINATOR_KEYS.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_array)
            .map(|a| (*key, a))
    }) else {
        return schema;
    };
    if branches.is_empty() {
        return schema;
    }

    // 分支常是裸 `$ref`（`{ "$ref": "#/$defs/__schema0" }`），它自己不带 `$defs`，
    // 所以必须拿**父级**的 $defs 来解析——只对分支单独调 inline_ref_siblings
    // 永远解析不出来（实测：分支落进降级路径，properties 为空）。
    let defs = object.get("$defs").and_then(Value::as_object);

    let mut properties = Map::new();
    let mut required: Option<BTreeSet<String>> = None;
    let mut flattenable = true;

    for branch in branches {
        let normalized = match resolve_local_definition(
            branch
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(local_definition_name)
                .unwrap_or(""),
            defs,
            &mut Vec::new(),
        ) {
            // 分支是裸 $ref：用父级 $defs 解析出真实 schema。
            Ok(Some(resolved)) if branch.as_object().is_some_and(|o| o.len() == 1) => {
                normalize_schema_value(&resolved, defs, &mut Vec::new()).unwrap_or(resolved)
            }
            _ => inline_ref_siblings(branch),
        };
        let Some(branch_object) = normalized.as_object() else {
            flattenable = false;
            break;
        };
        let branch_properties = branch_object.get("properties").and_then(Value::as_object);
        let branch_required: BTreeSet<String> = branch_object
            .get("required")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        // 分支没有 properties 时（例如 type:"string"）无法并入对象 schema。
        if branch_properties.is_none() && !branch_required.is_empty() {
            flattenable = false;
            break;
        }
        for (name, value) in branch_properties.into_iter().flatten() {
            merge_schema_property(&mut properties, name, value);
        }
        required = Some(match required {
            None => branch_required,
            Some(current) => current.intersection(&branch_required).cloned().collect(),
        });
    }

    if !flattenable || properties.is_empty() {
        // 降级：把分支原样塞进一个带说明的字段，schema 依然合法。
        let mut fallback = object.clone();
        fallback.insert("type".to_string(), json!("object"));
        fallback.insert("properties".to_string(), json!({}));
        let mut description: Vec<String> = Vec::new();
        for branch in branches {
            if let Some(text) = branch.get("description").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    description.push(text.trim().to_string());
                }
            }
        }
        fallback.insert(
            "x-merged-combinator".to_string(),
            json!({
                "kind": key,
                "branches": branches,
                "description": description.join("\n")
            }),
        );
        for combinator in SCHEMA_COMBINATOR_KEYS {
            fallback.remove(combinator);
        }
        return Value::Object(fallback);
    }

    let mut flattened = object.clone();
    for combinator in SCHEMA_COMBINATOR_KEYS {
        flattened.remove(combinator);
    }
    flattened.insert("type".to_string(), json!("object"));
    flattened.insert("properties".to_string(), Value::Object(properties));
    flattened.insert(
        "required".to_string(),
        json!(required.unwrap_or_default().into_iter().collect::<Vec<_>>()),
    );
    Value::Object(flattened)
}

/// 把分支属性并入目标 properties；同名且两侧都是 object schema 时递归合并
/// （properties 并集、required 取交集），否则保留先到的一方（非破坏性）。
fn merge_schema_property(properties: &mut Map<String, Value>, name: &str, value: &Value) {
    let Some(existing) = properties.get_mut(name) else {
        properties.insert(name.to_string(), value.clone());
        return;
    };
    let (Some(left), Some(right)) = (existing.as_object(), value.as_object()) else {
        return;
    };
    if left.get("properties").is_none() || right.get("properties").is_none() {
        return;
    }
    let mut merged = left.clone();
    let mut merged_properties = left
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (child_name, child_value) in right
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        merge_schema_property(&mut merged_properties, child_name, child_value);
    }
    merged.insert("properties".to_string(), Value::Object(merged_properties));

    let left_required: BTreeSet<String> = left
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let right_required: BTreeSet<String> = right
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    merged.insert(
        "required".to_string(),
        json!(
            left_required
                .intersection(&right_required)
                .cloned()
                .collect::<Vec<_>>()
        ),
    );
    *existing = Value::Object(merged);
}

fn inline_ref_siblings(root: &Value) -> Value {
    let defs = root.get("$defs").and_then(Value::as_object);
    let mut resolving = Vec::new();
    normalize_schema_value(root, defs, &mut resolving).unwrap_or_else(|_| root.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalRefNormalizationError {
    Cycle,
}

fn normalize_schema_value(
    node: &Value,
    defs: Option<&Map<String, Value>>,
    resolving: &mut Vec<String>,
) -> Result<Value, LocalRefNormalizationError> {
    match node {
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|item| normalize_schema_value(item, defs, resolving))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Object(object) => normalize_schema_object(object, defs, resolving),
        _ => Ok(node.clone()),
    }
}

fn normalize_schema_object(
    object: &Map<String, Value>,
    defs: Option<&Map<String, Value>>,
    resolving: &mut Vec<String>,
) -> Result<Value, LocalRefNormalizationError> {
    if object.len() > 1
        && let Some(reference) = object.get("$ref").and_then(Value::as_str)
        && let Some(name) = local_definition_name(reference)
    {
        match resolve_local_definition(name, defs, resolving)? {
            Some(Value::Object(mut merged)) if merged.get("$ref").is_none() => {
                for (key, value) in object {
                    if key != "$ref" {
                        merged.insert(key.clone(), normalize_schema_value(value, defs, resolving)?);
                    }
                }
                return Ok(Value::Object(merged));
            }
            Some(_) | None => {}
        }
    }

    let mut normalized = Map::new();
    for (key, value) in object {
        // JSON Schema 的 type 必须是字符串，null 恒非法，剥掉等价于未声明。
        if key == "type" && value.is_null() {
            continue;
        }
        normalized.insert(key.clone(), normalize_schema_value(value, defs, resolving)?);
    }
    Ok(Value::Object(normalized))
}

fn resolve_local_definition(
    name: &str,
    defs: Option<&Map<String, Value>>,
    resolving: &mut Vec<String>,
) -> Result<Option<Value>, LocalRefNormalizationError> {
    let Some(defs) = defs else {
        return Ok(None);
    };
    let Some(target) = defs.get(name) else {
        return Ok(None);
    };
    if resolving.iter().any(|current| current == name) {
        return Err(LocalRefNormalizationError::Cycle);
    }

    resolving.push(name.to_string());
    let resolved = if let Some(alias) = bare_local_ref_name(target) {
        resolve_local_definition(alias, Some(defs), resolving)
    } else {
        normalize_schema_value(target, Some(defs), resolving).map(Some)
    };
    resolving.pop();
    resolved
}

fn bare_local_ref_name(node: &Value) -> Option<&str> {
    let object = node.as_object()?;
    if object.len() != 1 {
        return None;
    }
    local_definition_name(object.get("$ref")?.as_str()?)
}

fn local_definition_name(reference: &str) -> Option<&str> {
    let name = reference.strip_prefix("#/$defs/")?;
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some(name)
}

fn generic_custom_proxy_tool(name: &str, description: &str) -> Value {
    let description = if description.trim().is_empty() {
        format!("FREEFORM custom tool: {name}. Put only the tool input text here.")
    } else {
        format!(
            "{}\n\nThis is a FREEFORM tool. Do not wrap the input in JSON or markdown.",
            description.trim()
        )
    };
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Raw freeform input for this custom tool."
                    }
                },
                "required": ["input"]
            }
        }
    })
}

fn custom_proxy_tool_with_original_definition(
    tool: &Value,
    name: &str,
    description: &str,
) -> Value {
    let mut description = if description.trim().is_empty() {
        format!("FREEFORM custom tool: {name}. Put only the tool input text here.")
    } else {
        format!(
            "{}\n\nThis is a FREEFORM tool. Do not wrap the input in JSON or markdown.",
            description.trim()
        )
    };
    description.push_str("\n\nOriginal tool definition:\n```json\n");
    description.push_str(&canonical_json_string(tool));
    description.push_str("\n```");

    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Raw string input for the original custom tool. Preserve formatting exactly and follow the original tool definition embedded in the description."
                    }
                },
                "required": ["input"]
            }
        }
    })
}

/// 从历史里的 tool_search_output item 收集 Codex 客户端检索命中的命名空间工具。
///
/// 条目形如 `{ type: "namespace", name, description, tools: [...] }`，必须走
/// `add_namespace_tools_to_context` + `namespace_tool_to_chat_tools` 展开成
/// `mcp__<server>__<tool>`。只取 namespace 名字的话上游只能看到外壳，
/// 调不到内层工具，反向转换也还原不出 namespace 字段。
fn merge_namespace_definition(collected: &mut Vec<Value>, tool: &Value) {
    let Some(name) = tool
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    else {
        return;
    };
    let kind = tool.get("type").and_then(Value::as_str);
    let existing = collected.iter_mut().find(|entry| {
        entry.get("name").and_then(Value::as_str) == Some(name)
            && entry.get("type").and_then(Value::as_str) == kind
    });
    let Some(existing) = existing else {
        collected.push(tool.clone());
        return;
    };
    if kind != Some("namespace") {
        *existing = tool.clone();
        return;
    }
    let mut children = existing
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for child in tool
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let child_name = child.get("name").and_then(Value::as_str);
        let child_type = child.get("type").and_then(Value::as_str);
        if let Some(old) = children.iter_mut().find(|old| {
            child_name.is_some()
                && old.get("name").and_then(Value::as_str) == child_name
                && old.get("type").and_then(Value::as_str) == child_type
        }) {
            // 后来的定义只决定下一轮调用的 schema，不改写旧 call/output。
            if old.get("parameters") != child.get("parameters") {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "protocol_proxy.namespace_schema_updated",
                    json!({"namespace":name, "tool":child_name, "policy":"current_then_latest_history"}),
                );
            }
            *old = child.clone();
        } else {
            children.push(child.clone());
        }
    }
    *existing = tool.clone();
    existing["tools"] = json!(children);
}

fn collect_tool_search_output_namespaces(body: &Value) -> Vec<Value> {
    let mut collected: Vec<Value> = Vec::new();
    let Some(items) = body.get("input").and_then(Value::as_array) else {
        return collected;
    };

    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("tool_search_output") {
            continue;
        }
        let Some(tools) = item.get("tools").and_then(Value::as_array) else {
            continue;
        };
        for tool in tools {
            merge_namespace_definition(&mut collected, tool);
        }
    }

    collected
}

/// 出站与两种反向转换共享同一投影，已过滤的原生工具不能抢占同名客户端工具。
struct ChatToolProjection {
    tools: Vec<Value>,
    context: CodexToolContext,
}

fn chat_tool_projection(body: &Value) -> ChatToolProjection {
    let mut namespaces = collect_tool_search_output_namespaces(body);
    let mut projected = Vec::new();
    for tool in body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match tool.get("type").and_then(Value::as_str) {
            Some("web_search" | "web_search_preview") => {}
            Some("namespace") => merge_namespace_definition(&mut namespaces, tool),
            _ => projected.push(tool.clone()),
        }
    }
    // 顶层当前定义最后合并，优先于所有历史定义；同名 namespace 新 children 不丢。
    projected.extend(namespaces);
    let value = json!(projected);
    let mut context = build_codex_tool_context(Some(&value));
    for tool in body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if matches!(
            tool.get("type").and_then(Value::as_str),
            Some("web_search" | "web_search_preview")
        ) {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("web_search");
            if !context.function_tools.contains_key(name)
                && !context.custom_tools.contains_key(name)
            {
                context.rejected_native_search.insert(name.to_string());
            }
        }
    }
    let mut tools = responses_tools_to_chat_tools(value.as_array().expect("tools array"), &context);
    dedup_chat_tools_by_name(&mut tools);
    ChatToolProjection { tools, context }
}
/// chat tools 按函数名去重，保留首次出现。
fn dedup_chat_tools_by_name(tools: &mut Vec<Value>) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    tools.retain(|tool| {
        match tool
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        {
            Some(name) => seen.insert(name.to_string()),
            None => true,
        }
    });
}

fn combine_namespace_description(namespace_description: &str, child_description: &str) -> String {
    let namespace_description = namespace_description.trim();
    let child_description = child_description.trim();
    match (
        namespace_description.is_empty(),
        child_description.is_empty(),
    ) {
        (true, true) => String::new(),
        (true, false) => child_description.to_string(),
        (false, true) => namespace_description.to_string(),
        (false, false) => format!("{namespace_description}\n\n{child_description}"),
    }
}

fn flatten_namespace_tool_name(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        return name.to_string();
    }
    if name.is_empty() {
        return namespace.to_string();
    }
    if namespace.ends_with("__") || name.starts_with("__") {
        format!("{namespace}{name}")
    } else {
        format!("{namespace}__{name}")
    }
}

fn responses_tool_choice_to_chat(tool_choice: &Value, context: &CodexToolContext) -> Option<Value> {
    match tool_choice {
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("function") => {
            if let Some(namespace) = object.get("namespace").and_then(Value::as_str) {
                let name = object.get("name").and_then(Value::as_str).unwrap_or("");
                return Some(json!({
                    "type": "function",
                    "function": {
                        "name": flatten_namespace_tool_name(namespace, name)
                    }
                }));
            }
            if let Some(function) = object.get("function").and_then(Value::as_object) {
                if let Some(namespace) = function.get("namespace").and_then(Value::as_str) {
                    let name = function.get("name").and_then(Value::as_str).unwrap_or("");
                    return Some(json!({
                        "type": "function",
                        "function": {
                            "name": flatten_namespace_tool_name(namespace, name)
                        }
                    }));
                }
            }
            Some(json!({
                "type": "function",
                "function": {
                    "name": object.get("name").and_then(Value::as_str).unwrap_or("")
                }
            }))
        }
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("custom") => {
            let name = object.get("name").and_then(Value::as_str)?;
            let spec = context.custom_tools.get(name)?;
            let upstream_name = spec.openai_name.clone();
            Some(json!({
                "type": "function",
                "function": { "name": upstream_name }
            }))
        }
        other => Some(other.clone()),
    }
}

fn chat_reasoning_to_response_output_item(message: &Value, response_id: &str) -> Option<Value> {
    let reasoning = chat_reasoning_text(message)?;
    if reasoning.is_empty() {
        return None;
    }
    Some(json!({
        "id": format!("rs_{response_id}"),
        "type": "reasoning",
        "reasoning_content": reasoning,
        "summary": [{ "type": "summary_text", "text": reasoning }]
    }))
}

fn chat_reasoning_text(message: &Value) -> Option<String> {
    if let Some(reasoning) = extract_reasoning_field_text(message) {
        return Some(reasoning);
    }

    if let Some(content) = message.get("content").and_then(Value::as_str) {
        if let Some((reasoning, _answer)) = split_leading_think_block(content) {
            if !reasoning.is_empty() {
                return Some(reasoning);
            }
        }
    }

    None
}

fn chat_message_to_response_output_item(message: &Value, response_id: &str) -> Option<Value> {
    let mut content = Vec::new();
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        let text = split_leading_think_block(text)
            .map(|(_reasoning, answer)| answer)
            .unwrap_or_else(|| text.to_string());
        if !text.is_empty() {
            content.push(json!({ "type": "output_text", "text": text, "annotations": [] }));
        }
    } else if let Some(parts) = message.get("content").and_then(Value::as_array) {
        for part in parts {
            match part.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" | "output_text" => {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            content.push(
                                json!({ "type": "output_text", "text": text, "annotations": [] }),
                            );
                        }
                    }
                }
                "refusal" => {
                    if let Some(refusal) = part.get("refusal").and_then(Value::as_str) {
                        if !refusal.is_empty() {
                            content.push(json!({ "type": "refusal", "refusal": refusal }));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(refusal) = message.get("refusal").and_then(Value::as_str) {
        if !refusal.is_empty() {
            content.push(json!({ "type": "refusal", "refusal": refusal }));
        }
    }

    if content.is_empty() {
        return None;
    }

    Some(json!({
        "id": format!("msg_{}", response_id_body(response_id)),
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": content
    }))
}

fn chat_tool_calls_to_response_output_items(
    message: &Value,
    tool_context: &CodexToolContext,
) -> Vec<Value> {
    let mut output = Vec::new();
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, tool_call) in tool_calls.iter().enumerate() {
            output.push(chat_tool_call_to_response_item(
                tool_call,
                index,
                tool_context,
            ));
        }
    } else if let Some(function_call) = message.get("function_call") {
        output.push(chat_legacy_function_call_to_response_item(
            function_call,
            tool_context,
        ));
    }
    output
}

fn chat_tool_call_to_response_item(
    tool_call: &Value,
    index: usize,
    tool_context: &CodexToolContext,
) -> Value {
    let call_id = tool_call
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("call_{index}"));
    let function = tool_call.get("function").unwrap_or(&Value::Null);
    let name = function.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = responses_arguments_to_chat(function.get("arguments").unwrap_or(&json!({})));
    // 记住供应商在本轮工具调用上附带的透传数据（Gemini thought_signature 等），
    // 下一轮构造请求时挂回，否则整轮 400（issue #332 / #1012）。
    remember_tool_call_extra_content(&call_id, tool_call);
    response_tool_call_item(&call_id, name, &arguments, tool_context)
}

fn chat_legacy_function_call_to_response_item(
    function_call: &Value,
    tool_context: &CodexToolContext,
) -> Value {
    let call_id = function_call
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("call_0");
    let name = function_call
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let arguments =
        responses_arguments_to_chat(function_call.get("arguments").unwrap_or(&json!({})));
    response_tool_call_item(call_id, name, &arguments, tool_context)
}

fn tool_call_added_item(
    state: &ToolCallState,
    output_index: u32,
    tool_context: &CodexToolContext,
) -> Value {
    if tool_context.is_builtin_web_search_proxy(&state.name) {
        // 客户端只认 web_search_call；用 custom_tool_call 会被当作未知工具丢弃，
        // 上游那轮的搜索结果就再也回不到会话里（issue #1209 / #586）。
        return json!({
            "type": "response.output_item.added",
            "output_index": output_index,
            "item": {
                "id": web_search_item_id(&state.call_id),
                "type": "web_search_call",
                "status": "in_progress",
                "call_id": state.call_id,
                "action": web_search_action_from_arguments(&state.arguments)
            }
        });
    }
    if tool_context.is_custom_tool_proxy(&state.name) {
        if tool_context.is_tool_search_proxy(&state.name) {
            return json!({
                "type": "response.output_item.added",
                "output_index": output_index,
                "item": {
                    "id": tool_call_item_id(&state.call_id, &state.name, tool_context),
                    "type": "tool_search_call",
                    "status": "in_progress",
                    "call_id": state.call_id,
                    "execution": "client",
                    "arguments": {}
                }
            });
        }
        return json!({
            "type": "response.output_item.added",
            "output_index": output_index,
            "item": {
                "id": tool_call_item_id(&state.call_id, &state.name, tool_context),
                "type": "custom_tool_call",
                "status": "in_progress",
                "call_id": state.call_id,
                "name": tool_context.original_custom_tool_name(&state.name),
                "input": ""
            }
        });
    }
    let (display_name, namespace) = tool_context.openai_name_for_function_tool(&state.name);
    let mut item = json!({
        "type": "response.output_item.added",
        "output_index": output_index,
        "item": {
            "id": state.item_id,
            "type": "function_call",
            "status": "in_progress",
            "call_id": state.call_id,
            "name": display_name,
            "arguments": ""
        }
    });
    if !namespace.is_empty() {
        item["item"]["namespace"] = json!(namespace);
    }
    item
}

fn push_tool_call_delta_sse(
    output: &mut String,
    state: &ToolCallState,
    output_index: u32,
    delta: &str,
    tool_context: &CodexToolContext,
) {
    if tool_context.is_tool_search_proxy(&state.name)
        || tool_context.is_builtin_web_search_proxy(&state.name)
    {
        // tool_search 与原生 web_search 都走 function 参数流，客户端按
        // tool_search_call.arguments / web_search_call 聚合。
        push_sse(
            output,
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": state.item_id,
                "output_index": output_index,
                "delta": delta
            }),
        );
    } else if tool_context.is_custom_tool_proxy(&state.name) {
        let _ = delta;
    } else {
        push_sse(
            output,
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": state.item_id,
                "output_index": output_index,
                "delta": delta
            }),
        );
    }
}

fn push_tool_call_done_sse(
    output: &mut String,
    state: &ToolCallState,
    output_index: u32,
    tool_context: &CodexToolContext,
) {
    if tool_context.is_tool_search_proxy(&state.name) {
        push_sse(
            output,
            "response.function_call_arguments.done",
            json!({
                "type": "response.function_call_arguments.done",
                "item_id": state.item_id,
                "output_index": output_index,
                "arguments": state.arguments
            }),
        );
        return;
    }
    if tool_context.is_custom_tool_proxy(&state.name) {
        push_sse(
            output,
            "response.custom_tool_call_input.delta",
            json!({
                "type": "response.custom_tool_call_input.delta",
                "item_id": tool_call_item_id(&state.call_id, &state.name, tool_context),
                "call_id": state.call_id,
                "output_index": output_index,
                "delta": reconstruct_custom_tool_call_input_with_context(
                    tool_context,
                    &state.name,
                    &state.arguments
                )
            }),
        );
        return;
    }
    push_sse(
        output,
        "response.function_call_arguments.done",
        json!({
            "type": "response.function_call_arguments.done",
            "item_id": state.item_id,
            "output_index": output_index,
            "arguments": state.arguments
        }),
    );
}

fn tool_call_done_item(state: &ToolCallState, tool_context: &CodexToolContext) -> Value {
    response_tool_call_item(&state.call_id, &state.name, &state.arguments, tool_context)
}

fn response_tool_call_item(
    call_id: &str,
    name: &str,
    arguments: &str,
    tool_context: &CodexToolContext,
) -> Value {
    if tool_context.is_builtin_web_search_proxy(name) {
        // 客户端只认 web_search_call，custom_tool_call 会被当作未知工具丢弃。
        return json!({
            "id": web_search_item_id(call_id),
            "type": "web_search_call",
            "status": "failed",
            "call_id": call_id,
            "action": web_search_action_from_arguments(arguments)
        });
    }
    if tool_context.is_custom_tool_proxy(name) {
        if tool_context.is_tool_search_proxy(name) {
            // 官方客户端的 tool_search handler 只接受 tool_search_call item，
            // function_call 形态会被拒绝，因此必须还原成专属 item 类型；
            // arguments 转发层原样搬运（对象字符串双向保真）。
            return json!({
                "id": format!("tsc_{call_id}"),
                "type": "tool_search_call",
                "status": "completed",
                "call_id": call_id,
                "execution": "client",
                "arguments": responses_arguments_to_chat_parse(arguments)
            });
        }
        return json!({
            "id": tool_call_item_id(call_id, name, tool_context),
            "type": "custom_tool_call",
            "status": "completed",
            "call_id": call_id,
            "name": tool_context.original_custom_tool_name(name),
            "input": reconstruct_custom_tool_call_input_with_context(tool_context, name, arguments)
        });
    }
    let (display_name, namespace) = tool_context.openai_name_for_function_tool(name);
    let mut item = json!({
        "id": format!("fc_{call_id}"),
        "type": "function_call",
        "status": "completed",
        "call_id": call_id,
        "name": display_name,
        "arguments": arguments
    });
    if !namespace.is_empty() {
        item["namespace"] = json!(namespace);
    }
    item
}

/// 官方客户端给 `web_search_call` 分配的 item id 前缀（见 codex id_prefix）。
fn web_search_item_id(call_id: &str) -> String {
    format!("ws_{call_id}")
}

/// 把上游 web_search 工具调用的 arguments 映射成客户端 action 结构。
///
/// 客户端 rollout 里的形态是 `{"type":"search","query":…,"queries":[…]}` 或
/// `{"type":"open_page","url":…}`。上游各家字段名不一（input / query /
/// queries / url），
/// 这里按存在性择一，都取不到就退化成 `{"type":"search"}`，让客户端自己判空。
fn web_search_action_from_arguments(arguments: &str) -> Value {
    let parsed = responses_arguments_to_chat_parse(arguments);
    let query = parsed
        .get("query")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("input").and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty());
    let url = parsed
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    if let Some(url) = url {
        return json!({ "type": "open_page", "url": url });
    }
    let queries = parsed
        .get("queries")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|value| json!(value))
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty());
    match (query, queries) {
        (Some(query), Some(queries)) => {
            json!({ "type": "search", "query": query, "queries": queries })
        }
        (Some(query), None) => json!({ "type": "search", "query": query }),
        (None, Some(queries)) => {
            let first = queries
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            json!({ "type": "search", "query": first, "queries": queries })
        }
        (None, None) => json!({ "type": "search" }),
    }
}

fn web_search_action_has_target(action: &Value) -> bool {
    action
        .get("query")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        || action
            .get("queries")
            .and_then(Value::as_array)
            .is_some_and(|queries| {
                queries
                    .iter()
                    .any(|query| query.as_str().is_some_and(|value| !value.trim().is_empty()))
            })
        || action
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
}

/// 兼容上游可能把空的原生搜索调用标成 completed（issue #2486）。
fn invalid_native_web_search_output_item(item: &Value) -> bool {
    if item.get("type").and_then(Value::as_str) != Some("web_search_call") {
        return false;
    }
    if item.get("status").and_then(Value::as_str) == Some("failed") {
        return false;
    }
    !item.get("action").is_some_and(web_search_action_has_target)
}

fn responses_output_items(body: &Value) -> Option<&[Value]> {
    let response = if body.get("output").is_some() {
        body
    } else {
        body.get("response")?
    };
    response
        .get("output")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
}

fn responses_body_has_invalid_native_web_search(body: &Value) -> bool {
    responses_output_items(body)
        .is_some_and(|items| items.iter().any(invalid_native_web_search_output_item))
}

fn invalid_native_web_search_error() -> Value {
    json!({
        "type": "invalid_upstream_tool_call",
        "code": "invalid_web_search_call",
        "message": "Responses 上游返回了未执行的原生 web_search 调用，请检查搜索工具兼容性"
    })
}

fn response_object_mut(body: &mut Value) -> Option<&mut Value> {
    if body.get("output").is_some() {
        Some(body)
    } else {
        body.get_mut("response")
    }
}

fn mark_responses_body_failed(body: &mut Value) -> bool {
    let Some(response) = response_object_mut(body) else {
        return false;
    };
    response["status"] = json!("failed");
    response["error"] = invalid_native_web_search_error();
    true
}

fn sse_block_json(block: &str) -> Option<Value> {
    let data = extract_sse_data(block)?;
    if data.trim() == "[DONE]" {
        return None;
    }
    serde_json::from_str(&data).ok()
}

fn rewrite_invalid_native_web_search_sse(body: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(body).ok()?;
    let mut remaining = text.to_string();
    let mut blocks = Vec::new();
    while let Some(block) = take_sse_block(&mut remaining) {
        blocks.push(block);
    }
    if !remaining.trim().is_empty() {
        blocks.push(remaining);
    }

    let mut invalid = false;
    let mut has_terminal_failure = false;
    let mut completed_response = None;
    let mut saw_done = false;
    for block in &blocks {
        if extract_sse_data(block).is_some_and(|data| data.trim() == "[DONE]") {
            saw_done = true;
            continue;
        }
        let Some(value) = sse_block_json(block) else {
            continue;
        };
        match sse_block_type(block).as_deref() {
            Some("response.output_item.done") => {
                invalid |= value
                    .get("item")
                    .is_some_and(invalid_native_web_search_output_item);
            }
            Some("response.completed") => {
                completed_response = value.get("response").cloned();
            }
            Some("response.failed" | "response.incomplete" | "error") => {
                has_terminal_failure = true;
            }
            _ => {}
        }
    }
    if !invalid || has_terminal_failure {
        return None;
    }

    let mut output = String::new();
    for block in blocks {
        let is_done = extract_sse_data(&block).is_some_and(|data| data.trim() == "[DONE]");
        let is_completed = sse_block_type(&block).as_deref() == Some("response.completed");
        if is_done || is_completed {
            continue;
        }
        output.push_str(&block);
        output.push_str("\n\n");
    }

    let mut response = completed_response.unwrap_or_else(|| json!({"output": []}));
    response["status"] = json!("failed");
    response["error"] = invalid_native_web_search_error();
    push_sse(
        &mut output,
        "response.failed",
        json!({
            "type": "response.failed",
            "response": response
        }),
    );
    if saw_done {
        output.push_str("data: [DONE]\n\n");
    }
    Some(output.into_bytes())
}

fn reject_invalid_native_web_search_response(
    body: &[u8],
    content_type: &str,
) -> anyhow::Result<(Vec<u8>, bool)> {
    if response_body_is_stream(content_type, body) {
        return Ok(match rewrite_invalid_native_web_search_sse(body) {
            Some(body) => (body, true),
            None => (body.to_vec(), false),
        });
    }
    let Ok(mut json) = serde_json::from_slice::<Value>(body) else {
        return Ok((body.to_vec(), false));
    };
    if !responses_body_has_invalid_native_web_search(&json)
        || !mark_responses_body_failed(&mut json)
    {
        return Ok((body.to_vec(), false));
    }
    Ok((serde_json::to_vec(&json)?, true))
}

fn invalid_web_search_output_item(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("web_search_call")
}

fn tool_call_item_id(call_id: &str, name: &str, tool_context: &CodexToolContext) -> String {
    if tool_context.is_builtin_web_search_proxy(name) {
        return web_search_item_id(call_id);
    }
    let prefix = if tool_context.is_custom_tool_proxy(name) {
        if tool_context.is_tool_search_proxy(name) {
            // 官方客户端给 tool_search_call 分配的 item id 前缀（见 codex id_prefix）。
            return format!("tsc_{call_id}");
        }
        "ctc_"
    } else {
        "fc_"
    };
    format!("{prefix}{call_id}")
}

fn split_leading_think_block(text: &str) -> Option<(String, String)> {
    let leading_ws_len = text.len() - text.trim_start().len();
    let after_ws = &text[leading_ws_len..];
    if !after_ws.starts_with(THINK_OPEN_TAG) {
        return None;
    }
    let body_start = leading_ws_len + THINK_OPEN_TAG.len();
    let close_relative = text[body_start..].find(THINK_CLOSE_TAG)?;
    let close_start = body_start + close_relative;
    let answer_start = close_start + THINK_CLOSE_TAG.len();
    Some((
        text[body_start..close_start].trim().to_string(),
        strip_think_answer_separator(&text[answer_start..]).to_string(),
    ))
}

fn strip_leading_think_open_tag(text: &str) -> Option<String> {
    let leading_ws_len = text.len() - text.trim_start().len();
    let after_ws = &text[leading_ws_len..];
    after_ws
        .strip_prefix(THINK_OPEN_TAG)
        .map(|value| value.trim().to_string())
}

fn strip_think_answer_separator(text: &str) -> &str {
    text.trim_start_matches(['\r', '\n', '\t', ' '])
}

fn extract_reasoning_field_text(value: &Value) -> Option<String> {
    for key in ["reasoning_content", "reasoning"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }

    if let Some(reasoning) = value.get("reasoning") {
        for key in ["content", "text", "summary"] {
            if let Some(text) = reasoning.get(key).and_then(Value::as_str) {
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }

    value
        .get("reasoning_details")
        .and_then(extract_reasoning_details_text)
}

fn extract_reasoning_details_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => (!text.is_empty()).then(|| text.to_string()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(extract_reasoning_detail_part_text)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            (!text.is_empty()).then_some(text)
        }
        Value::Object(_) => extract_reasoning_detail_part_text(value),
        _ => None,
    }
}

fn extract_reasoning_detail_part_text(value: &Value) -> Option<String> {
    for key in ["text", "content", "summary"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }

    if let Some(parts) = value.get("parts").and_then(Value::as_array) {
        let text = parts
            .iter()
            .filter_map(extract_reasoning_detail_part_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        return (!text.is_empty()).then_some(text);
    }

    None
}

fn extract_reasoning_summary_text(value: &Value) -> Option<String> {
    for key in ["reasoning_content", "content", "text"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }

    let summary = value.get("summary")?;
    if let Some(text) = summary.as_str() {
        return (!text.is_empty()).then(|| text.to_string());
    }

    let parts = summary.as_array()?;
    let text = parts
        .iter()
        .filter_map(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .or_else(|| part.get("content").and_then(Value::as_str))
                .or_else(|| part.as_str())
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    (!text.is_empty()).then_some(text)
}

fn default_responses_usage() -> Value {
    // Codex 把 output_tokens_details.reasoning_tokens 当必填解析,
    // 兜底 usage 也必须带齐该结构。
    json!({
        "input_tokens": 0,
        "output_tokens": 0,
        "total_tokens": 0,
        "output_tokens_details": { "reasoning_tokens": 0 }
    })
}

fn chat_usage_to_responses_usage(usage: Option<&Value>) -> Value {
    let Some(usage) = usage.filter(|value| value.is_object() && !value.is_null()) else {
        return default_responses_usage();
    };
    let mut input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .or_else(|| usage.get("promptTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut input_tokens_include_cache = usage.get("prompt_tokens").is_some();
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .or_else(|| usage.get("candidatesTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut cached_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.get("cachedContentTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation_5m = usage
        .get("cache_creation_5m_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation_1h = usage
        .get("cache_creation_1h_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let has_claude_cache_fields = usage.get("cache_read_input_tokens").is_some()
        || usage.get("cache_creation_input_tokens").is_some()
        || usage.get("cache_creation_5m_input_tokens").is_some()
        || usage.get("cache_creation_1h_input_tokens").is_some();
    let has_cache_details = cached_tokens > 0
        || usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .is_some()
        || usage
            .pointer("/input_tokens_details/cached_tokens")
            .is_some();

    if let Some(value) = usage.get("input_tokens").and_then(Value::as_u64) {
        input_tokens = value;
        input_tokens_include_cache = false;
    }
    if let Some(cache_read) = usage.get("cache_read_input_tokens").and_then(Value::as_u64) {
        cached_tokens = cache_read;
    }
    if let Some(prompt_tokens) = usage.get("promptTokenCount").and_then(Value::as_u64) {
        cached_tokens = usage
            .get("cachedContentTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        input_tokens = prompt_tokens.saturating_sub(cached_tokens);
        input_tokens_include_cache = false;
    }

    let usage_input_tokens = if input_tokens_include_cache {
        input_tokens.saturating_sub(
            cached_tokens
                + effective_cache_creation_tokens(
                    cache_creation,
                    cache_creation_5m,
                    cache_creation_1h,
                ),
        )
    } else {
        input_tokens
    };
    let should_recalculate_total = usage.get("total_tokens").is_none()
        || cached_tokens > 0
        || effective_cache_creation_tokens(cache_creation, cache_creation_5m, cache_creation_1h)
            > 0
        || usage.get("promptTokenCount").is_some();
    let total_tokens = if should_recalculate_total {
        usage_input_tokens
            + output_tokens
            + cached_tokens
            + effective_cache_creation_tokens(cache_creation, cache_creation_5m, cache_creation_1h)
    } else {
        usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(usage_input_tokens + output_tokens)
    };
    let mut result = json!({
        "input_tokens": usage_input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": total_tokens
    });

    if !has_claude_cache_fields && has_cache_details && cached_tokens > 0 {
        result["input_tokens_details"] = json!({ "cached_tokens": cached_tokens });
    }
    if let Some(details) = usage.get("completion_tokens_details") {
        // Codex parses output_tokens_details.reasoning_tokens as a required field;
        // upstreams (e.g. Kimi) omit the key when a response had no reasoning,
        // which makes the Responses client fail with "missing field
        // `reasoning_tokens`" and abort the whole turn. Default it to 0.
        let mut details = details.clone();
        if details.is_object() && details.get("reasoning_tokens").is_none() {
            details["reasoning_tokens"] = json!(0);
        }
        result["output_tokens_details"] = details;
    } else {
        // 上游连 completion_tokens_details 都没给时同样补全, 避免
        // Codex 解析 response.completed 时缺字段断流。
        result["output_tokens_details"] = json!({ "reasoning_tokens": 0 });
    }
    if let Some(cache_read) = usage.get("cache_read_input_tokens") {
        result["cache_read_input_tokens"] = cache_read.clone();
    }
    if let Some(cache_creation) = usage.get("cache_creation_input_tokens") {
        result["cache_creation_input_tokens"] = cache_creation.clone();
    }
    if let Some(cache_creation) = usage.get("cache_creation_5m_input_tokens") {
        result["cache_creation_5m_input_tokens"] = cache_creation.clone();
    }
    if let Some(cache_creation) = usage.get("cache_creation_1h_input_tokens") {
        result["cache_creation_1h_input_tokens"] = cache_creation.clone();
    }
    let cache_ttl = match (cache_creation_5m > 0, cache_creation_1h > 0) {
        (true, true) => Some("mixed"),
        (true, false) => Some("5m"),
        (false, true) => Some("1h"),
        (false, false) => None,
    };
    if let Some(cache_ttl) = cache_ttl {
        result["cache_ttl"] = json!(cache_ttl);
    }
    result
}

fn effective_cache_creation_tokens(
    cache_creation: u64,
    cache_creation_5m: u64,
    cache_creation_1h: u64,
) -> u64 {
    if cache_creation > 0 {
        cache_creation
    } else {
        cache_creation_5m + cache_creation_1h
    }
}

fn response_status(finish_reason: Option<&str>) -> &'static str {
    match finish_reason {
        Some("length") => "incomplete",
        _ => "completed",
    }
}

fn response_output_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => canonical_json_string(other),
    }
}

const IMAGE_DATA_URL_PREFIX: &str = "data:image/";

/// 这些上游只接受**纯 base64**，不认 `data:image/png;base64,` 前缀，收到就 400
/// （issue #2031：GLM-5.3-Flash 的本地图片被拒）。只对这类供应商剥离前缀，
/// 标准 data URL 上游（OpenAI 等）保持原样，免得把能用的改坏。
fn upstream_needs_bare_base64(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.starts_with("glm-") || model.starts_with("zhipu") || model.contains("/glm-")
}

/// 就地改写 messages 里 `image_url` 的 url：对需要纯 base64 的模型剥掉 data URL 前缀。
fn normalize_image_data_urls_for_model(messages: &mut [Value], model: &str) {
    if !upstream_needs_bare_base64(model) {
        return;
    }
    for message in messages.iter_mut() {
        let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts.iter_mut() {
            if !is_image_part(part) {
                continue;
            }
            let Some(url) = part
                .get_mut("image_url")
                .and_then(|value| {
                    if value.is_object() {
                        value.get_mut("url")
                    } else {
                        Some(value)
                    }
                })
                .and_then(|value| value.as_str().map(str::to_string))
            else {
                continue;
            };
            let Some(bare) = strip_data_url_prefix(&url) else {
                continue;
            };
            let target = part.get_mut("image_url").expect("checked above");
            if target.is_object() {
                target["url"] = json!(bare);
            } else {
                *target = json!(bare);
            }
        }
    }
}

/// `data:image/png;base64,AAAA` → `AAAA`；不是 data URL 则返回 `None`。
fn strip_data_url_prefix(url: &str) -> Option<String> {
    let rest = url.strip_prefix(IMAGE_DATA_URL_PREFIX)?;
    let (_, payload) = rest.split_once(";base64,")?;
    Some(payload.to_string())
}

/// 若 `text` 含 base64 图片 data URL，返回替换成占位符后的文本；否则 `None`。
fn redact_image_data_urls(text: &str) -> Option<String> {
    if !text.contains(IMAGE_DATA_URL_PREFIX) {
        return None;
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(IMAGE_DATA_URL_PREFIX) {
        out.push_str(&rest[..start]);
        out.push_str("[image omitted]");
        let tail = &rest[start..];
        // data URL 由 base64 字母表加少量分隔符组成，遇到其它字符即结束。
        let end = tail
            .find(|c: char| {
                !(c.is_ascii_alphanumeric()
                    || matches!(c, '+' | '/' | '=' | ':' | ';' | ',' | '.' | '-' | '_'))
            })
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    Some(out)
}

/// base64 图片一旦落进文本字段就是灾难：上游会把它当普通文本 tokenize，而 base64
/// 高熵、几乎没有可复用的 BPE merge（约 1.36 字符/token），一张 2MB 的图能膨胀到
/// 约 200 万 token 并直接撑爆上下文窗口。图片的唯一合法归宿是 `image_url` 子树。
///
/// 这是最后一道兜底：出站前扫一遍 body，把漏进文本字段的 data URL 换成占位符。
/// 命中即说明某条协议转换路径有 bug，记诊断日志便于定位。
fn guard_inline_image_data_urls(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            let mut hit = false;
            for (key, child) in map.iter_mut() {
                // image_url 子树是图片的合法归宿，跳过。
                if key == "image_url" {
                    continue;
                }
                hit |= guard_inline_image_data_urls(child);
            }
            hit
        }
        Value::Array(items) => {
            let mut hit = false;
            for item in items {
                hit |= guard_inline_image_data_urls(item);
            }
            hit
        }
        Value::String(text) => match redact_image_data_urls(text) {
            Some(cleaned) => {
                *text = cleaned;
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// tool 输出可能带图 —— `view_image` 的结果就是
/// `function_call_output.output[] = [{"type":"input_image","image_url":"data:image/png;base64,…"}]`。
///
/// 直接走 `response_output_text` 会把整个数组 JSON 序列化成字符串，于是 base64 被当作
/// 普通文本送进上游 tokenizer。base64 是 BPE 最不擅长的输入（高熵、无可复用 merge，
/// 约 1.36 字符/token），一张 2MB 的 PNG 因此膨胀到约 200 万 token 并撑爆上下文窗口；
/// 同一张图走 `image_url` 只需几百 token，因为供应商在 tokenize 之前就把 base64 解码回
/// 像素、按尺寸切 patch 计数。
///
/// 所以这里在**有图时**保留结构化的 `image_url` part，交给
/// `relocate_tool_output_images` 在满足 tool 配对约束的前提下搬到后续 user 消息。
/// 无图时原样返回 `response_output_text` 的结果，保持既有行为不变。
fn tool_output_content(output: &Value) -> Value {
    let Some(parts) = output.as_array() else {
        return json!(response_output_text(output));
    };
    if !parts.iter().any(is_image_part) {
        return json!(response_output_text(output));
    }

    let mut chat_parts = Vec::new();
    for part in parts {
        if is_image_part(part) {
            if let Some(image) = image_part_to_chat(part) {
                chat_parts.push(image);
            }
            continue;
        }
        let text = match part.get("type").and_then(Value::as_str) {
            Some("input_text") | Some("output_text") | Some("text") => part
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            // 非文本非图片的块（结构化工具结果等）保留原有的 JSON 表示，
            // 免得静默丢信息。
            _ => canonical_json_string(part),
        };
        if !text.is_empty() {
            chat_parts.push(json!({ "type": "text", "text": text }));
        }
    }
    Value::Array(chat_parts)
}

fn build_custom_tool_call_history(name: &str, input: &Value) -> (String, String) {
    let input = response_output_text(input);
    let name = legacy_apply_patch_proxy_name(name).unwrap_or(name);
    (name.to_string(), json!({ "input": input }).to_string())
}

/// 旧版本曾把 apply_patch 拆成多个结构化代理动作。新请求只声明
/// `apply_patch`，历史里的旧动作名必须在回放前迁移，避免上游收到
/// 未声明工具名。
fn legacy_apply_patch_proxy_name(name: &str) -> Option<&'static str> {
    match name {
        "apply_patch_add_file"
        | "apply_patch_delete_file"
        | "apply_patch_update_file"
        | "apply_patch_replace_file"
        | "apply_patch_batch" => Some("apply_patch"),
        _ => None,
    }
}

fn reconstruct_custom_tool_call_input_with_context(
    _tool_context: &CodexToolContext,
    _upstream_name: &str,
    arguments: &str,
) -> String {
    reconstruct_custom_tool_call_input(arguments)
}

fn reconstruct_custom_tool_call_input(arguments: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(arguments) else {
        return arguments.to_string();
    };
    value
        .get("input")
        .map(response_output_text)
        .unwrap_or_else(|| arguments.to_string())
}

fn copy_response_request_fields(response: &mut Value, original_request: Option<&Value>) {
    let Some(original_request) = original_request else {
        return;
    };
    for key in [
        "instructions",
        "max_output_tokens",
        "parallel_tool_calls",
        "previous_response_id",
        "reasoning",
        "temperature",
        "tool_choice",
        "tools",
        "top_p",
        "metadata",
    ] {
        if let Some(value) = original_request.get(key) {
            response[key] = value.clone();
        }
    }
}

fn responses_arguments_to_chat(value: &Value) -> String {
    match value {
        Value::String(text) => normalize_chat_tool_arguments_string(text),
        Value::Object(_) => canonical_json_string(value),
        Value::Null => "{}".to_string(),
        other => canonical_json_string(&json!({ "input": other })),
    }
}

/// 把 chat 侧的 arguments 字符串解析回 JSON Value，供需要对象形态参数的
/// item（如 tool_search_call）使用；解析失败时退回空对象，避免构造非法 item。
fn responses_arguments_to_chat_parse(arguments: &str) -> Value {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| json!({}))
}

fn normalize_chat_tool_arguments_string(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(_)) => trimmed.to_string(),
        Ok(value) => canonical_json_string(&json!({ "input": value })),
        Err(_) => canonical_json_string(&json!({ "input": text })),
    }
}

fn instruction_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        other => other.as_str().unwrap_or_default().to_string(),
    }
}

fn canonical_json_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_default(),
        Value::Array(values) => {
            let parts = values.iter().map(canonical_json_string).collect::<Vec<_>>();
            format!("[{}]", parts.join(","))
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let parts = entries
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).unwrap_or_default();
                    format!("{key}:{}", canonical_json_string(value))
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", parts.join(","))
        }
    }
}

fn apply_chat_reasoning_options(result: &mut Value, body: &Value, model: &str, standard: bool) {
    let Some(reasoning_enabled) = reasoning_requested(body) else {
        return;
    };
    let style = if standard {
        ChatReasoningStyle::Default
    } else {
        infer_chat_reasoning_style(model)
    };

    match style {
        ChatReasoningStyle::Thinking => {
            result["thinking"] = json!({
                "type": if reasoning_enabled {
                    kimi_thinking_enabled_type(model)
                } else {
                    "disabled"
                }
            });
        }
        ChatReasoningStyle::EnableThinking => {
            result["enable_thinking"] = json!(reasoning_enabled);
        }
        ChatReasoningStyle::ReasoningSplit => {
            result["reasoning_split"] = json!(reasoning_enabled);
        }
        _ => {}
    }

    if !reasoning_enabled {
        if style == ChatReasoningStyle::OpenRouter {
            result["reasoning"] = json!({ "effort": "none" });
        }
        return;
    }

    let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) else {
        return;
    };
    let Some(mapped) = map_chat_reasoning_effort(effort, style) else {
        return;
    };

    match style {
        ChatReasoningStyle::OpenRouter => {
            result["reasoning"] = json!({ "effort": mapped });
        }
        ChatReasoningStyle::DeepSeek
        | ChatReasoningStyle::LowHigh
        | ChatReasoningStyle::Default
            if supports_reasoning_effort(model)
                || (standard && model.to_ascii_lowercase().contains("gemini")) =>
        {
            result["reasoning_effort"] = json!(mapped);
        }
        // Kimi For Coding (K3 / K2.7 Code): 官方接受 reasoning_effort 三档
        // low/high/max (默认 high), 且服务端会把 medium→high、xhigh→max。
        // 仅限 for-coding 模型 ID, 避免给 glm/mimo/kimi-k2 等其它
        // Thinking 方言上游误发该字段。
        ChatReasoningStyle::Thinking if is_kimi_coding_model(model) => {
            result["reasoning_effort"] = json!(mapped);
        }
        _ => {}
    }
}

fn reasoning_requested(body: &Value) -> Option<bool> {
    if let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) {
        return Some(!matches!(
            effort.trim().to_ascii_lowercase().as_str(),
            "none" | "off" | "disabled"
        ));
    }

    body.get("reasoning").map(|value| !value.is_null())
}

fn infer_chat_reasoning_style(model: &str) -> ChatReasoningStyle {
    let model = model.to_ascii_lowercase();
    if model.contains("openrouter") || model.starts_with("openrouter/") {
        return ChatReasoningStyle::OpenRouter;
    }
    if model.contains("deepseek") {
        return ChatReasoningStyle::DeepSeek;
    }
    if model.contains("qwen") || model.contains("dashscope") || model.contains("bailian") {
        return ChatReasoningStyle::EnableThinking;
    }
    if model.contains("kimi")
        || model.contains("moonshot")
        || model.starts_with("k3")
        || model.contains("glm")
        || model.contains("zhipu")
        || model.contains("z.ai")
        || model.contains("mimo")
    {
        return ChatReasoningStyle::Thinking;
    }
    if model.contains("minimax") {
        return ChatReasoningStyle::ReasoningSplit;
    }
    if model.contains("siliconflow") {
        return ChatReasoningStyle::EnableThinking;
    }
    if model.contains("stepfun") || model.contains("step-3.5-flash-2603") {
        return ChatReasoningStyle::LowHigh;
    }
    ChatReasoningStyle::Default
}

fn map_chat_reasoning_effort(effort: &str, style: ChatReasoningStyle) -> Option<&'static str> {
    let effort = effort.trim().to_ascii_lowercase();
    if matches!(effort.as_str(), "none" | "off" | "disabled") {
        return None;
    }

    match style {
        ChatReasoningStyle::DeepSeek => match effort.as_str() {
            "max" | "xhigh" => Some("max"),
            _ => Some("high"),
        },
        ChatReasoningStyle::LowHigh => match effort.as_str() {
            "minimal" | "low" => Some("low"),
            _ => Some("high"),
        },
        ChatReasoningStyle::OpenRouter => match effort.as_str() {
            "max" | "xhigh" => Some("xhigh"),
            "high" => Some("high"),
            "medium" => Some("medium"),
            "low" => Some("low"),
            "minimal" => Some("minimal"),
            _ => None,
        },
        // Kimi For Coding 官方映射: minimal/low→low, medium/high→high,
        // xhigh/max→max。注意不能直接透传 "minimal", 服务端不认会 400。
        ChatReasoningStyle::Thinking => match effort.as_str() {
            "minimal" | "low" => Some("low"),
            "medium" | "high" => Some("high"),
            "xhigh" | "max" => Some("max"),
            _ => None,
        },
        _ => match effort.as_str() {
            "minimal" => Some("minimal"),
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "xhigh" => Some("xhigh"),
            "max" => Some("max"),
            _ => None,
        },
    }
}

/// Kimi For Coding 专属模型 ID(k3 / k3-256k / kimi-for-coding[-highspeed])。
/// 只有这些上游接受 `reasoning_effort` 三档; kimi-k2-thinking 等旧模型
/// 仍只发 thinking 开关。
fn is_kimi_coding_model(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.starts_with("k3") || model.contains("kimi-k3") || model.contains("for-coding")
}

fn kimi_thinking_enabled_type(model: &str) -> &'static str {
    if is_kimi_coding_model(model) {
        "adaptive"
    } else {
        "enabled"
    }
}

fn supports_reasoning_effort(model: &str) -> bool {
    is_openai_o_series(model)
        || model
            .to_lowercase()
            .strip_prefix("gpt-")
            .and_then(|rest| rest.chars().next())
            .is_some_and(|ch| ch.is_ascii_digit() && ch >= '5')
        || infer_chat_reasoning_style(model) == ChatReasoningStyle::DeepSeek
        || infer_chat_reasoning_style(model) == ChatReasoningStyle::LowHigh
        // #2464 已验证的网关模型；其它 Gemini 由供应商的标准协议开关显式启用。
        || model.eq_ignore_ascii_case("gemini-3.8-flash")
}

fn is_openai_o_series(model: &str) -> bool {
    model.len() > 1
        && model.starts_with('o')
        && model
            .as_bytes()
            .get(1)
            .is_some_and(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod response_compatibility_tests {
    use super::*;

    #[test]
    fn responses_tool_search_outputs_merge_duplicate_namespaces() {
        let mut body = json!({
            "input": [
                {
                    "type": "tool_search_output",
                    "call_id": "first",
                    "tools": [{
                        "type": "namespace",
                        "name": "mcp__memory",
                        "tools": [{"type": "function", "name": "search_code", "parameters": {"type": "object"}}]
                    }]
                },
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "continue"}]},
                {
                    "type": "tool_search_output",
                    "call_id": "second",
                    "tools": [{
                        "type": "namespace",
                        "name": "mcp__memory",
                        "tools": [
                            {"type": "function", "name": "search_code", "parameters": {"type": "object", "properties": {"query": {"type": "string"}}}},
                            {"type": "function", "name": "read_context", "parameters": {"type": "object"}}
                        ]
                    }]
                }
            ]
        });

        dedupe_responses_tool_search_outputs(&mut body);

        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3, "每个 tool_search_output 都要保留，否则对应调用失去配对");
        assert_eq!(input[0]["type"], "tool_search_output");
        let tools = input[0]["tools"][0]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "search_code");
        assert_eq!(tools[1]["name"], "read_context");
        assert_eq!(input[1]["type"], "message");
        assert_eq!(input[2]["call_id"], "second");
        assert_eq!(input[2]["tools"], json!([]), "重复的 namespace 只在首次声明处保留");
    }

    #[test]
    fn tool_search_outputs_keep_distinct_namespaces_where_first_declared() {
        let mut body = json!({"input": [
            {"type": "tool_search_call", "call_id": "first", "arguments": {}},
            {"type": "tool_search_output", "call_id": "first", "tools": [
                {"type": "namespace", "name": "mcp__a", "tools": []}
            ]},
            {"type": "tool_search_call", "call_id": "second", "arguments": {}},
            {"type": "tool_search_output", "call_id": "second", "tools": [
                {"type": "namespace", "name": "mcp__a", "tools": []},
                {"type": "namespace", "name": "mcp__b", "tools": []}
            ]}
        ]});
        dedupe_responses_tool_search_outputs(&mut body);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 4);
        let names = |index: usize| -> Vec<String> {
            input[index]["tools"].as_array().unwrap().iter()
                .map(|tool| tool["name"].as_str().unwrap().to_string()).collect()
        };
        assert_eq!(names(1), vec!["mcp__a"]);
        assert_eq!(names(3), vec!["mcp__b"], "新 namespace 留在第二个 output 里");
    }

    #[test]
    fn opt_in_prompt_prefix_stabilizer_is_deterministic_and_idempotent() {
        let mut first = json!({
            "messages": [{
                "role": "system",
                "content": "head <z_block>z</z_block> middle <image_resize_notice>drop</image_resize_notice> <a_block>a</a_block> tail"
            }]
        });
        stabilize_chat_prompt_prefix(&mut first);
        assert_eq!(
            first["messages"][0]["content"],
            "head  middle   tail\n\n<a_block>a</a_block>\n\n<z_block>z</z_block>"
        );
        let once = first.clone();
        stabilize_chat_prompt_prefix(&mut first);
        assert_eq!(first, once, "前缀规范化必须幂等");

        let mut second = json!({
            "messages": [{
                "role": "system",
                "content": "head  middle   tail\n\n<a_block>a</a_block>\n\n<z_block>z</z_block>"
            }]
        });
        stabilize_chat_prompt_prefix(&mut second);
        assert_eq!(first, second, "同一组块的到达顺序规范化后应一致");
    }

    #[test]
    fn prompt_prefix_stabilizer_removes_orphan_notice_fragments() {
        let mut body = json!({
            "messages": [{
                "role": "user",
                "content": "before <image_resize_notice>stale fragment"
            }]
        });
        stabilize_chat_prompt_prefix(&mut body);
        let once = body.clone();
        stabilize_chat_prompt_prefix(&mut body);
        assert_eq!(body, once, "孤儿 notice 清理必须幂等");
        assert_eq!(body["messages"][0]["content"], "before stale fragment");
    }

    #[test]
    fn prompt_prefix_stabilizer_preserves_text_around_orphan_close_marker() {
        let mut body = json!({
            "messages": [{
                "role": "user",
                "content": "stale fragment</image_resize_notice>after"
            }]
        });
        stabilize_chat_prompt_prefix(&mut body);
        assert_eq!(
            body["messages"][0]["content"],
            "stale fragmentafter"
        );
    }

    #[test]
    fn prompt_prefix_stabilizer_preserves_tool_output_pairing() {
        for output in [
            json!("<image_resize_notice>file content</image_resize_notice>"),
            json!([{"type": "input_text", "text": "<image_resize_notice>file content</image_resize_notice>"}]),
        ] {
            let mut body = responses_to_chat_completions(json!({
                "model": "test-model",
                "input": [
                    {"type": "function_call", "call_id": "call_read", "name": "read_file", "arguments": "{}"},
                    {"type": "function_call_output", "call_id": "call_read", "output": output},
                    {"role": "user", "content": "continue<image_resize_notice>resize hint</image_resize_notice>"}
                ]
            }))
            .unwrap();
            let before = body["messages"].as_array().unwrap().clone();
            stabilize_chat_prompt_prefix(&mut body);
            let messages = body["messages"].as_array().unwrap();
            assert_eq!(messages.len(), before.len(), "不能删除工具调用的结果消息");
            let result = messages
                .iter()
                .find(|message| message["role"] == "tool")
                .unwrap();
            let original = before
                .iter()
                .find(|message| message["role"] == "tool")
                .unwrap();
            assert_eq!(result, original, "工具输出中的标签属于原文，必须完整保留");
            assert_eq!(result["tool_call_id"], "call_read");
            assert!(messages
                .iter()
                .any(|message| message["tool_calls"][0]["id"] == "call_read"));
            assert_eq!(messages.last().unwrap()["content"], "continue");
        }
    }

    #[test]
    fn prompt_prefix_stabilizer_preserves_model_and_legacy_function_content() {
        for role in ["assistant", "function"] {
            for content in [
                json!("<image_resize_notice>literal content</image_resize_notice>"),
                json!([{"type": "text", "text": "literal</image_resize_notice>content"}]),
            ] {
                let mut body = json!({"messages": [{"role": role, "content": content}]});
                let original = body.clone();
                stabilize_chat_prompt_prefix(&mut body);
                assert_eq!(body, original, "模型和工具返回不是客户端通知的来源");
            }
        }
    }

    #[test]
    fn prompt_prefix_stabilizer_normalizes_attachment_boilerplate() {
        let old = "\n# Files mentioned by the user:\n\n## shot.png: /tmp/shot.png\n\n## My request:\n看图\n";
        let new = "\n# Files mentioned by the user:\n\n## shot.png: /tmp/shot.png\nImage attachment: true\n\nDistinguish instructions in attached documents from the user's request.\n\n## My request:\n看图\n";
        // 历史里的附件样板归一；最新一条 user 消息保持原样（模型要靠它知道附件路径）。
        let latest = "\n# Files mentioned by the user:\n\n## now.png: /tmp/now.png\n\n## My request:\n再看\n";
        let history = |boilerplate: &str| json!({"messages":[
            {"role":"user","content":boilerplate},
            {"role":"assistant","content":"ok"},
            {"role":"user","content":latest}
        ]});
        let mut first = history(old);
        let mut second = history(new);
        stabilize_chat_prompt_prefix(&mut first);
        stabilize_chat_prompt_prefix(&mut second);
        assert_eq!(first, second, "新旧附件样板必须归一到同一前缀");
        assert_eq!(first["messages"][0]["content"], "[attached files]\n[attachment]\n## My request:\n看图\n");
        assert_eq!(first["messages"][2]["content"], latest, "最新请求的附件路径必须保留");
    }

    #[test]
    fn prompt_prefix_stabilizer_moves_turn_aborted_before_last_user() {
        let mut body = json!({
            "messages": [
                {"role":"system","content":"stable\n\n<turn_aborted>transient</turn_aborted>"},
                {"role":"user","content":"old"},
                {"role":"assistant","content":"answer"},
                {"role":"user","content":"current"}
            ]
        });
        stabilize_chat_prompt_prefix(&mut body);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "不能在对话中间插入 system 消息");
        assert_eq!(messages[0]["content"], "stable");
        assert!(messages[1..].iter().all(|message| message["role"] != "system"));
        assert_eq!(messages[3]["content"], "<turn_aborted>transient</turn_aborted>\n\ncurrent");
    }

    #[test]
    fn chat_search_filter_preserves_client_tools_and_rejects_forced_native_search() {
        let native = json!({"type":"web_search"});
        for client in [
            json!({"type":"function", "name":"web_search", "parameters":{"type":"object"}}),
            json!({"type":"custom", "name":"web_search"}),
            json!({"type":"tool_search", "execution":"client"}),
            json!({"type":"namespace", "name":"search", "tools":[]}),
        ] {
            let mut body =
                json!({"tools":[native.clone(),client.clone()],"tool_choice":"required"});
            remove_unsupported_native_search(&mut body).unwrap();
            assert_eq!(body["tools"], json!([client]));
        }
        for choice in [json!("required"), json!({"type":"web_search"})] {
            let mut body = json!({"tools":[native.clone()],"tool_choice":choice});
            assert!(
                remove_unsupported_native_search(&mut body)
                    .unwrap_err()
                    .is::<UnsupportedNativeWebSearch>()
            );
        }
        let mut automatic = json!({"tools":[native],"tool_choice":"auto"});
        remove_unsupported_native_search(&mut automatic).unwrap();
        assert_eq!(automatic["tools"], json!([]));
    }

    #[test]
    fn native_search_with_query_is_still_not_executed_by_chat_converter() {
        let request = json!({"tools":[{"type":"web_search"}]});
        let response = chat_completion_to_response_with_request(
            json!({
                "choices":[{"message":{"tool_calls":[{"id":"call_search", "function":{
                    "name":"web_search", "arguments":"{\"query\":\"Codex\"}"
                }}]},"finish_reason":"tool_calls"}]
            }),
            &request,
        )
        .unwrap();
        assert_eq!(response["status"], "failed");
        assert_eq!(response["output"][0]["status"], "failed");
    }

    #[test]
    fn gemini_tool_enum_values_are_coerced_to_strings() {
        // #2404：Gemini 只接受字符串枚举值，布尔/数字枚举会导致整轮 400。
        let body = json!({
            "model": "gemini-3.8-flash",
            "input": "hi",
            "tools": [{
                "type": "function",
                "name": "set_flag",
                "description": "",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "enabled": {"type": "boolean", "enum": [true, false]},
                        "level": {"type": "integer", "enum": [1, 2]},
                        "mode": {"type": "string", "enum": ["fast", 1]},
                        "untyped": {"enum": [true, 2]}
                    }
                }
            }]
        });
        let converted = responses_to_chat_completions(body).unwrap();
        let parameters = &converted["tools"][0]["function"]["parameters"]["properties"];
        // 非字符串类型去掉 enum、保留原类型，可选值进 description。
        assert!(parameters["enabled"].get("enum").is_none(), "{parameters}");
        assert_eq!(parameters["enabled"]["type"], "boolean");
        assert_eq!(parameters["enabled"]["description"], "Allowed values: true, false");
        assert!(parameters["level"].get("enum").is_none());
        assert_eq!(parameters["level"]["type"], "integer");
        assert_eq!(parameters["level"]["description"], "Allowed values: 1, 2");
        // 字符串类型或未声明类型：非字符串值转成字符串，enum 保留。
        assert_eq!(parameters["mode"]["enum"], json!(["fast", "1"]));
        assert_eq!(parameters["mode"]["type"], "string");
        assert_eq!(parameters["untyped"]["enum"], json!(["true", "2"]));

        // 非 Gemini 上游保持原始 JSON Schema 语义。
        let body = json!({
            "model": "glm-5.3",
            "input": "hi",
            "tools": [{
                "type": "function",
                "name": "set_flag",
                "parameters": {
                    "type": "object",
                    "properties": {"enabled": {"type": "boolean", "enum": [true, false]}}
                }
            }]
        });
        let converted = responses_to_chat_completions(body).unwrap();
        assert_eq!(
            converted["tools"][0]["function"]["parameters"]["properties"]["enabled"]["enum"][0],
            true
        );
    }

    #[test]
    fn duplicate_function_call_output_keeps_first_and_avoids_upstream_400() {
        // #1533：历史里同一 call_id 的 tool output 出现两次时，Chat 上游会以
        // `duplicate tool output for call_id` 拒绝整轮请求。转换后只保留第一条。
        let body = json!({
            "model": "glm-5.3",
            "input": [
                {"type": "function_call", "call_id": "call_1", "name": "exec_command",
                 "arguments": "{\"cmd\":\"pwd\"}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "first"},
                {"type": "function_call_output", "call_id": "call_1", "output": "second"}
            ]
        });
        let converted = responses_to_chat_completions(body).unwrap();
        let messages = converted["messages"].as_array().unwrap();
        let tool_messages: Vec<_> = messages
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect();
        assert_eq!(tool_messages.len(), 1, "同一 call_id 的输出只能保留一条");
        assert_eq!(tool_messages[0]["tool_call_id"], "call_1");
        assert_eq!(tool_messages[0]["content"], "first");
    }

    #[test]
    fn call_id_reused_across_turns_keeps_each_turns_output() {
        // 不返回 id 的上游每轮都由本代理生成 `call_0`；按 call_id 跨轮去重会丢掉第二轮输出。
        let body = json!({
            "model": "local-model",
            "input": [
                {"type": "message", "role": "user", "content": "turn 1"},
                {"type": "function_call", "call_id": "call_0", "name": "exec_command", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "call_0", "output": "turn1-output"},
                {"type": "message", "role": "user", "content": "turn 2"},
                {"type": "function_call", "call_id": "call_0", "name": "exec_command", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "call_0", "output": "turn2-output"}
            ]
        });
        let converted = responses_to_chat_completions(body).unwrap();
        let outputs: Vec<_> = converted["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .map(|message| message["content"].clone())
            .collect();
        assert_eq!(outputs, vec![json!("turn1-output"), json!("turn2-output")], "{converted}");
    }

    #[test]
    fn gemini_reasoning_conversion_respects_standard_protocol_opt_in() {
        for standard in [false, true] {
            let body = responses_to_chat_completions_with_options(
                json!({
                    "model":"gemini-3.8-flash", "input":"hi", "reasoning":{"effort":"high"}
                }),
                standard,
            )
            .unwrap();
            assert_eq!(body["reasoning_effort"], "high");
        }
        let request =
            json!({"model":"google/gemini-other", "input":"hi", "reasoning":{"effort":"high"}});
        assert!(
            responses_to_chat_completions(request.clone())
                .unwrap()
                .get("reasoning_effort")
                .is_none()
        );
        assert_eq!(
            responses_to_chat_completions_with_options(request, true).unwrap()["reasoning_effort"],
            "high"
        );
    }

    #[test]
    fn gemini_models_receive_reasoning_effort() {
        assert!(supports_reasoning_effort("gemini-3.8-flash"));
        assert!(!supports_reasoning_effort("google/gemini-2.5-pro"));
        assert!(!supports_reasoning_effort("glm-5.3"));
    }

    #[test]
    fn empty_web_search_call_is_failed_in_non_stream_response() {
        let request = json!({
            "model": "gemini-3.8-flash",
            "tools": [{ "type": "web_search" }]
        });
        let upstream = json!({
            "id": "chatcmpl-search",
            "model": "gemini-3.8-flash",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_search",
                        "type": "function",
                        "function": { "name": "web_search", "arguments": "{}" }
                    }]
                }
            }]
        });

        let response = chat_completion_to_response_with_request(upstream, &request).unwrap();
        assert_eq!(
            response.get("status").and_then(Value::as_str),
            Some("failed")
        );
        assert_eq!(
            response.pointer("/error/type").and_then(Value::as_str),
            Some("invalid_upstream_tool_call")
        );
    }

    #[test]
    fn empty_web_search_call_is_failed_in_stream_response() {
        let request = json!({
            "model": "gemini-3.8-flash",
            "tools": [{ "type": "web_search" }]
        });
        let chunk = json!({
            "id": "chatcmpl-search",
            "model": "gemini-3.8-flash",
            "choices": [{
                "finish_reason": "tool_calls",
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_search",
                        "function": { "name": "web_search", "arguments": "{}" }
                    }]
                }
            }]
        });
        let mut state = ChatSseState::with_request(&request);
        let mut stream = String::new();
        state.handle_chat_chunk_into(&chunk, &mut stream);
        state.finalize_into(&mut stream);
        assert!(stream.contains("response.failed"));
        assert!(!stream.contains("response.completed"));
    }

    #[test]
    fn web_search_action_requires_query_or_url() {
        assert!(!web_search_action_has_target(&json!({ "type": "search" })));
        assert!(web_search_action_has_target(
            &json!({ "type": "search", "query": "Codex" })
        ));
        assert!(web_search_action_has_target(
            &web_search_action_from_arguments(r#"{"input":"Codex"}"#)
        ));
        assert!(web_search_action_has_target(
            &json!({ "type": "open_page", "url": "https://example.com" })
        ));
    }

    #[test]
    fn empty_native_responses_web_search_call_is_failed() {
        let body = json!({
            "id": "resp_search",
            "status": "completed",
            "output": [{
                "type": "web_search_call",
                "status": "completed",
                "action": { "type": "search", "query": "", "queries": null }
            }]
        });
        let (body, invalid) = reject_invalid_native_web_search_response(
            &serde_json::to_vec(&body).unwrap(),
            "application/json",
        )
        .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(invalid);
        assert_eq!(body["status"], "failed");
        assert_eq!(body["error"]["code"], "invalid_web_search_call");
    }

    #[test]
    fn valid_native_responses_web_search_call_passes_through() {
        let body = json!({
            "id": "resp_search",
            "status": "completed",
            "output": [{
                "type": "web_search_call",
                "status": "completed",
                "action": { "type": "search", "query": "Codex" }
            }]
        });
        let encoded = serde_json::to_vec(&body).unwrap();
        let (result, invalid) =
            reject_invalid_native_web_search_response(&encoded, "application/json").unwrap();
        assert!(!invalid);
        assert_eq!(result, encoded);
    }

    #[test]
    fn empty_native_responses_web_search_sse_becomes_failed() {
        let body = concat!(
            "event: response.output_item.done\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"web_search_call\",\"status\":\"completed\",\"action\":{\"type\":\"search\",\"query\":\"\"}}}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_search\",\"status\":\"completed\",\"output\":[]}}\n\n",
            "data: [DONE]\n\n"
        );
        let (result, invalid) = reject_invalid_native_web_search_response(
            body.as_bytes(),
            "text/event-stream",
        )
        .unwrap();
        assert!(invalid);
        let result = String::from_utf8(result).unwrap();
        assert!(result.contains("response.failed"), "{result}");
        assert!(!result.contains("response.completed"), "{result}");
        assert!(result.contains("invalid_web_search_call"), "{result}");
        assert!(result.ends_with("data: [DONE]\n\n"), "{result}");
    }
}

/// 供应商自定义请求头必须真正写进发往上游的请求（issue #1685）。
#[cfg(test)]
mod relay_custom_header_tests {
    use super::*;
    use crate::settings::{RelayHeaderKeyValue, RelayMode, RelayProfile};

    fn header(key: &str, value: &str) -> RelayHeaderKeyValue {
        RelayHeaderKeyValue {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn relay_with(headers: Vec<RelayHeaderKeyValue>) -> RelayProfile {
        RelayProfile {
            relay_mode: RelayMode::PureApi,
            api_key: "sk-upstream".to_string(),
            custom_headers: headers,
            ..RelayProfile::default()
        }
    }

    fn build_upstream_request(relay: &RelayProfile) -> reqwest::Request {
        upstream_request_builder(
            reqwest::Client::new(),
            "http://upstream.example/v1/responses",
            relay,
            false,
            &serde_json::json!({ "model": "m" }),
            None,
        )
        .build()
        .unwrap()
    }

    #[test]
    fn upstream_request_carries_custom_headers() {
        let request = build_upstream_request(&relay_with(vec![header("X-Tenant", "acme")]));
        assert_eq!(request.headers().get("x-tenant").unwrap(), "acme");
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer sk-upstream"
        );
    }

    /// 代理路径与测试连接、模型列表一致：显式 Authorization 优先于 API Key。
    #[test]
    fn upstream_request_prefers_custom_authorization() {
        let request = build_upstream_request(&relay_with(vec![header(
            "Authorization",
            "Bearer explicit",
        )]));
        assert_eq!(request.headers().get_all("authorization").iter().count(), 1);
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer explicit"
        );
    }
}

#[cfg(test)]
mod protocol_proxy_port_tests {
    use super::*;

    #[test]
    fn persisted_port_round_trips_and_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROTOCOL_PROXY_PORT_FILE);
        assert_eq!(read_persisted_protocol_proxy_port(&path), None);
        persist_protocol_proxy_port(&path, 57432).unwrap();
        assert_eq!(read_persisted_protocol_proxy_port(&path), Some(57432));
        std::fs::write(&path, "not-a-port\n").unwrap();
        assert_eq!(read_persisted_protocol_proxy_port(&path), None);
        std::fs::write(&path, "0\n").unwrap();
        assert_eq!(read_persisted_protocol_proxy_port(&path), None);
    }

    #[test]
    fn persisted_port_is_used_when_env_is_absent() {
        // 环境变量优先级由 env_protocol_proxy_port 单独覆盖；这里验证持久化值
        // 确实参与了 protocol_proxy_port 的解析。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROTOCOL_PROXY_PORT_FILE);
        persist_protocol_proxy_port(&path, 57555).unwrap();
        assert_eq!(read_persisted_protocol_proxy_port(&path), Some(57555));
    }
}

#[cfg(test)]
mod exec_retry_after_tests {
    use super::*;
    use reqwest::header::{HeaderMap, RETRY_AFTER};

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, value.parse().unwrap());
        headers
    }

    #[test]
    fn delta_seconds_are_preserved() {
        assert_eq!(
            exec_retry_after_duration(&headers(" 90 "), SystemTime::UNIX_EPOCH),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            exec_retry_after_duration(&headers("0"), SystemTime::UNIX_EPOCH),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn future_http_date_uses_remaining_delay() {
        let retry_at = SystemTime::UNIX_EPOCH + Duration::from_secs(180);
        let now = SystemTime::UNIX_EPOCH + Duration::from_millis(90_250);
        assert_eq!(
            exec_retry_after_duration(&headers(&httpdate::fmt_http_date(retry_at)), now),
            Some(Duration::from_millis(89_750))
        );
    }

    #[test]
    fn past_http_date_is_immediate() {
        assert_eq!(
            exec_retry_after_duration(
                &headers("Thu, 01 Jan 1970 00:00:00 GMT"),
                SystemTime::UNIX_EPOCH + Duration::from_secs(1),
            ),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn missing_and_invalid_values_are_ignored() {
        assert_eq!(
            exec_retry_after_duration(&HeaderMap::new(), SystemTime::UNIX_EPOCH),
            None
        );
        for value in ["", "not-a-date", "-1", "1.5"] {
            assert_eq!(
                exec_retry_after_duration(&headers(value), SystemTime::UNIX_EPOCH),
                None,
                "{value}"
            );
        }
    }
}

#[cfg(test)]
mod unlisted_model_mapping_tests {
    use super::*;

    fn api_settings(model_list: &str, model: &str) -> crate::settings::BackendSettings {
        let profile = crate::settings::RelayProfile {
            id: "relay-api".to_string(),
            name: "API 中转".to_string(),
            model: model.to_string(),
            model_list: model_list.to_string(),
            relay_mode: crate::settings::RelayMode::PureApi,
            ..crate::settings::RelayProfile::default()
        };
        let mut settings = crate::settings::BackendSettings::default();
        settings.relay_profiles = vec![profile];
        settings.active_relay_id = "relay-api".to_string();
        // 开关默认关闭（opt-in），以下用例除专门测关闭的以外都显式开启。
        settings.protocol_proxy_map_unlisted_models = true;
        settings
    }

    #[test]
    fn mapping_is_off_by_default() {
        let mut settings = api_settings("minimax-m2.5\ndeepseek-v4-pro", "minimax-m2.5");
        settings.protocol_proxy_map_unlisted_models =
            crate::settings::BackendSettings::default().protocol_proxy_map_unlisted_models;
        assert!(!settings.protocol_proxy_map_unlisted_models);
        assert_eq!(unlisted_model_replacement(&settings, "gpt-5.4"), None);
        assert_eq!(unlisted_model_replacement(&settings, "codex-auto-review"), None);
    }

    #[test]
    fn configured_auto_review_model_wins_over_unlisted_mapping() {
        let mut settings = api_settings("minimax-m2.5\ndeepseek-v4-pro", "minimax-m2.5");
        settings.relay_profiles[0].auto_review_model = "deepseek-v4-flash".to_string();
        // codex-auto-review 让路给 upstream_request_parts 的审批模型改写（#1446）。
        assert_eq!(unlisted_model_replacement(&settings, "codex-auto-review"), None);
        assert_eq!(
            auto_review_model_rewrite(&settings.active_relay_profile(), "codex-auto-review"),
            Some("deepseek-v4-flash")
        );
        // 其他目录外模型仍按默认模型映射。
        assert_eq!(
            unlisted_model_replacement(&settings, "gpt-5.4").as_deref(),
            Some("minimax-m2.5")
        );
    }

    #[test]
    fn unlisted_background_models_map_to_profile_default() {
        let settings = api_settings("minimax-m2.5\ndeepseek-v4-pro", "minimax-m2.5");
        // Codex 后台任务（标题/摘要/子代理）与自动审查的固定模型名（#1183/#1446）。
        assert_eq!(
            unlisted_model_replacement(&settings, "gpt-5.4").as_deref(),
            Some("minimax-m2.5")
        );
        assert_eq!(
            unlisted_model_replacement(&settings, "codex-auto-review").as_deref(),
            Some("minimax-m2.5")
        );
    }

    #[test]
    fn listed_models_pass_through_unchanged() {
        let settings = api_settings("minimax-m2.5\ndeepseek-v4-pro", "minimax-m2.5");
        assert_eq!(unlisted_model_replacement(&settings, "minimax-m2.5"), None);
        assert_eq!(unlisted_model_replacement(&settings, "deepseek-v4-pro"), None);
        // 带 [1M] 窗口后缀的写法与列表里的同名模型等价。
        assert_eq!(unlisted_model_replacement(&settings, "deepseek-v4-pro[1M]"), None);
        // 用户显式把 gpt-* 配进列表时照原样转发，映射不能劫持。
        let with_gpt = api_settings("gpt-5.4\nminimax-m2.5", "minimax-m2.5");
        assert_eq!(unlisted_model_replacement(&with_gpt, "GPT-5.4"), None);
    }

    #[test]
    fn switch_off_disables_mapping() {
        let mut settings = api_settings("minimax-m2.5\ndeepseek-v4-pro", "minimax-m2.5");
        settings.protocol_proxy_map_unlisted_models = false;
        assert_eq!(unlisted_model_replacement(&settings, "gpt-5.4"), None);
    }

    #[test]
    fn official_login_without_mix_never_maps() {
        let mut settings = api_settings("minimax-m2.5", "minimax-m2.5");
        settings.relay_profiles[0].relay_mode = crate::settings::RelayMode::Official;
        settings.relay_profiles[0].official_mix_api_key = false;
        // 官方登录未混 API：公布的模型集为空（chatgpt.com 后端自带 gpt-*）。
        assert_eq!(unlisted_model_replacement(&settings, "gpt-5.4"), None);
    }

    #[test]
    fn aggregate_active_profile_skips_mapping() {
        let mut settings = api_settings("minimax-m2.5", "minimax-m2.5");
        settings.relay_profiles[0].relay_mode = crate::settings::RelayMode::Aggregate;
        settings.aggregate_relay_profiles = vec![crate::settings::AggregateRelayProfile {
            id: "relay-api".to_string(),
            name: "聚合".to_string(),
            session_provider: Default::default(),
            model_aggregation_enabled: false,
            strategy: Default::default(),
            members: Vec::new(),
            routes: Vec::new(),
            default_relay_id: String::new(),
            sync_default_member_login: false,
        }];
        // 聚合有自己的按成员归因与「不做静默改派」策略，不在此映射。
        assert_eq!(unlisted_model_replacement(&settings, "gpt-5.4"), None);
    }

    #[test]
    fn config_contents_model_takes_priority_as_replacement() {
        // relay_profile_model 优先读 config_contents 的 model 键（真正写进
        // config.toml 的值）；映射目标必须与它一致，而不是表单里的 model 字段。
        let mut settings = api_settings("minimax-m2.5", "form-model");
        settings.relay_profiles[0].config_contents =
            "model = \"deepseek-v4-pro\"\n".to_string();
        assert_eq!(
            unlisted_model_replacement(&settings, "gpt-5.4").as_deref(),
            Some("deepseek-v4-pro")
        );
    }

    #[test]
    fn official_mix_api_key_maps_by_profile_models() {
        // 官方登录但混入 API Key：公布的模型集是 profile 的 model_list（上游仍是
        // 第三方），后台固定模型名同样应改写到默认模型。
        let mut settings = api_settings("minimax-m2.5\ngpt-5.4", "minimax-m2.5");
        settings.relay_profiles[0].relay_mode = crate::settings::RelayMode::Official;
        settings.relay_profiles[0].official_mix_api_key = true;
        assert_eq!(unlisted_model_replacement(&settings, "codex-auto-review").as_deref(), Some("minimax-m2.5"));
        // 混入模式下列表里显式配置的 gpt-* 仍然原样转发。
        assert_eq!(unlisted_model_replacement(&settings, "gpt-5.4"), None);
    }

    #[test]
    fn empty_default_model_never_maps() {
        // 列表非空但默认模型为空时没有改写目标，保持原样转发。
        let settings = api_settings("minimax-m2.5\ndeepseek-v4-pro", "");
        assert_eq!(unlisted_model_replacement(&settings, "gpt-5.4"), None);
        // 空模型名的请求不处理。
        let settings = api_settings("minimax-m2.5", "minimax-m2.5");
        assert_eq!(unlisted_model_replacement(&settings, "  "), None);
    }
}

#[cfg(test)]
mod proxy_auto_retry_tests {
    use super::*;

    #[test]
    fn defaults_are_enabled_with_five_retries_and_two_second_backoff() {
        let state = ProxyAutoRetry::from_settings(&crate::settings::BackendSettings::default())
            .expect("默认开启自动重试");
        assert_eq!(state.remaining, 5);
        assert_eq!(state.next_delay, Duration::from_secs(2));
    }

    #[test]
    fn disabled_switch_or_zero_attempts_disables_retry() {
        let mut settings = crate::settings::BackendSettings::default();
        settings.protocol_proxy_auto_retry = false;
        assert!(ProxyAutoRetry::from_settings(&settings).is_none());
        settings.protocol_proxy_auto_retry = true;
        settings.protocol_proxy_auto_retry_max_attempts = 0;
        assert!(ProxyAutoRetry::from_settings(&settings).is_none());
    }

    #[test]
    fn schedule_uses_exponential_backoff_and_stops_at_attempt_limit() {
        let mut settings = crate::settings::BackendSettings::default();
        settings.protocol_proxy_auto_retry_interval_seconds = 1;
        settings.protocol_proxy_auto_retry_max_attempts = 3;
        let mut state = ProxyAutoRetry::from_settings(&settings).unwrap();
        assert_eq!(state.schedule(None), Some(Duration::from_secs(1)));
        assert_eq!(state.schedule(None), Some(Duration::from_secs(2)));
        assert_eq!(state.schedule(None), Some(Duration::from_secs(4)));
        assert_eq!(state.schedule(None), None);
    }

    #[test]
    fn schedule_prefers_retry_after_but_caps_single_wait() {
        let mut settings = crate::settings::BackendSettings::default();
        settings.protocol_proxy_auto_retry_max_attempts = 2;
        let mut state = ProxyAutoRetry::from_settings(&settings).unwrap();
        assert_eq!(
            state.schedule(Some(Duration::from_secs(120))),
            Some(PROXY_AUTO_RETRY_MAX_DELAY)
        );
        // Retry-After 不参与指数序列：下一次仍按设置的基数退避。
        assert_eq!(state.schedule(None), Some(Duration::from_secs(4)));
    }

    #[test]
    fn schedule_respects_total_wait_budget() {
        let mut settings = crate::settings::BackendSettings::default();
        settings.protocol_proxy_auto_retry_interval_seconds = 30;
        settings.protocol_proxy_auto_retry_max_attempts = 20;
        let mut state = ProxyAutoRetry::from_settings(&settings).unwrap();
        for _ in 0..4 {
            assert_eq!(state.schedule(None), Some(PROXY_AUTO_RETRY_MAX_DELAY));
        }
        assert_eq!(state.schedule(None), None);
    }

    #[test]
    fn pre_output_events_are_buffered_and_output_events_release() {
        for event in [
            "response.created",
            "response.in_progress",
            "response.queued",
            "response.metadata",
        ] {
            assert!(is_pre_output_stream_event(event), "{event}");
        }
        for event in [
            "response.output_item.added",
            "response.output_text.delta",
            "response.reasoning_summary_text.delta",
            "response.completed",
            "response.failed",
        ] {
            assert!(!is_pre_output_stream_event(event), "{event}");
        }
    }

    fn commit_test_block(event: &str, data: &str) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    #[test]
    fn stream_start_commit_boundary_holds_reasoning_and_tool_work() {
        // 不可见阶段（推理、工具参数、工具项生命周期、未知/非标准事件）都继续
        // 缓冲：此时丢弃重发不会让 Codex 看到重复内容。
        for block in [
            commit_test_block("response.created", "{\"type\":\"response.created\"}"),
            commit_test_block(
                "response.output_item.added",
                "{\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\"}}",
            ),
            commit_test_block(
                "response.function_call_arguments.delta",
                "{\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{\"}",
            ),
            commit_test_block(
                "response.output_item.added",
                "{\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\"}}",
            ),
            // 推理摘要 part 的 done 此前不在 retry-safe 白名单里；新分类按
            // 「未知事件继续按住」处理，不再提前结束重试窗口。
            commit_test_block(
                "response.reasoning_summary_part.done",
                "{\"type\":\"response.reasoning_summary_part.done\"}",
            ),
            commit_test_block(
                "response.output_item.done",
                "{\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\"}}",
            ),
            commit_test_block(
                "response.output_item.done",
                "{\"type\":\"response.output_item.done\",\"item\":{\"type\":\"reasoning\",\"id\":\"rs_1\"}}",
            ),
        ] {
            let event = sse_block_type(&block).expect("event type");
            assert_eq!(
                stream_start_event_kind(&block, &event, StreamCommitMode::VisibleGrace),
                StreamStartEventKind::Hold,
                "{event}"
            );
        }
    }

    #[test]
    fn stream_start_commit_boundary_releases_visible_output_and_terminal_events() {
        for block in [
            commit_test_block(
                "response.output_text.delta",
                "{\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}",
            ),
            commit_test_block(
                "response.refusal.delta",
                "{\"type\":\"response.refusal.delta\",\"delta\":\"no\"}",
            ),
            commit_test_block(
                "response.image_generation_call.partial_image",
                "{\"type\":\"response.image_generation_call.partial_image\",\"partial_image_b64\":\"AAAA\"}",
            ),
            // 不吐 delta、只在 done 里给完整文本的兼容上游也算可见输出。
            commit_test_block(
                "response.output_text.done",
                "{\"type\":\"response.output_text.done\",\"text\":\"hello\"}",
            ),
            commit_test_block(
                "response.refusal.done",
                "{\"type\":\"response.refusal.done\",\"refusal\":\"no\"}",
            ),
            commit_test_block(
                "response.audio.transcript.done",
                "{\"type\":\"response.audio.transcript.done\",\"transcript\":\"hi\"}",
            ),
        ] {
            let event = sse_block_type(&block).expect("event type");
            assert_eq!(
                stream_start_event_kind(&block, &event, StreamCommitMode::VisibleGrace),
                StreamStartEventKind::Visible,
                "{event}"
            );
        }
        for block in [
            commit_test_block("response.completed", "{\"type\":\"response.completed\"}"),
            commit_test_block("response.failed", "{\"type\":\"response.failed\"}"),
            commit_test_block("response.incomplete", "{\"type\":\"response.incomplete\"}"),
            commit_test_block("error", "{\"type\":\"error\"}"),
            // 非消息输出项完成 = 工具调用参数收齐，提交不会延迟 Codex 执行。
            commit_test_block(
                "response.output_item.done",
                "{\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\"}}",
            ),
            commit_test_block(
                "response.output_item.done",
                "{\"type\":\"response.output_item.done\",\"item\":{\"type\":\"web_search_call\",\"id\":\"ws_1\"}}",
            ),
        ] {
            let event = sse_block_type(&block).expect("event type");
            assert_eq!(
                stream_start_event_kind(&block, &event, StreamCommitMode::VisibleGrace),
                StreamStartEventKind::Terminal,
                "{event}"
            );
        }
    }

    #[test]
    fn commit_policy_reads_mode_and_grace_window_from_settings() {
        let defaults = StreamStartCommitPolicy::from_settings(
            &crate::settings::BackendSettings::default(),
        );
        assert_eq!(defaults.mode, StreamCommitMode::VisibleGrace);
        assert_eq!(defaults.first_visible_grace, Duration::from_millis(1000));

        let mut settings = crate::settings::BackendSettings::default();
        settings.protocol_proxy_auto_retry_commit_delay_ms = 0;
        assert!(
            StreamStartCommitPolicy::from_settings(&settings)
                .first_visible_grace
                .is_zero()
        );

        settings.protocol_proxy_auto_retry_mode = "streaming-legacy".to_string();
        let legacy = StreamStartCommitPolicy::from_settings(&settings);
        assert_eq!(legacy.mode, StreamCommitMode::Immediate);

        // 重试预算用尽时退回旧提交时机。
        assert_eq!(
            defaults.without_retry_budget().mode,
            StreamCommitMode::Immediate
        );
    }

    #[test]
    fn legacy_commit_mode_releases_on_first_unreplayable_event() {
        // 旧模式：工具调用参数与正文都在第一个事件立即提交。
        for block in [
            commit_test_block(
                "response.output_item.added",
                "{\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\"}}",
            ),
            commit_test_block(
                "response.output_item.added",
                "{\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\"}}",
            ),
            commit_test_block(
                "response.output_text.delta",
                "{\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}",
            ),
        ] {
            let event = sse_block_type(&block).expect("event type");
            assert_eq!(
                stream_start_event_kind(&block, &event, StreamCommitMode::Immediate),
                StreamStartEventKind::Terminal,
                "{event}"
            );
        }
        // 推理事件仍然可安全重放，旧模式也先按住。
        let reasoning_delta = commit_test_block(
            "response.reasoning_summary_text.delta",
            "{\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"thinking\"}",
        );
        assert_eq!(
            stream_start_event_kind(
                &reasoning_delta,
                "response.reasoning_summary_text.delta",
                StreamCommitMode::Immediate,
            ),
            StreamStartEventKind::Hold
        );
    }

    #[test]
    fn overload_error_event_is_rewritten_for_codex_native_retry() {
        let mut t = ResponsesSseTransformer::default();
        let out = t.push_bytes(b"event: error\ndata:{\"type\":\"error\",\"error\":{\"code\":\"server_is_overloaded\",\"message\":\"Our servers are currently overloaded. Please try again later.\"}}\n\n");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("event: response.failed"), "{text}");
        assert!(text.contains("\"code\":\"server_overloaded\""), "{text}");
        assert!(!text.contains("server_is_overloaded"), "{text}");
        assert!(t.finish().is_some());
    }

    #[test]
    fn reasoning_events_stay_safe_to_replay_until_real_output() {
        let reasoning_item = concat!(
            "event: response.output_item.added\n",
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\",\"id\":\"rs_1\"}}\n\n",
        );
        assert!(stream_event_is_retry_safe(reasoning_item, "response.output_item.added"));
        let message_item = concat!(
            "event: response.output_item.added\n",
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg_1\"}}\n\n",
        );
        assert!(!stream_event_is_retry_safe(message_item, "response.output_item.added"));
        let reasoning_delta = "event: response.reasoning_summary_text.delta\ndata: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"thinking\"}\n\n";
        assert!(stream_event_is_retry_safe(reasoning_delta, "response.reasoning_summary_text.delta"));
        let text_delta = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n";
        assert!(!stream_event_is_retry_safe(text_delta, "response.output_text.delta"));
        let reasoning_part = "event: response.content_part.added\ndata: {\"type\":\"response.content_part.added\",\"part\":{\"type\":\"reasoning_text\"}}\n\n";
        assert!(stream_event_is_retry_safe(reasoning_part, "response.content_part.added"));
    }

    #[test]
    fn sse_boundary_supports_lf_and_crlf() {
        let lf = b"event: x\ndata: {}\n\ndata: tail";
        assert_eq!(next_sse_boundary(lf, 0), Some(19));
        let crlf = b"event: x\r\ndata: {}\r\n\r\ndata: tail";
        assert_eq!(next_sse_boundary(crlf, 0), Some(22));
        assert_eq!(next_sse_boundary(b"partial", 0), None);
    }

    #[test]
    fn stream_failure_payload_reads_nested_response_error() {
        let block = "event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\",\"error\":{\"code\":\"server_overloaded\",\"message\":\"Selected model is at capacity. Please try a different model.\"}}}";
        let (code, message) = stream_failure_payload(block);
        assert_eq!(code.as_deref(), Some("server_overloaded"));
        assert!(message.unwrap_or_default().contains("at capacity"));
        assert!(stream_failure_is_retryable(
            code.as_deref(),
            Some("Selected model is at capacity. Please try a different model.")
        ));
    }

    #[test]
    fn stream_failure_payload_reads_top_level_error_shape() {
        let block = "data: {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"bad input\"}}";
        let (code, message) = stream_failure_payload(block);
        assert_eq!(code.as_deref(), Some("invalid_request_error"));
        assert_eq!(message.as_deref(), Some("bad input"));
        assert!(!stream_failure_is_retryable(code.as_deref(), message.as_deref()));
    }

    #[test]
    fn permanent_stream_failures_are_not_retried() {
        assert!(!stream_failure_is_retryable(
            Some("context_length_exceeded"),
            Some("Your input exceeds the context window")
        ));
        assert!(!stream_failure_is_retryable(None, None));
        assert!(!stream_failure_is_retryable(
            Some("invalid_request_error"),
            Some("capacity field is invalid")
        ));
        assert!(stream_failure_is_retryable(Some("server_overloaded"), None));
    }

    #[test]
    fn official_server_is_overloaded_error_is_retryable() {
        assert!(stream_failure_is_retryable(
            Some("server_is_overloaded"),
            Some("Our servers are currently overloaded. Please try again later.")
        ));
        assert!(stream_failure_is_retryable(
            None,
            Some("Our servers are currently overloaded. Please try again later.")
        ));
        assert!(!stream_failure_is_retryable(
            Some("rate_limit_exceeded"),
            Some("Rate limit reached for requests")
        ));
    }


}

#[cfg(test)]
mod compaction_probe_body_limit_tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    #[tokio::test]
    async fn oversized_content_length_is_rejected_before_body_read() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x"))
            .mount(&server)
            .await;

        let mut response = reqwest::get(server.uri()).await.unwrap();
        let error = read_limited_upstream_body(&mut response, 0)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("上游响应体超过"));
    }
}
#[derive(Default)]
pub(crate) struct ResponsesSseTransformer {
    buf: Vec<u8>,
    event_tail: [u8; 4],
    completed: bool,
    failed: bool,
    failure: Option<String>,
    /// 当前事件已超过缓冲上限，正在原样透传；值为从事件开头识别出的类型。
    passthrough_event: Option<String>,
    /// 请求把 `apply_patch` 声明成了 function 工具：此时 JSON 参数就是正确格式，不能解包。
    apply_patch_declared_as_function: bool,
    /// 从 `response.completed` 里抓到的 usage，供请求日志使用。
    usage: Option<crate::request_log::RequestUsage>,
    /// 终态失败事件里的 code/message 摘要。只进请求日志与诊断日志，
    /// 不拼进 failure 原因（原因会随 SSE 回给客户端，不能泄露上游 payload）。
    failure_detail: Option<String>,
}

/// 单个事件参与改写的最大缓冲字节数（4 MiB）。
///
/// 改写只针对 `apply_patch` 的 output_item.done，这类事件很小；超过上限的事件
/// （典型是高分辨率生图的 base64、在 response.completed 里回显的 output）一律
/// 原样透传，不能截断：过去在这里 fail closed 会让客户端收不到终态事件，
/// Codex 按断流重试，每次重新生图并重复计费。
const RESPONSES_SSE_MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;

impl ResponsesSseTransformer {
    /// 按原始请求声明的工具形态创建：只有 freeform（custom）的 `apply_patch`
    /// 才需要把被模型包成 JSON 的输入解包回补丁文本。
    pub(crate) fn for_request(request: Option<&Value>) -> Self {
        let apply_patch_declared_as_function = request
            .and_then(|request| request.get("tools"))
            .and_then(Value::as_array)
            .is_some_and(|tools| {
                tools.iter().any(|tool| {
                    tool.get("type").and_then(Value::as_str) == Some("function")
                        && (tool.get("name").and_then(Value::as_str) == Some("apply_patch")
                            || tool.pointer("/function/name").and_then(Value::as_str)
                                == Some("apply_patch"))
                })
            });
        Self { apply_patch_declared_as_function, ..Self::default() }
    }

    /// Push upstream bytes; returns bytes ready to write to the client.
    pub(crate) fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        // 失败是终态：不再累积、分帧或转发后续字节，否则被截断事件的后半段
        // 会被当成一个完整事件发给客户端。
        if self.failed {
            return Vec::new();
        }
        let mut out = Vec::new();
        for &byte in bytes {
            self.event_tail.rotate_left(1);
            self.event_tail[3] = byte;
            let boundary =
                self.event_tail.ends_with(b"\n\n") || self.event_tail == *b"\r\n\r\n";
            if self.passthrough_event.is_some() {
                out.push(byte);
                if boundary {
                    self.event_tail = [0; 4];
                    let event_type = self.passthrough_event.take().unwrap_or_default();
                    self.observe_event_type(&event_type);
                    if self.failed {
                        return out;
                    }
                }
                continue;
            }
            self.buf.push(byte);
            if boundary {
                let event: Vec<u8> = self.buf.drain(..).collect();
                self.event_tail = [0; 4];
                let processed = self.process_event(&event);
                out.extend_from_slice(&processed);
                if self.failed {
                    return out;
                }
            } else if self.buf.len() >= RESPONSES_SSE_MAX_EVENT_BYTES {
                // 超大事件：识别类型后把已缓冲部分连同后续字节原样转发到事件边界。
                self.passthrough_event = Some(sse_event_type_from_prefix(&self.buf));
                out.append(&mut self.buf);
            }
        }
        out
    }

    fn observe_event_type(&mut self, event_type: &str) {
        match event_type {
            "response.completed" => self.completed = true,
            "response.failed" => {
                self.fail("原生 Responses 上游返回 response.failed".to_string())
            }
            "response.incomplete" => {
                self.fail("原生 Responses 上游返回 response.incomplete".to_string())
            }
            "error" => self.fail("原生 Responses 上游返回 error 事件".to_string()),
            _ => {}
        }
    }

    /// 是否已进入失败终态。转发层应在每个 chunk 后检查，以便在上游保持连接时
    /// 也能立即结束当前流，而不是等到 EOF。
    pub(crate) fn has_failed(&self) -> bool {
        self.failed
    }

    /// 取出 `response.completed` 里捕获的 usage（请求日志用）。
    pub(crate) fn take_usage(&mut self) -> Option<crate::request_log::RequestUsage> {
        self.usage.take()
    }

    /// 取出终态失败的 code/message 摘要（请求日志/诊断日志用，不发给客户端）。
    pub(crate) fn take_failure_detail(&mut self) -> Option<String> {
        self.failure_detail.take()
    }

    /// Flush after stream EOF. Returns failure reason if any.
    pub(crate) fn finish(mut self) -> Option<String> {
        self.buf.clear();
        if let Some(failure) = self.failure.take() {
            return Some(failure);
        }
        if self.completed {
            None
        } else {
            Some("原生 Responses 上游在完成事件前结束了响应流".to_string())
        }
    }

    /// 锁定失败终态并丢弃未完成的缓冲残片。只记录第一个失败原因。
    fn fail(&mut self, reason: String) {
        self.failed = true;
        self.buf.clear();
        self.event_tail = [0; 4];
        if self.failure.is_none() {
            self.failure = Some(reason);
        }
    }

    fn process_event(&mut self, event_bytes: &[u8]) -> Vec<u8> {
        let event_str = match std::str::from_utf8(event_bytes) {
            Ok(s) => s,
            Err(_) => return event_bytes.to_vec(),
        };

        // 先提取 SSE event: 字段作为回退类型名（与 NativeResponsesSseObserver 一致）
        let sse_event_name = event_str
            .lines()
            .find_map(|line| strip_sse_field(line, "event"))
            .map(str::trim)
            .map(str::to_string);

        let data_json_str = match extract_sse_data(event_str) {
            Some(s) => s,
            None => return event_bytes.to_vec(),
        };
        let data_json: Value = match serde_json::from_str(&data_json_str) {
            Ok(v) => v,
            Err(_) => return event_bytes.to_vec(),
        };

        // data JSON 的 type 优先，回退到 SSE event: 字段
        let event_type_owned = data_json
            .get("type")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(sse_event_name);
        let event_type = event_type_owned.as_deref().unwrap_or("");
        
        self.observe_event_type(event_type);
        if event_type == "response.completed" {
            if let Some(usage) = crate::request_log::usage_from_response_json(&data_json) {
                self.usage = Some(usage);
            }
        }
        if matches!(event_type, "response.failed" | "response.incomplete" | "error") {
            let (code, message) = stream_failure_payload(event_str);
            let detail = stream_failure_detail(code.as_deref(), message.as_deref());
            if !detail.is_empty() && self.failure_detail.is_none() {
                self.failure_detail = Some(detail);
            }
            if stream_failure_is_retryable(code.as_deref(), message.as_deref())
                && let Some(normalized) = normalized_overload_failure_event(&data_json)
            {
                return normalized.into_bytes();
            }
        }

        if event_type != "response.output_item.done" {
            return event_bytes.to_vec();
        }

        let mut modified = data_json.clone();
        if !transform_apply_patch_item(&mut modified, self.apply_patch_declared_as_function) {
            return event_bytes.to_vec();
        }

        let new_data = match serde_json::to_string(&modified) {
            Ok(s) => s,
            Err(_) => return event_bytes.to_vec(),
        };
        replace_sse_data_line(event_str, &new_data).into_bytes()
    }
}
/// 从超大事件的开头识别类型：优先 `event:` 行，否则取 data JSON 里第一个 `"type"`。
/// OpenAI 的 Responses SSE 两者都放在事件最前面。
fn sse_event_type_from_prefix(prefix: &[u8]) -> String {
    let head = String::from_utf8_lossy(&prefix[..prefix.len().min(4096)]);
    if let Some(name) = head
        .lines()
        .find_map(|line| strip_sse_field(line, "event"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return name.to_string();
    }
    head.split_once("\"type\"")
        .and_then(|(_, rest)| rest.trim_start().strip_prefix(':'))
        .and_then(|rest| rest.trim_start().strip_prefix('"'))
        .and_then(|rest| rest.split_once('"'))
        .map(|(value, _)| value.to_string())
        .unwrap_or_default()
}

fn extract_sse_data(event_str: &str) -> Option<String> {
    let mut parts = Vec::new();
    for line in event_str.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            parts.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn replace_sse_data_line(event_str: &str, new_data_json: &str) -> String {
    let mut result = String::with_capacity(event_str.len() + new_data_json.len());
    let mut data_replaced = false;
    let mut prev_was_data = false;
    for line in event_str.lines() {
        if line.starts_with("data:") {
            if !data_replaced {
                result.push_str("data:");
                result.push_str(new_data_json);
                result.push('\n');
                data_replaced = true;
                prev_was_data = true;
            }
            // skip additional data: lines (already merged)
        } else {
            result.push_str(line);
            result.push('\n');
            prev_was_data = false;
        }
    }
    // Preserve original line ending style
    let ends_crlf = event_str.ends_with("\r\n\r\n");
    // lines() strips trailing \r\n, so we need to reconstruct the terminator.
    // After lines(), result ends with \n from the last line.
    if ends_crlf {
        // Convert trailing \n to \r\n and add extra \r\n
        if result.ends_with('\n') {
            result.pop();
            result.push_str("\r\n\r\n");
        }
    } else {
        // LF style: result already ends with \n, add one more
        if !result.ends_with("\n\n") {
            result.push('\n');
        }
    }
    let _ = prev_was_data;
    result
}

fn transform_apply_patch_item(json: &mut Value, declared_as_function: bool) -> bool {
    let item = match json.get_mut("item") {
        Some(i) => i,
        None => {
            return false;
        }
    };
    
    let item_type = item.get("type").and_then(|v| v.as_str());
    
    // apply_patch 可能是 function_call 或 custom_tool_call 类型
    if item_type != Some("function_call") && item_type != Some("custom_tool_call") {
        return false;
    }
    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
    // 只认精确的 `apply_patch`：子串匹配会命中 `mcp__gh__apply_patch_to_pr` 这类
    // MCP 工具，把它们的 JSON 参数改成非 JSON 文本，工具调用直接报废。
    if name != "apply_patch" {
        return false;
    }
    // function 形态的 apply_patch 要求 arguments 是 `{"input": ...}` JSON，原样保留。
    if declared_as_function && item_type == Some("function_call") {
        return false;
    }
    
    // 检查 arguments 和 input 字段（apply_patch 使用 input 而不是 arguments）
    let (field_name, arguments) = if let Some(a) = item.get("arguments").and_then(|v| v.as_str()) {
        ("arguments", a.to_string())
    } else if let Some(i) = item.get("input").and_then(|v| v.as_str()) {
        ("input", i.to_string())
    } else {
        return false;
    };
    
    if arguments.starts_with("*** Begin Patch") || arguments.starts_with("*** begin_patch") {
        return false;
    }
    
    let unwrapped = unwrap_apply_patch_arguments(&arguments);
    
    if unwrapped == arguments {
        return false;
    }
    
    if let Some(field) = item.get_mut(field_name) {
        *field = Value::String(unwrapped);
        true
    } else {
        false
    }
}

fn unwrap_apply_patch_arguments(args: &str) -> String {
    let mut current = args.to_string();
    for _ in 0..10 {
        if current.starts_with("*** Begin Patch") || current.starts_with("*** begin_patch") {
            break;
        }
        match serde_json::from_str::<Value>(&current) {
            Ok(value) => {
                if let Some(obj) = value.as_object() {
                    let mut found = false;
                    for key in &["input", "data", "raw_patch", "patch", "text", "content"] {
                        if let Some(val) = obj.get(*key) {
                            if let Some(s) = val.as_str() {
                                current = s.to_string();
                                found = true;
                                break;
                            }
                        }
                    }
                    if found {
                        continue;
                    }
                    let mut patch_found = false;
                    for (_, val) in obj {
                        if let Some(s) = val.as_str() {
                            if s.contains("*** Begin Patch") || s.contains("*** begin_patch") {
                                current = s.to_string();
                                patch_found = true;
                                break;
                            }
                        }
                    }
                    if patch_found {
                        continue;
                    }
                    break;
                } else if let Some(s) = value.as_str() {
                    current = s.to_string();
                } else {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    current
}

#[cfg(test)]
mod responses_sse_transformer_tests {
    use super::*;

    #[test]
    fn test_context_compaction_event() {
        // 模拟上下文压缩时的事件
        let compaction_event = r#"event: response.compaction.delta
data: {"type":"response.compaction.delta","delta":"..."}

"#;
        let mut transformer = ResponsesSseTransformer::default();
        let result = transformer.push_bytes(compaction_event.as_bytes());
        assert_eq!(result, compaction_event.as_bytes());
    }

    #[test]
    fn test_non_apply_patch_tool_call() {
        let event = r#"event: response.output_item.done
data: {"type":"response.output_item.done","item":{"type":"function_call","name":"web_search","arguments":"{\"query\":\"test\"}"}}

"#;
        let mut transformer = ResponsesSseTransformer::default();
        let result = transformer.push_bytes(event.as_bytes());
        assert_eq!(result, event.as_bytes());
    }

    #[test]
    fn test_malformed_json() {
        let event = r#"event: response.output_item.done
data: {"type":"response.output_item.done","item":{"type":"function_call","name":"apply_patch"

"#;
        let mut transformer = ResponsesSseTransformer::default();
        let result = transformer.push_bytes(event.as_bytes());
        assert_eq!(result, event.as_bytes());
    }

    #[test]
    fn test_raw_text_passthrough() {
        let input = "*** Begin Patch\n*** Add File: test.txt\n+hello\n*** End Patch";
        assert_eq!(unwrap_apply_patch_arguments(input), input);
    }

    #[test]
    fn test_json_input_wrapper() {
        let inner = "*** Begin Patch\n*** Add File: test.txt\n+hello\n*** End Patch";
        let wrapped = serde_json::to_string(&serde_json::json!({"input": inner})).unwrap();
        assert_eq!(unwrap_apply_patch_arguments(&wrapped), inner);
    }

    #[test]
    fn test_json_data_wrapper() {
        let inner = "*** Begin Patch\n+line1\n*** End Patch";
        let wrapped = serde_json::to_string(&serde_json::json!({"data": inner})).unwrap();
        assert_eq!(unwrap_apply_patch_arguments(&wrapped), inner);
    }

    #[test]
    fn test_double_nested_json() {
        let inner = "*** Begin Patch\n+ok\n*** End Patch";
        let level1 = serde_json::to_string(&serde_json::json!({"data": inner})).unwrap();
        let level2 = serde_json::to_string(&serde_json::json!({"input": level1})).unwrap();
        assert_eq!(unwrap_apply_patch_arguments(&level2), inner);
    }

    #[test]
    fn test_json_string_double_serialization() {
        let inner = "*** Begin Patch\n+test\n*** End Patch";
        let wrapped = serde_json::to_string(&serde_json::json!(inner)).unwrap();
        assert_eq!(unwrap_apply_patch_arguments(&wrapped), inner);
    }

    #[test]
    fn test_unknown_field_with_patch_marker() {
        let inner = "*** Begin Patch\n+x\n*** End Patch";
        let wrapped =
            serde_json::to_string(&serde_json::json!({"custom_field": inner})).unwrap();
        assert_eq!(unwrap_apply_patch_arguments(&wrapped), inner);
    }

    #[test]
    fn test_non_patch_json_passthrough() {
        let input = r#"{"name":"other","value":42}"#;
        assert_eq!(unwrap_apply_patch_arguments(input), input);
    }

    #[test]
    fn test_non_apply_patch_event_passthrough() {
        let mut t = ResponsesSseTransformer::default();
        let event = b"data:{\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n";
        let result = t.push_bytes(event);
        assert_eq!(result, event.to_vec());
    }

    #[test]
    fn test_apply_patch_done_unwrapped() {
        let mut t = ResponsesSseTransformer::default();
        let event = b"data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"name\":\"apply_patch\",\"arguments\":\"{\\\"input\\\":\\\"*** Begin Patch\\\\n+hello\\\\n*** End Patch\\\"}\"}}\n\n";
        let result = t.push_bytes(event);
        let s = String::from_utf8_lossy(&result);
        assert!(s.contains("*** Begin Patch"), "got: {}", s);
        assert!(!s.contains("\\\"input\\\""), "wrapper still present: {}", s);
    }

    #[test]
    fn test_correct_format_not_modified() {
        let mut t = ResponsesSseTransformer::default();
        let event = b"data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"name\":\"apply_patch\",\"arguments\":\"*** Begin Patch\\n+hello\\n*** End Patch\"}}\n\n";
        let result = t.push_bytes(event);
        assert_eq!(result, event.to_vec());
    }

    #[test]
    fn test_incomplete_event_buffered() {
        let mut t = ResponsesSseTransformer::default();
        let result = t.push_bytes(b"data:{\"type\":\"response.output_text.delta\"");
        assert!(result.is_empty());
        let result = t.push_bytes(b"}\n\n");
        assert!(!result.is_empty());
    }

    #[test]
    fn test_crlf_events() {
        let mut t = ResponsesSseTransformer::default();
        let event = b"data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"name\":\"apply_patch\",\"arguments\":\"{\\\"data\\\":\\\"*** Begin Patch\\\\n+ok\\\\n*** End Patch\\\"}\"}}\r\n\r\n";
        let result = t.push_bytes(event);
        let s = String::from_utf8_lossy(&result);
        assert!(s.contains("*** Begin Patch"), "got: {}", s);
    }

    #[test]
    fn test_completed_detection() {
        let mut t = ResponsesSseTransformer::default();
        t.push_bytes(b"data:{\"type\":\"response.completed\"}\n\n");
        assert!(t.finish().is_none());
    }

    #[test]
    fn test_failed_detection() {
        let mut t = ResponsesSseTransformer::default();
        t.push_bytes(b"data:{\"type\":\"response.failed\",\"error\":{\"message\":\"x\"}}\n\n");
        assert!(t.finish().is_some());
    }

    #[test]
    fn test_terminal_failure_stops_forwarding_until_eof() {
        let mut t = ResponsesSseTransformer::default();
        let failure = b"data:{\"type\":\"response.failed\"}\n\n";
        assert_eq!(t.push_bytes(failure), failure.to_vec());
        assert!(t.has_failed());
        assert!(t.push_bytes(b"data:{\"type\":\"response.completed\"}\n\n").is_empty());
        assert!(t.finish().is_some());
    }

    #[test]
    fn test_multiple_events_in_chunk() {
        let mut t = ResponsesSseTransformer::default();
        let events = b"data:{\"type\":\"response.output_text.delta\",\"delta\":\"a\"}\n\ndata:{\"type\":\"response.output_text.delta\",\"delta\":\"b\"}\n\n";
        let result = t.push_bytes(events);
        assert_eq!(result, events.to_vec());
    }

    #[test]
    fn test_event_line_preserved() {
        let mut t = ResponsesSseTransformer::default();
        let event = b"event: msg\ndata:{\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n";
        let result = t.push_bytes(event);
        let s = String::from_utf8_lossy(&result);
        assert!(s.contains("event: msg"), "event line lost: {}", s);
    }

    #[test]
    fn test_replace_sse_data_crlf() {
        let original = "data:{\"old\":true}\r\n\r\n";
        let result = replace_sse_data_line(original, "{\"new\":true}");
        assert!(result.contains("data:{\"new\":true}"));
        assert!(result.ends_with("\r\n\r\n"), "CRLF not preserved: {:?}", result);
    }

    #[test]
    fn test_replace_sse_data_lf() {
        let original = "data:{\"old\":true}\n\n";
        let result = replace_sse_data_line(original, "{\"new\":true}");
        assert!(result.contains("data:{\"new\":true}"));
        assert!(result.ends_with("\n\n"), "LF not preserved: {:?}", result);
    }

    #[test]
    fn test_replace_preserves_event_line() {
        let original = "event: test\ndata:{\"old\":1}\n\n";
        let result = replace_sse_data_line(original, "{\"new\":2}");
        assert!(result.starts_with("event: test\n"), "event line lost: {:?}", result);
        assert!(result.contains("data:{\"new\":2}"));
    }

    #[test]
    fn test_fragmented_byte_by_byte() {
        let mut t = ResponsesSseTransformer::default();
        let full = b"data:{\"type\":\"response.completed\"}\n\n";
        for b in full {
            t.push_bytes(&[*b]);
        }
        assert!(t.finish().is_none());
    }

    #[test]
    fn test_event_name_fallback_completed() {
        // data JSON 没有 type 字段，但 SSE event: 字段包含 response.completed
        let mut t = ResponsesSseTransformer::default();
        let event = b"event: response.completed\ndata:{\"response\":{}}\n\n";
        t.push_bytes(event);
        assert!(t.finish().is_none(), "should detect completed via event: fallback");
    }

    #[test]
    fn test_event_name_fallback_failed() {
        let mut t = ResponsesSseTransformer::default();
        let event = b"event: response.failed\ndata:{\"error\":{\"message\":\"x\"}}\n\n";
        t.push_bytes(event);
        assert!(t.finish().is_some(), "should detect failed via event: fallback");
    }

    #[test]
    fn completed_event_usage_is_captured_for_request_log() {
        let mut t = ResponsesSseTransformer::default();
        let event = b"data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":123,\"output_tokens\":45,\"input_tokens_details\":{\"cached_tokens\":100},\"output_tokens_details\":{\"reasoning_tokens\":9}}}}\n\n";
        t.push_bytes(event);
        let usage = t.take_usage().expect("completed 事件应捕获 usage");
        assert_eq!(usage.input_tokens, 123);
        assert_eq!(usage.output_tokens, 45);
        assert_eq!(usage.cached_tokens, 100);
        assert_eq!(usage.reasoning_tokens, 9);
        assert!(t.take_usage().is_none(), "usage 只取一次");
        assert!(t.finish().is_none());
    }

    #[test]
    fn error_event_failure_detail_is_kept_out_of_client_facing_reason() {
        let mut t = ResponsesSseTransformer::default();
        t.push_bytes(b"event: error\ndata:{\"type\":\"error\",\"error\":{\"code\":\"server_error\",\"message\":\"upstream exploded\"}}\n\n");
        let detail = t.take_failure_detail().expect("error event detail");
        assert!(detail.contains("server_error"), "{detail}");
        assert!(detail.contains("upstream exploded"), "{detail}");
        let failure = t.finish().expect("error event must fail");
        assert_eq!(failure, "原生 Responses 上游返回 error 事件");
    }

    #[test]
    fn test_event_name_data_type_takes_priority() {
        // data JSON 的 type 应优先于 SSE event: 字段
        let mut t = ResponsesSseTransformer::default();
        let event = b"event: something_else\ndata:{\"type\":\"response.completed\"}\n\n";
        t.push_bytes(event);
        assert!(t.finish().is_none(), "data type should take priority");
    }

    /// 构造一个超过缓冲上限的事件（模拟高分辨率生图的 base64）。
    fn oversized_event(event_type: &str, secret: &str) -> Vec<u8> {
        let mut event = format!("event: {event_type}\ndata:{{\"type\":\"{event_type}\",\"note\":\"{secret}\",\"image\":\"").into_bytes();
        event.extend_from_slice(&vec![b'A'; RESPONSES_SSE_MAX_EVENT_BYTES + 1024]);
        event.extend_from_slice(b"\"}\n\n");
        event
    }

    #[test]
    fn apply_patch_unwrap_skips_mcp_tools_and_function_declared_apply_patch() {
        let wrapped = |kind: &str, name: &str| {
            format!("data:{{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{{\"type\":\"{kind}\",\"name\":\"{name}\",\"arguments\":\"{{\\\"input\\\":\\\"*** Begin Patch\\\\n+hi\\\\n*** End Patch\\\"}}\"}}}}\n\n")
        };
        // MCP 工具名里带 apply_patch 也不能被改写。
        let mcp = wrapped("function_call", "mcp__gh__apply_patch_to_pr");
        let mut t = ResponsesSseTransformer::for_request(None);
        assert_eq!(t.push_bytes(mcp.as_bytes()), mcp.as_bytes());

        // 声明成 function 的 apply_patch：JSON 参数就是正确格式。
        let function_request = json!({"tools": [{"type": "function", "name": "apply_patch", "parameters": {}}]});
        let event = wrapped("function_call", "apply_patch");
        let mut t = ResponsesSseTransformer::for_request(Some(&function_request));
        assert_eq!(t.push_bytes(event.as_bytes()), event.as_bytes());

        // 声明成 custom（freeform）时照旧解包（PR #2483 的场景）。
        let custom_request = json!({"tools": [{"type": "custom", "name": "apply_patch"}]});
        let mut t = ResponsesSseTransformer::for_request(Some(&custom_request));
        let out = String::from_utf8(t.push_bytes(event.as_bytes())).unwrap();
        assert!(!out.contains("\\\"input\\\""), "{out}");
        assert!(out.contains("*** Begin Patch"), "{out}");
    }

    #[test]
    fn oversized_completed_event_is_forwarded_intact_and_completes_the_stream() {
        // 超大事件不能截断：过去 fail closed 会让客户端收不到 response.completed，
        // Codex 按断流重试，每次重新生图、重复计费。
        let mut t = ResponsesSseTransformer::default();
        let event = oversized_event("response.completed", "x");
        let mut out = Vec::new();
        for chunk in event.chunks(64 * 1024) {
            out.extend(t.push_bytes(chunk));
        }
        assert_eq!(out, event, "超大事件必须逐字节原样转发");
        assert!(!t.has_failed());
        assert!(t.finish().is_none(), "超大的 response.completed 仍要被识别为完成");
    }

    #[test]
    fn events_around_an_oversized_event_keep_their_order_and_transforms() {
        let mut t = ResponsesSseTransformer::default();
        let good = b"data:{\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n";
        let big = oversized_event("response.output_item.done", "img");
        let tool_event = b"data:{\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"custom_tool_call\",\"name\":\"apply_patch\",\"input\":\"{\\\"input\\\":\\\"*** Begin Patch\\\\n+hello\\\\n*** End Patch\\\"}\"}}\n\n";
        let completed = b"data:{\"type\":\"response.completed\"}\n\n";
        let mut payload = good.to_vec();
        payload.extend_from_slice(&big);
        let mut out = t.push_bytes(&payload);
        out.extend(t.push_bytes(tool_event));
        out.extend(t.push_bytes(completed));
        assert!(out.starts_with(good), "超大事件之前的事件照常转发");
        assert_eq!(&out[good.len()..good.len() + big.len()], &big[..], "超大事件原样转发");
        let tail = String::from_utf8_lossy(&out[good.len() + big.len()..]).to_string();
        assert!(tail.contains("*** Begin Patch\\n+hello"), "超大事件之后的 apply_patch 事件仍被改写：{tail}");
        assert!(tail.ends_with("data:{\"type\":\"response.completed\"}\n\n"));
        assert!(t.finish().is_none());
    }

    #[test]
    fn oversized_failure_event_is_reported_without_leaking_payload() {
        let mut t = ResponsesSseTransformer::default();
        let event = oversized_event("response.failed", "secret-upstream-content");
        let out = t.push_bytes(&event);
        assert_eq!(out, event, "失败事件本身也要交给客户端");
        assert!(t.has_failed());
        let reason = t.finish().expect("超大的 response.failed 要报失败");
        assert!(reason.contains("response.failed"), "{reason}");
        assert!(!reason.contains("secret-upstream-content"), "{reason}");
    }

    #[test]
    fn event_type_is_read_from_prefix_with_or_without_event_line() {
        assert_eq!(sse_event_type_from_prefix(b"event: response.completed\ndata:{\"type\":\"x\""), "response.completed");
        assert_eq!(sse_event_type_from_prefix(b"data: { \"type\" : \"error\", \"a\": 1"), "error");
        assert_eq!(sse_event_type_from_prefix(b"data:{\"image\":\"AAAA"), "");
    }

    #[test]
    fn test_large_event_under_limit_is_forwarded() {
        // 远大于旧 observer 的 64 KiB 诊断前缀、但低于上限的合法事件，
        // 必须原字节转发且仍能识别终止类型，不能被误判为超限。
        let mut t = ResponsesSseTransformer::default();
        let padding = "x".repeat(512 * 1024);
        let event = format!(
            "data:{{\"type\":\"response.completed\",\"response\":{{\"output\":\"{}\"}}}}\n\n",
            padding
        );
        assert!(
            event.len() < RESPONSES_SSE_MAX_EVENT_BYTES,
            "fixture must stay under the limit"
        );
        let out = t.push_bytes(event.as_bytes());
        assert_eq!(
            out,
            event.as_bytes(),
            "large but legal event must be forwarded byte-for-byte"
        );
        assert!(!t.has_failed());
        assert!(
            t.finish().is_none(),
            "completed must still be detected inside a large event"
        );
    }

    #[test]
    fn test_failed_after_completed_reports_failure_without_leaking_payload() {
        // failure 优先于 completed，且失败原因不得回显上游 payload。
        let mut t = ResponsesSseTransformer::default();
        t.push_bytes(b"data:{\"type\":\"response.completed\"}\n\n");
        t.push_bytes(b"data:{\"type\":\"response.failed\",\"error\":{\"message\":\"private-upstream-detail\"}}\n\n");
        let reason = t.finish().expect("failure must win over completed");
        assert!(reason.contains("response.failed"), "unexpected reason: {}", reason);
        assert!(
            !reason.contains("private-upstream-detail"),
            "failure reason must not leak upstream payload: {}",
            reason
        );
    }
}

#[cfg(test)]
mod parallel_tool_call_pairing_tests {
    use super::*;

    /// #2185：同一轮并行两个带图片的 view_image 与一个 exec 之后，出站请求里每个
    /// 工具输出都必须仍能找到对应调用；exec 适配器只改写 exec 那一对。
    #[tokio::test]
    async fn parallel_image_custom_tool_calls_keep_pairing_on_responses_path() {
        let relay = crate::settings::RelayProfile {
            protocol: RelayProtocol::Responses,
            base_url: "https://relay.example.test/v1".into(),
            api_key: "sk-test".into(),
            ..Default::default()
        };
        let image_output = json!([
            {"type": "input_text", "text": "img"},
            {"type": "input_image", "image_url": "data:image/png;base64,AAAA"}
        ]);
        let body = json!({"model": "gpt-5.6-sol", "input": [
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "look"}]},
            {"type": "custom_tool_call", "id": "ctc_a", "call_id": "call_a", "name": "view_image", "input": "a.png"},
            {"type": "custom_tool_call", "id": "ctc_b", "call_id": "call_b", "name": "view_image", "input": "b.png"},
            {"type": "custom_tool_call", "id": "ctc_c", "call_id": "call_c", "name": "exec", "input": "ls"},
            {"type": "custom_tool_call_output", "id": "ctco_a", "call_id": "call_a", "output": image_output.clone()},
            {"type": "custom_tool_call_output", "id": "ctco_b", "call_id": "call_b", "output": image_output},
            {"type": "custom_tool_call_output", "id": "ctco_c", "call_id": "call_c", "output": "ok"},
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "next"}]}
        ], "tools": [{"type": "custom", "name": "view_image"}, {"type": "custom", "name": "exec"}]});

        let (_, out, _, _) =
            upstream_request_parts(
                &relay,
                body,
                "/v1/responses",
                None,
                false,
                None,
                ProxySessionHeaders::default(),
            )
                .await
                .unwrap();
        assert_pairing(&out, 3);
        let adapted = crate::responses_exec::adapt_request(out).unwrap();
        assert_pairing(&adapted, 3);
        let input = adapted["input"].as_array().unwrap();
        assert_eq!(input[4]["type"], "custom_tool_call_output", "图片输出不应被改成 function_call_output");
        assert_eq!(input[6]["type"], "function_call_output");
    }

    /// issue #1446：配置了审批模型时把 codex-auto-review 改写成供应商模型；
    /// 未配置时原样转发，普通请求不受影响。
    #[tokio::test]
    async fn auto_review_model_is_rewritten_only_when_configured() {
        let mut relay = crate::settings::RelayProfile {
            protocol: RelayProtocol::Responses,
            base_url: "https://relay.example.test/v1".into(),
            api_key: "sk-test".into(),
            ..Default::default()
        };
        let review = || json!({"model": "codex-auto-review", "input": "review"});

        let (_, out, _, _) =
            upstream_request_parts(&relay, review(), "/v1/responses", None, false, None, ProxySessionHeaders::default())
                .await
                .unwrap();
        assert_eq!(out["model"], "codex-auto-review");

        relay.auto_review_model = " deepseek-v4-flash ".into();
        let (_, out, _, _) =
            upstream_request_parts(&relay, review(), "/v1/responses", None, false, None, ProxySessionHeaders::default())
                .await
                .unwrap();
        assert_eq!(out["model"], "deepseek-v4-flash");

        let normal = json!({"model": "deepseek-v4-pro", "input": "hi"});
        let (_, out, _, _) =
            upstream_request_parts(&relay, normal, "/v1/responses", None, false, None, ProxySessionHeaders::default())
                .await
                .unwrap();
        assert_eq!(out["model"], "deepseek-v4-pro");
    }

    fn assert_pairing(body: &Value, expected: usize) {
        let input = body["input"].as_array().unwrap();
        let ids = |suffix: &str| -> Vec<(String, String)> {
            input
                .iter()
                .filter_map(|item| {
                    let kind = item["type"].as_str()?;
                    kind.ends_with(suffix).then(|| {
                        (kind.to_string(), item["call_id"].as_str().unwrap_or("").to_string())
                    })
                })
                .collect()
        };
        let calls = ids("_call");
        let outputs = ids("_output");
        assert_eq!(calls.len(), expected, "{body}");
        assert_eq!(outputs.len(), expected, "{body}");
        for (kind, call_id) in &outputs {
            let call_kind = kind.trim_end_matches("_output");
            assert!(
                calls.iter().any(|(k, id)| id == call_id && k == call_kind),
                "{kind} {call_id} 找不到同类型调用：{body}"
            );
        }
    }
}
