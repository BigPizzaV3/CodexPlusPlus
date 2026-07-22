/// VLM (Vision Language Model) analysis for send-as-is models.
/// Batches images into groups, sends each batch as one API call.
/// Includes image-description cache, retry, concurrency limits,
/// round-depth control, dynamic context-window overflow protection,
/// and two-phase (sync + background) analysis with X-governed injection.
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::protocol_proxy::chat_completions_url;

const BATCH_SIZE: usize = 5;
/// 黄金窗口：Phase 1 同步补全的最近 N 轮 user 消息。
const GOLDEN_WINDOW_DEPTH: usize = 10;
/// Phase 2 后台可推进的最大深度（user 消息轮数）。
const ANALYZE_DEPTH_LIMIT: usize = 50;
/// 每条描述的平均 token 预算（~200 chars → estimate_tokens = 200/2 = 100）。
const AVG_DESC_BUDGET: u64 = 100;
/// per-batch 最大重试次数（共 3 次尝试）。
const BATCH_MAX_ATTEMPTS: u32 = 2;
/// 单图最大重试次数。
#[allow(dead_code)]
const SINGLE_MAX_ATTEMPTS: u32 = 1;
/// 上下文窗口安全余量（0.9 = 留 10% 给上游 tokenizer 差异）。
const CONTEXT_SAFETY_MARGIN: f64 = 0.9;
/// VLM 返回的错误文本截断长度。
const ERROR_BODY_TRUNCATE: usize = 256;
/// 单图描述最大字符数。
const DESC_MAX_CHARS: usize = 2000;

// ── Global state ──────────────────────────────────────────────────────

/// 缓存容量上限。
const CACHE_CAPACITY: usize = 500;
/// 缓存 TTL（24 小时）。
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

/// 缓存条目：(描述文本, 写入时间)。
type CacheEntry = (String, Instant);

/// 缓存键：Tier1 只看 URL（历史轮/无文字当前轮），Tier2 看 URL+问题（有文字当前轮）。
/// 用结构键而非 hash 值，HashMap 的 Eq 比较原始字符串，彻底消除碰撞误命中。
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
enum CacheKey {
    Url(String),
    UrlQuestion(String, String),
}

/// 图片描述缓存：key=CacheKey，value=(描述, 写入时间)。
static VLM_CACHE: LazyLock<Mutex<HashMap<CacheKey, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 全局 VLM 信号量，限制跨请求并发数。
static VL_SEMAPHORE: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(5));

fn url_hash(url: &str) -> CacheKey {
    CacheKey::Url(url.to_string())
}

fn url_question_hash(url: &str, question: &str) -> CacheKey {
    CacheKey::UrlQuestion(url.to_string(), question.to_string())
}

fn cache_get(key: &CacheKey) -> Option<String> {
    let mut cache = VLM_CACHE.lock().unwrap();
    if let Some((desc, written)) = cache.get(key).cloned() {
        if written.elapsed() < CACHE_TTL {
            return Some(desc);
        }
        cache.remove(key);
    }
    None
}

fn cache_put(key: CacheKey, desc: String) {
    let mut cache = VLM_CACHE.lock().unwrap();
    if cache.len() >= CACHE_CAPACITY {
        // clone oldest key 以便 remove（cache 已被 &mut 借用，不能直接拿 &key）
        if let Some((oldest, _)) = cache.iter().min_by_key(|(_, (_, t))| *t) {
            let oldest = oldest.clone();
            cache.remove(&oldest);
        }
    }
    cache.insert(key, (desc, Instant::now()));
}

fn cache_contains(key: &CacheKey) -> bool {
    let cache = VLM_CACHE.lock().unwrap();
    cache
        .get(key)
        .map(|(_, t)| t.elapsed() < CACHE_TTL)
        .unwrap_or(false)
}

#[doc(hidden)]
pub fn cache_clear_for_tests() {
    VLM_CACHE.lock().unwrap().clear();
}

// ── Configuration ─────────────────────────────────────────────────────

#[derive(Clone)]
pub struct VlmConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub protocol: crate::settings::RelayProtocol,
}

impl Default for VlmConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: String::new(),
            base_url: String::new(),
            protocol: crate::settings::RelayProtocol::ChatCompletions,
        }
    }
}

// ── Public helpers ────────────────────────────────────────────────────

/// 图片处理模式（per-model）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageHandling {
    /// 图片原样保留，不做任何处理（默认）。
    #[serde(rename = "send-as-is")]
    SendAsIs,
    /// 剥离图片块，替换为占位符，不调 VLM。
    #[serde(rename = "strip")]
    Strip,
    /// VLM 分析管线（两阶段：同步当前+黄金窗口，后台补深层）。
    #[serde(rename = "vlm")]
    Vlm,
}

/// 解析 model_vlm JSON，返回该模型的图片处理模式。
pub fn image_handling_mode(model: &str, model_vlm_json: &str) -> ImageHandling {
    if let Ok(map) =
        serde_json::from_str::<std::collections::BTreeMap<String, ImageHandling>>(model_vlm_json)
    {
        if let Some(mode) = map.get(model) {
            return *mode;
        }
    }
    ImageHandling::SendAsIs
}

/// 纯剥离模式：删除所有消息中的图片块，替换为 "[图片已省略]"。
/// 不调 VLM，不入缓存，不注入描述。
pub fn strip_images_only(messages: &mut [Value]) {
    for msg in messages.iter_mut() {
        let Some(content) = msg.get_mut("content") else {
            continue;
        };

        match &content {
            Value::Array(parts) => {
                let mut new_content: Vec<Value> = Vec::new();
                for part in parts.iter() {
                    let is_image = part
                        .get("type")
                        .and_then(Value::as_str)
                        .map_or(false, |t| t == "image_url" || t == "input_image");
                    if is_image {
                        new_content
                            .push(serde_json::json!({"type": "text", "text": "[图片已省略]"}));
                    } else {
                        new_content.push(part.clone());
                    }
                }
                *content = Value::Array(new_content);
            }
            Value::String(s) => {
                // 字符串 content 场景不会有图片，跳过
                let _ = s;
            }
            _ => {}
        }
    }
}

/// 纯剥离模式：删除所有消息中的图片块，替换为 "[图片已省略]"，返回剥离数。
/// 不调 VLM，不入缓存，不注入描述。
pub fn strip_images_only_counted(messages: &mut [Value]) -> usize {
    let mut total = 0;
    for msg in messages.iter_mut() {
        let Some(content) = msg.get_mut("content") else {
            continue;
        };

        match &content {
            Value::Array(parts) => {
                let mut new_content: Vec<Value> = Vec::new();
                for part in parts.iter() {
                    let is_image = part
                        .get("type")
                        .and_then(Value::as_str)
                        .map_or(false, |t| t == "image_url" || t == "input_image");
                    if is_image {
                        new_content
                            .push(serde_json::json!({"type": "text", "text": "[图片已省略]"}));
                        total += 1;
                    } else {
                        new_content.push(part.clone());
                    }
                }
                *content = Value::Array(new_content);
            }
            Value::String(_) => {}
            _ => {}
        }
    }
    total
}

/// 向 messages 的最后一条 user 消息注入"追问强化提示"。
///
/// 当用户对历史图片进行纯文本追问时，告诉模型：
/// 1) 优先从已注入的描述回答；2) 若追问细节描述未覆盖，必须告知用户重发图片+问题。
/// 防止模型在缺乏信息时编造图片内容。
fn inject_followup_note(messages: &mut [Value], n_history_images: usize) {
    let note = format!(
        "[系统：用户之前发送了 {n_history_images} 张图片，描述已在上面（# 图片内容描述）。\
         请优先从这些描述回答。\n若用户追问的细节描述中没有明确覆盖，\
         必须如实告知用户「需要重新查看原始图片，请重新发送图片并附上问题」。\
         绝对不要猜测或编造图片中未描述的细节。]"
    );
    if let Some(msg) = messages
        .iter_mut()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
    {
        if let Some(parts) = msg.get_mut("content").and_then(Value::as_array_mut) {
            parts.insert(0, serde_json::json!({"type": "text", "text": note}));
        }
    }
}

/// 检查窗口内除当前轮消息外，是否有其他 user 消息包含图片。
/// 用于 F4 追问检测：纯文本追问 + 窗口内历史有图 → 注入追问强化提示。
fn window_has_history_image(messages: &[Value], current_idx: Option<usize>) -> bool {
    messages.iter().enumerate().any(|(i, m)| {
        Some(i) != current_idx
            && m.get("role").and_then(Value::as_str) == Some("user")
            && m.get("content")
                .and_then(Value::as_array)
                .map(|ps| {
                    ps.iter().any(|p| {
                        matches!(
                            p.get("type").and_then(Value::as_str),
                            Some("input_image") | Some("image_url")
                        )
                    })
                })
                .unwrap_or(false)
    })
}

/// 向消息数组注入"看不到图片"的系统提示。
///
/// 自动检测数组格式：
/// - `messages` 格式（首条有 `role` 键）：插入 `{role:"system",content:[{type:"text",text:note}]}`
/// - `input` 格式（首条有 `type:"message"`）：插入 `{type:"message",role:"system",content:[{type:"input_text",text:note}]}`
pub fn inject_cannot_see_note_slice(arr: &mut Vec<Value>, n: usize, reason: &str) {
    if n == 0 {
        return;
    }
    let note = format!(
        "[系统：用户发送了 {n} 张图片，但{reason}，图片内容未被处理。你无法看到这些图片。\
         请如实告知用户当前状况（图片已剥离 / 路由中转失败 / 当前模式），\
         并建议：① 换用支持多模态的模型；或 ② 在 Codex++ 中为该纯文本模型配置视觉模型路由。\
         不要猜测或编造图片内容。]"
    );

    let is_input_format = arr
        .first()
        .and_then(|item| item.get("type").and_then(Value::as_str))
        .map_or(false, |t| t == "message");

    if is_input_format {
        arr.insert(
            0,
            serde_json::json!({
                "type": "message",
                "role": "system",
                "content": [{"type": "input_text", "text": note}]
            }),
        );
    } else {
        arr.insert(
            0,
            serde_json::json!({
                "role": "system",
                "content": [{"type": "text", "text": note}]
            }),
        );
    }
}

/// 注入"看不到图片"提示到 messages 的最后一条 user 消息中。
fn inject_cannot_see_note(messages: &mut [Value], n: usize, reason: &str) {
    if n == 0 {
        return;
    }
    let note = format!(
        "[系统：用户发送了 {n} 张图片，但{reason}，图片内容未被处理。你无法看到这些图片。\
         请如实告知用户当前状况（图片已剥离 / 路由中转失败 / 当前模式），\
         并建议：① 换用支持多模态的模型；或 ② 在 Codex++ 中为该纯文本模型配置视觉模型路由。\
         不要猜测或编造图片内容。]"
    );
    if let Some(msg) = messages
        .iter_mut()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
    {
        if let Some(parts) = msg.get_mut("content").and_then(Value::as_array_mut) {
            parts.insert(0, serde_json::json!({"type": "text", "text": note}));
        }
    }
}

/// 统计 messages 中所有图片块的数量。
fn count_images(messages: &[Value]) -> usize {
    messages
        .iter()
        .map(|m| {
            m.get("content")
                .and_then(Value::as_array)
                .map(|ps| {
                    ps.iter()
                        .filter(|p| {
                            matches!(
                                p.get("type").and_then(Value::as_str),
                                Some("input_image") | Some("image_url")
                            )
                        })
                        .count()
                })
                .unwrap_or(0)
        })
        .sum()
}

/// 删除所有消息中的全部 image 块，返回剥离数量。
fn strip_all_images_counted(messages: &mut [Value]) -> usize {
    let mut n = 0;
    for msg in messages.iter_mut() {
        if let Some(parts) = msg.get_mut("content").and_then(Value::as_array_mut) {
            let before = parts.len();
            parts.retain(|p| {
                !matches!(
                    p.get("type").and_then(Value::as_str),
                    Some("input_image") | Some("image_url")
                )
            });
            n += before - parts.len();
        }
    }
    n
}

/// 删除单条消息中的全部 image 块，返回剥离数量。
fn strip_images_in_message(msg: &mut Value) -> usize {
    if let Some(parts) = msg.get_mut("content").and_then(Value::as_array_mut) {
        let before = parts.len();
        parts.retain(|p| {
            !matches!(
                p.get("type").and_then(Value::as_str),
                Some("input_image") | Some("image_url")
            )
        });
        before - parts.len()
    } else {
        0
    }
}

// ── URL collection ────────────────────────────────────────────────────

/// 收集单条消息中的全部图片 URL（不修改消息）。
fn collect_urls(msg: &Value) -> Vec<String> {
    let mut urls = Vec::new();
    let Some(content) = msg.get("content") else {
        return urls;
    };
    let Some(parts) = content.as_array() else {
        return urls;
    };
    for part in parts {
        let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
        if (kind == "image_url" || kind == "input_image")
            && let Some(url) = part
                .pointer("/image_url/url")
                .or_else(|| part.pointer("/image_url"))
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty())
        {
            urls.push(url.to_string());
        }
    }
    urls
}

/// 收集最近 `depth_limit` 轮对话（所有 user 消息，无论是否带图）中的带图消息（最新优先），
/// 返回 `(message_index, Vec<url>)`。
fn collect_recent_image_messages(
    messages: &[Value],
    depth_limit: usize,
) -> Vec<(usize, Vec<String>)> {
    // 1. 取最近 depth_limit 条 user 消息（全部，不限是否带图）
    let user_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.get("role").and_then(Value::as_str) == Some("user"))
        .map(|(i, _)| i)
        .rev()
        .take(depth_limit)
        .collect();
    // 2. 在其中找出带图消息（已按最新优先排序）
    user_indices
        .into_iter()
        .map(|i| (i, collect_urls(&messages[i])))
        .filter(|(_, urls)| !urls.is_empty())
        .collect()
}

// ── Image stripping ───────────────────────────────────────────────────

/// 删除所有消息中的全部 image 块。
fn strip_all_images(messages: &mut [Value]) {
    for msg in messages.iter_mut() {
        let Some(content) = msg.get_mut("content") else {
            continue;
        };
        let Some(parts) = content.as_array_mut() else {
            continue;
        };
        let mut i = 0;
        while i < parts.len() {
            let kind = parts[i].get("type").and_then(Value::as_str).unwrap_or("");
            if kind == "image_url" || kind == "input_image" {
                parts.remove(i);
            } else {
                i += 1;
            }
        }
    }
}

// ── Context window ────────────────────────────────────────────────────

/// 从 relay 配置解析模型上下文窗口上限（token 数）。
/// 三级 fallback：model_windows JSON → context_window 全局 → 272_000 硬兜底。
fn resolve_context_window(
    model_windows_json: &str,
    context_window_str: &str,
    request_model: &str,
) -> u64 {
    let model_name = request_model.rsplit('/').next().unwrap_or(request_model);
    if let Ok(map) =
        serde_json::from_str::<std::collections::HashMap<String, String>>(model_windows_json)
    {
        if let Some(token) = map.get(model_name) {
            if let Some(w) = crate::model_suffix::parse_window_token(token) {
                return w;
            }
        }
    }
    if let Ok(w) = context_window_str.parse::<u64>() {
        if w > 0 {
            return w;
        }
    }
    272_000
}

/// bytes/2 粗估 token 数。主流 tokenizer 对中英文混合内容的压缩比约 1.5-2 bytes/token，
/// 用 bytes/2 确保估计值偏向保守，避免注入描述后实际 token 数超出模型窗口。
fn estimate_tokens(messages: &[Value]) -> usize {
    serde_json::to_string(messages).unwrap_or_default().len() / 2
}

// ── Prompts & helpers ─────────────────────────────────────────────────

const TIER1_PROMPT: &str =
    "请详细描述这张图片，重点涵盖：文字（如包含则逐字提取）、UI 元素、错误信息、布局结构。";

fn tier2_prompt(question: &str) -> String {
    format!("{TIER1_PROMPT}\n用户当前问题：{question}\n在全面描述基础上，对与上述问题相关的内容做更详细说明。")
}

fn truncate_char_safe(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// 从 messages 收集用户原文：最新 user 消息有文字 → 取；无文字 → 回溯最近一条有文字的 user 消息。
fn collect_input_text(messages: &[Value]) -> String {
    for item in messages.iter().rev() {
        if item.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        if let Some(parts) = item.get("content").and_then(Value::as_array) {
            for part in parts {
                if matches!(
                    part.get("type").and_then(Value::as_str),
                    Some("input_text") | Some("text")
                ) {
                    if let Some(t) = part.get("text").and_then(Value::as_str) {
                        return t.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

fn latest_user_message_has_image(messages: &[Value]) -> bool {
    messages
        .iter()
        .rev()
        .find(|it| it.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|it| it.get("content").and_then(Value::as_array))
        .map(|parts| {
            parts.iter().any(|p| {
                matches!(
                    p.get("type").and_then(Value::as_str),
                    Some("input_image") | Some("image_url")
                )
            })
        })
        .unwrap_or(false)
}

// ── VLM API call ──────────────────────────────────────────────────────

fn per_batch_timeout(n: usize) -> Duration {
    Duration::from_secs(35 + 10 * n as u64)
}

fn backoff_delay(attempt: u32, salt: &str) -> Duration {
    let base = 3.0 * 2u32.pow(attempt.saturating_sub(1)) as f64;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    attempt.hash(&mut h);
    salt.hash(&mut h);
    let jitter = (h.finish() % 20) as f64 / 100.0;
    Duration::from_secs_f64(base * (0.8 + jitter))
}

/// 多图时追加"按顺序描述 + [[图片K]] 标记"提示；单图直接返回原 prompt。
fn batch_prompt(prompt: &str, n: usize) -> String {
    if n > 1 {
        format!(
            "{prompt}\n请按顺序描述以下{n}张图片，每张图片的描述以 [[图片K]] 开头（K=1..{n}），每张单独描述。"
        )
    } else {
        prompt.to_string()
    }
}

fn build_vl_batch_body(urls: &[String], prompt: &str, config: &VlmConfig) -> Value {
    let final_prompt = batch_prompt(prompt, urls.len());
    let mut content = vec![serde_json::json!({"type":"text","text":final_prompt})];
    for u in urls {
        content.push(serde_json::json!({"type":"image_url","image_url":{"url":u}}));
    }
    serde_json::json!({"model": config.model, "messages":[{"role":"user","content":content}]})
}

/// Responses API 格式请求体：input + input_text/input_image parts。
fn build_vl_batch_body_responses(urls: &[String], prompt: &str, config: &VlmConfig) -> Value {
    let final_prompt = batch_prompt(prompt, urls.len());
    let mut content = vec![serde_json::json!({"type":"input_text","text":final_prompt})];
    for u in urls {
        content.push(serde_json::json!({"type":"input_image","image_url":{"url":u}}));
    }
    serde_json::json!({"model": config.model, "input":[{"role":"user","content":content}]})
}

fn extract_vl_text(response_body: &Value) -> Option<String> {
    response_body["choices"][0]["message"]["content"]
        .as_str()
        .map(String::from)
}

/// Responses 响应：output[].content[] 中 type==output_text 的 text 拼接。
fn extract_vl_text_responses(response_body: &Value) -> Option<String> {
    let output = response_body.get("output")?.as_array()?;
    let mut texts: Vec<String> = Vec::new();
    for item in output {
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for part in content {
            if part.get("type").and_then(Value::as_str) == Some("output_text") {
                if let Some(t) = part.get("text").and_then(Value::as_str) {
                    texts.push(t.to_string());
                }
            }
        }
    }
    if texts.is_empty() {
        None
    } else {
        Some(texts.join(""))
    }
}

fn parse_batch_descriptions(text: &str, n: usize) -> Option<Vec<String>> {
    let mut result = Vec::with_capacity(n);
    for i in 1..=n {
        let marker = format!("[[图片{i}]]");
        let start = text.find(&marker)? + marker.len();
        let end = (i + 1..=n)
            .find_map(|j| {
                let m = format!("[[图片{j}]]");
                text[start..].find(&m).map(|p| start + p)
            })
            .unwrap_or(text.len());
        result.push(text[start..end].trim().to_string());
    }
    Some(result)
}

fn log_vl_call(
    config: &VlmConfig,
    n_urls: usize,
    started: Instant,
    status: &str,
    http_code: Option<u16>,
    error: Option<&str>,
) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.vl_call",
        serde_json::json!({
            "vlModel": config.model,
            "n_urls": n_urls,
            "duration_ms": started.elapsed().as_millis() as u64,
            "status": status,
            "http_code": http_code,
            "error": error,
        }),
    );
}

/// 单次 VLM 调用的结构化结果，供真实代理路径与测试命令共享。
#[derive(Debug, Clone, serde::Serialize)]
pub struct VlCallOutcome {
    pub status: String,
    pub http_code: Option<u16>,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub text: Option<String>,
}

/// 单次 VLM 调用核心：按 protocol 选 URL/构造/解析，返回结构化结果。
/// 真实代理路径（call_vlm_batch_once）与测试命令（test_vlm_once）共享，
/// vl_call 日志在此统一记录。
pub async fn call_vl_once_structured(
    urls: &[String],
    prompt: &str,
    config: &VlmConfig,
    client: &reqwest::Client,
) -> VlCallOutcome {
    use crate::settings::RelayProtocol;
    let _permit = VL_SEMAPHORE.acquire().await.expect("sem closed");
    let endpoint = match config.protocol {
        RelayProtocol::Responses => crate::protocol_proxy::responses_url(&config.base_url),
        RelayProtocol::ChatCompletions => chat_completions_url(&config.base_url),
    };
    let body = match config.protocol {
        RelayProtocol::Responses => build_vl_batch_body_responses(urls, prompt, config),
        RelayProtocol::ChatCompletions => build_vl_batch_body(urls, prompt, config),
    };
    let started = Instant::now();
    let n = urls.len();
    let response = match client
        .post(&endpoint)
        .bearer_auth(&config.api_key)
        .json(&body)
        .timeout(per_batch_timeout(n))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let s = if e.is_timeout() { "timeout" } else { "send_error" };
            log_vl_call(config, n, started, s, None, Some(&e.to_string()));
            return VlCallOutcome {
                status: s.to_string(),
                http_code: None,
                duration_ms: started.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
                text: None,
            };
        }
    };
    let http_code = response.status().as_u16();
    // 先按文本读 body，再判 HTTP 码：非 2xx 时无论 body 是否 JSON 都报 http_error（含 body 片段），
    // 避免上游返回 HTML 错误页/纯文本时被误判为 json_error 而掩盖真实状态码。
    let body_text = response.text().await.unwrap_or_default();
    let body_snippet: String = body_text.chars().take(ERROR_BODY_TRUNCATE).collect();
    if !(200..300).contains(&http_code) {
        let msg = format!("VL API {http_code}: {body_snippet}");
        log_vl_call(config, n, started, "http_error", Some(http_code), Some(&msg));
        return VlCallOutcome {
            status: "http_error".to_string(),
            http_code: Some(http_code),
            duration_ms: started.elapsed().as_millis() as u64,
            error: Some(msg),
            text: None,
        };
    }
    let response_body: Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("JSON parse failed: {e} | body: {body_snippet}");
            log_vl_call(config, n, started, "json_error", Some(http_code), Some(&msg));
            return VlCallOutcome {
                status: "json_error".to_string(),
                http_code: Some(http_code),
                duration_ms: started.elapsed().as_millis() as u64,
                error: Some(msg),
                text: None,
            };
        }
    };
    let text = match config.protocol {
        RelayProtocol::Responses => extract_vl_text_responses(&response_body),
        RelayProtocol::ChatCompletions => extract_vl_text(&response_body),
    };
    let text = match text {
        Some(t) => t,
        None => {
            log_vl_call(config, n, started, "no_text", Some(http_code), None);
            return VlCallOutcome {
                status: "no_text".to_string(),
                http_code: Some(http_code),
                duration_ms: started.elapsed().as_millis() as u64,
                error: Some("no content".to_string()),
                text: None,
            };
        }
    };
    if n <= 1 {
        log_vl_call(config, n, started, "ok", Some(http_code), None);
        return VlCallOutcome {
            status: "ok".to_string(),
            http_code: Some(http_code),
            duration_ms: started.elapsed().as_millis() as u64,
            error: None,
            text: Some(text),
        };
    }
    match parse_batch_descriptions(&text, n) {
        Some(_) => {
            log_vl_call(config, n, started, "ok", Some(http_code), None);
            VlCallOutcome {
                status: "ok".to_string(),
                http_code: Some(http_code),
                duration_ms: started.elapsed().as_millis() as u64,
                error: None,
                text: Some(text),
            }
        }
        None => {
            log_vl_call(config, n, started, "parse_error", Some(http_code), None);
            VlCallOutcome {
                status: "parse_error".to_string(),
                http_code: Some(http_code),
                duration_ms: started.elapsed().as_millis() as u64,
                error: Some("batch parse failed".to_string()),
                text: Some(text),
            }
        }
    }
}

async fn call_vlm_batch_once(
    urls: &[String],
    prompt: &str,
    config: &VlmConfig,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<String>> {
    let n = urls.len();
    let outcome = call_vl_once_structured(urls, prompt, config, client).await;
    match outcome.status.as_str() {
        "ok" => {
            if n <= 1 {
                Ok(vec![outcome.text.unwrap_or_default()])
            } else {
                parse_batch_descriptions(&outcome.text.unwrap_or_default(), n)
                    .ok_or_else(|| anyhow::anyhow!("batch parse failed"))
            }
        }
        _ => anyhow::bail!(outcome.error.unwrap_or_else(|| outcome.status.clone())),
    }
}

/// 测试入口：用 TIER1_PROMPT 对单张图片（data URL 或 http URL）调用 VLM，返回结构化结果。
pub async fn test_vlm_once(
    config: &VlmConfig,
    image_data_url: &str,
    client: &reqwest::Client,
) -> VlCallOutcome {
    let mut outcome =
        call_vl_once_structured(&[image_data_url.to_string()], TIER1_PROMPT, config, client).await;
    // 描述文本上限沿用 DESC_MAX_CHARS，防超长描述撑爆弹窗。
    if let Some(text) = outcome.text.take() {
        outcome.text = Some(truncate_char_safe(&text, DESC_MAX_CHARS));
    }
    outcome
}

async fn call_vlm_batch(
    urls: &[String],
    prompt: &str,
    config: &VlmConfig,
    client: &reqwest::Client,
    max_attempts: u32,
) -> anyhow::Result<Vec<String>> {
    if urls.is_empty() {
        return Ok(Vec::new());
    }
    let salt = urls.first().map(String::as_str).unwrap_or("");
    let mut all_descs: Vec<String> = Vec::with_capacity(urls.len());
    // 按 BATCH_SIZE 分批，每批独立重试，避免一次性发送过多图片触发 provider 限制。
    for chunk in urls.chunks(BATCH_SIZE) {
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..max_attempts {
            if attempt > 0 {
                tokio::time::sleep(backoff_delay(attempt, salt)).await;
            }
            match call_vlm_batch_once(chunk, prompt, config, client).await {
                Ok(v) => {
                    all_descs.extend(v);
                    last_err = None;
                    break;
                }
                Err(e) => last_err = Some(e),
            }
        }
        if let Some(e) = last_err {
            return Err(e);
        }
    }
    Ok(all_descs)
}

// ── Description injection ─────────────────────────────────────────────

/// 向指定 user 消息末尾注入分析文本。
fn inject_text_into_user_message(msg: &mut Value, text: &str) {
    match msg.get_mut("content") {
        Some(Value::Array(parts)) => {
            parts.push(serde_json::json!({"type": "text", "text": text}));
        }
        Some(Value::String(existing)) => {
            let old = existing.clone();
            *msg.get_mut("content").unwrap() = serde_json::json!([
                {"type": "text", "text": old},
                {"type": "text", "text": text},
            ]);
        }
        _ => {}
    }
}

/// 注入分析结果到**最后一条** user 消息（兼容旧接口，供 analyze_all 返回值注入）。
pub fn inject_analysis(messages: &mut [Value], result: &Result<String, String>) {
    let text = match result {
        Ok(c) => c.clone(),
        Err(_) => "用户发送了图片，但是 Router VLM 调用失败。请在回复中包含 \"Router VLM 调用失败，未能识别图片内容\""
            .to_string(),
    };
    for msg in messages.iter_mut().rev() {
        if msg.get("role").and_then(Value::as_str) == Some("user") {
            inject_text_into_user_message(msg, &text);
            break;
        }
    }
}

// ── Main entry: strip + analyze + inject with cache ───────────────────

/// 对 messages 做图片剥离、VLM 分析、描述注入（带缓存、并发控制、上下文溢出保护）。
///
/// # 参数
/// - `messages`: 需原地修改的消息数组
/// - `vlm_config`: VLM API 配置
/// - `model_windows_json`: relay.model_windows 的 JSON 字符串
/// - `context_window_str`: relay.context_window 的字符串
/// - `request_model`: 请求中的 model 字段值
/// - `client`: reqwest HTTP client（由调用方注入）
pub async fn strip_image_blocks(
    messages: &mut [Value],
    vlm_config: &VlmConfig,
    model_windows_json: &str,
    context_window_str: &str,
    request_model: &str,
    client: &reqwest::Client,
) {
    // 0. 上下文溢出保护：基于剥离图片后的纯文本预估，因为图片最终会被删掉。
    let context_window =
        resolve_context_window(model_windows_json, context_window_str, request_model);
    // 留 10% 安全余量给上游 tokenizer 差异。
    let effective_window = (context_window as f64 * CONTEXT_SAFETY_MARGIN) as u64;
    let current_tokens = {
        let mut stripped = messages.to_vec();
        strip_all_images(&mut stripped);
        estimate_tokens(&stripped)
    };
    let available = effective_window.saturating_sub(current_tokens as u64);
    // 1 token 安全余量，防止零宽窗口。
    if available <= 1 {
        // 上下文已满：剥离图片释放空间，注入"看不到图"提示，记录 vl_strip 事件。
        let n = strip_all_images_counted(messages);
        if n > 0 {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.vl_strip",
                json!({"reason": "overflow", "n": n}),
            );
            inject_cannot_see_note(messages, n, "上下文已满，图片未处理");
        }
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "vlm_context_overflow",
            json!({
                "context_window": context_window,
                "text_only_estimated_tokens": current_tokens,
                "skipped_images": n,
            }),
        );
        return;
    }

    // 1. 计算注入预算 X = available / AVG_DESC_BUDGET。
    let x_budget = (available / AVG_DESC_BUDGET) as usize;

    // 2. 收集 50 轮对话中的带图消息（最新优先）。
    let all_image_msgs = collect_recent_image_messages(messages, ANALYZE_DEPTH_LIMIT);
    if all_image_msgs.is_empty() {
        strip_all_images(messages);
        return;
    }

    // 3. 确定黄金窗口边界（最近 GOLDEN_WINDOW_DEPTH 轮 user 消息中最早一条的 index）。
    let golden_user_cutoff = {
        let user_indices: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.get("role").and_then(Value::as_str) == Some("user"))
            .map(|(i, _)| i)
            .rev()
            .take(GOLDEN_WINDOW_DEPTH)
            .collect();
        user_indices.last().copied().unwrap_or(0)
    };
    let golden_total: usize = all_image_msgs
        .iter()
        .filter(|(idx, _)| *idx >= golden_user_cutoff)
        .map(|(_, urls)| urls.len())
        .sum(); // N
    let deep_total: usize = all_image_msgs
        .iter()
        .filter(|(idx, _)| *idx < golden_user_cutoff)
        .map(|(_, urls)| urls.len())
        .sum(); // M

    // 4. 分离当前轮（最后一条 user 消息）。
    let current_round_msg_idx: Option<usize> = messages
        .iter()
        .rev()
        .position(|m| m.get("role").and_then(Value::as_str) == Some("user"))
        .map(|pos| messages.len() - 1 - pos);

    let user_text = collect_input_text(messages);

    // F4: 在剥离所有图片之前捕获追问状态（剥离后窗口内不再有图，无法判断）。
    // - is_followup: 纯文本追问 + 窗口内历史有图
    // - history_image_count: 窗口内历史图数（仅当 is_followup 为 true 时有意义）
    // - total_stripped_this_request: 累计本请求异常路径 strip 的图片数（fail-open/overflow），
    //   用于判断是否跳过追问提示（异常路径已注入"看不到图"提示，优先）。
    let mut total_stripped_this_request: usize = 0;
    let is_followup = !user_text.is_empty()
        && !latest_user_message_has_image(messages)
        && window_has_history_image(messages, current_round_msg_idx);
    let history_image_count: usize = if is_followup {
        count_images(messages)
    } else {
        0
    };

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "vlm_strip_entry",
        json!({
            "image_rounds": all_image_msgs.len(),
            "golden_total": golden_total,   // N
            "deep_total": deep_total,       // M
            "x_budget": x_budget,           // X
        }),
    );

    // 5. Phase 1 同步分析 + 注入。
    let mut descriptions: std::collections::BTreeMap<usize, String> =
        std::collections::BTreeMap::new();
    let mut historical_injected: usize; // 黄金窗口 + 深层缓存命中合计，≤ X（延后赋值）

    // ── 辅助函数：对一批 URL 调 VLM 并注入描述 ──
    async fn analyze_and_inject(
        round_urls: &[String],
        vlm_config: &VlmConfig,
        descriptions: &mut std::collections::BTreeMap<usize, String>,
        msg_idx: usize,
        prompt: &str,
        client: &reqwest::Client,
    ) -> Result<(), ()> {
        if round_urls.is_empty() {
            return Ok(());
        }
        match call_vlm_batch(round_urls, prompt, vlm_config, client, BATCH_MAX_ATTEMPTS).await {
            Ok(desc_vec) => {
                for (url, desc) in round_urls.iter().zip(desc_vec.iter()) {
                    let desc = truncate_char_safe(desc, DESC_MAX_CHARS);
                    cache_put(url_hash(url), desc.clone());
                    descriptions
                        .entry(msg_idx)
                        .or_default()
                        .push_str(&format!("\n[图片描述] {desc}"));
                }
                Ok(())
            }
            Err(_) => Err(()),
        }
    }

    // 5a. 当前轮：不限量 VLM 同步分析，不计入 X 预算。
    // 若有用户文字问题则使用 Tier2 (URL+问题) prompt/缓存键，否则退级 Tier1。
    for (_, (msg_idx, urls)) in all_image_msgs.iter().enumerate() {
        if Some(*msg_idx) != current_round_msg_idx {
            continue;
        }
        let tier2_prompt_str = tier2_prompt(&user_text);
        let use_tier2 = !user_text.is_empty();
        let mut round_urls: Vec<String> = Vec::new();
        for url in urls {
            let key = if use_tier2 {
                url_question_hash(url, &user_text)
            } else {
                url_hash(url)
            };
            if let Some(cached) = cache_get(&key) {
                descriptions
                    .entry(*msg_idx)
                    .or_default()
                    .push_str(&format!("\n[图片描述] {cached}"));
            } else {
                round_urls.push(url.clone());
            }
        }
        if round_urls.is_empty() {
            continue;
        }
        let prompt = if use_tier2 {
            tier2_prompt_str.as_str()
        } else {
            TIER1_PROMPT
        };
        match call_vlm_batch(&round_urls, prompt, vlm_config, client, BATCH_MAX_ATTEMPTS).await {
            Ok(desc_vec) => {
                for (url, desc) in round_urls.iter().zip(desc_vec.iter()) {
                    let desc = truncate_char_safe(desc, DESC_MAX_CHARS);
                    let key = if use_tier2 {
                        url_question_hash(url, &user_text)
                    } else {
                        url_hash(url)
                    };
                    cache_put(key, desc.clone());
                    descriptions
                        .entry(*msg_idx)
                        .or_default()
                        .push_str(&format!("\n[图片描述] {desc}"));
                }
            }
            Err(_) => {
                // fail-open：strip 当前轮图片 + 注入"看不到图"提示（不再 return 保留图片）。
                if let Some(idx) = current_round_msg_idx {
                    if idx < messages.len() {
                        let n = strip_images_in_message(&mut messages[idx]);
                        total_stripped_this_request += n;
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "protocol_proxy.vl_strip",
                            json!({"reason": "vl_failed", "n": n}),
                        );
                        inject_cannot_see_note(messages, n, "视觉模型调用失败");
                    }
                }
                // 继续执行后续阶段（历史轮仍可走缓存），不 return。
            }
        }
    }

    // 5b. 黄金窗口：Phase 1 同步处理，计入 X 预算。
    let cap = if x_budget <= 10 {
        golden_total.min(x_budget)
    } else {
        golden_total.min(GOLDEN_WINDOW_DEPTH)
    };
    let mut golden_injected: usize = 0;

    for (_, (msg_idx, urls)) in all_image_msgs.iter().enumerate() {
        if Some(*msg_idx) == current_round_msg_idx || *msg_idx < golden_user_cutoff {
            continue;
        }
        if golden_injected >= cap {
            break;
        }
        let mut round_urls: Vec<String> = Vec::new();
        for url in urls {
            if golden_injected >= cap {
                break;
            }
            let key = url_hash(url);
            if let Some(cached) = cache_get(&key) {
                descriptions
                    .entry(*msg_idx)
                    .or_default()
                    .push_str(&format!("\n[图片描述] {cached}"));
                golden_injected += 1;
            } else {
                round_urls.push(url.clone());
                golden_injected += 1;
            }
        }
        if round_urls.is_empty() {
            continue;
        }
        // 历史轮 VLM 失败 → 静默跳过，已计数的 golden_injected 不减（窗口已占用）。
        let _ = analyze_and_inject(
            &round_urls,
            vlm_config,
            &mut descriptions,
            *msg_idx,
            TIER1_PROMPT,
            client,
        )
        .await;
    }
    historical_injected = golden_injected;

    // 5c. 深层缓存命中注入（计入 X 预算余量）。
    // 从近到远填充：越靠近当前轮的消息相关性越高，优先注入。
    if historical_injected < x_budget {
        let mut remaining = x_budget - historical_injected;
        for (msg_idx, urls) in all_image_msgs.iter() {
            if Some(*msg_idx) == current_round_msg_idx || *msg_idx >= golden_user_cutoff {
                continue;
            }
            if remaining == 0 {
                break;
            }
            for url in urls {
                if remaining == 0 {
                    break;
                }
                let key = url_hash(url);
                if let Some(cached) = cache_get(&key) {
                    descriptions
                        .entry(*msg_idx)
                        .or_default()
                        .push_str(&format!("\n[图片描述] {cached}"));
                    remaining -= 1;
                    historical_injected += 1;
                }
            }
        }
    }

    // 6. Phase 2 后台准备：在 strip 之前收集未缓存的 URL 列表。
    // Phase 2 仅当 X > 10 时触发，分析 50 轮深度内未缓存的图片，写入缓存供后续请求使用。
    // NOTE: Phase 2 逻辑完整迁移留 Task 6；当前占位保留结构。
    let bg_config_opt: Option<(VlmConfig, Vec<String>)> = None;

    // 7. 截断注入以适配上下文窗口。
    // estimate_tokens = bytes/2，故 available × 2 = 可用字节预算。
    let available_chars = available.saturating_mul(2) as usize;
    let mut total_chars = 0usize;
    let mut truncated = false;
    for (_msg_idx, desc) in descriptions.iter_mut().rev() {
        let desc_chars = desc.chars().count();
        total_chars += desc_chars;
        if total_chars > available_chars {
            let keep = desc_chars.saturating_sub(total_chars - available_chars);
            *desc =
                desc.chars().take(keep.max(1)).collect::<String>() + "\n[历史图片描述已省略]";
            truncated = true;
            break;
        }
    }
    if truncated {
        let mut keys: Vec<usize> = descriptions.keys().copied().collect();
        keys.sort();
        let mut cum = 0usize;
        for k in keys.iter().rev() {
            cum += descriptions[k].chars().count();
            if cum > available_chars {
                descriptions.remove(k);
            }
        }
    }

    // 8. 删除所有 image 块。
    strip_all_images(messages);

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "vlm_strip_done",
        json!({
            "descriptions_injected": descriptions.len(),
            "historical_injected": historical_injected,
            "x_budget": x_budget,
        }),
    );

    // 9. 注入描述文本。
    for (msg_idx, desc) in &descriptions {
        if *msg_idx < messages.len() {
            inject_text_into_user_message(&mut messages[*msg_idx], desc);
        }
    }

    // F4: 纯文本追问 + 窗口内历史有图 → 注入追问强化提示（仅当本请求无异常 strip 时）。
    // 若 overflow/fail-open 已发生（total_stripped_this_request > 0），其"看不到图"提示已
    // 覆盖当前轮图片，注入追问提示会冲突/冗余，故跳过。
    if is_followup && total_stripped_this_request == 0 {
        inject_followup_note(messages, history_image_count);
    }

    // 10. Phase 2 后台：异步分析未缓存图片写入缓存（X > 10 时触发）。
    // Phase 2 完整迁移留 Task 6。
    let _ = bg_config_opt;
}

// ── Test wrapper ───────────────────────────────────────────────────────

#[doc(hidden)]
pub async fn strip_image_blocks_for_tests(
    messages: &mut [Value],
    config: &VlmConfig,
    model_windows_json: &str,
    context_window_str: &str,
    request_model: &str,
) {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    strip_image_blocks(
        messages,
        config,
        model_windows_json,
        context_window_str,
        request_model,
        &client,
    )
    .await;
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlm_config_default_is_chat_completions() {
        use crate::settings::RelayProtocol;
        let cfg = VlmConfig::default();
        assert_eq!(cfg.protocol, RelayProtocol::ChatCompletions);
        assert_eq!(cfg.api_key, "");
        assert_eq!(cfg.model, "");
        assert_eq!(cfg.base_url, "");
    }

    #[test]
    fn batch_prompt_single_image_is_plain_prompt() {
        assert_eq!(batch_prompt("PROMPT", 1), "PROMPT");
    }

    #[test]
    fn batch_prompt_multi_image_has_markers() {
        let p = batch_prompt("P", 3);
        assert!(p.contains("P\n请按顺序描述以下3张图片"));
        assert!(p.contains("[[图片K]]"));
    }

    #[test]
    fn build_vl_batch_body_responses_single_image() {
        let cfg = VlmConfig { model: "vlm-m".into(), ..Default::default() };
        let body = build_vl_batch_body_responses(
            &["https://e.example/i.png".into()],
            "PROMPT",
            &cfg,
        );
        assert_eq!(body["model"], "vlm-m");
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        let content = input[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[0]["text"], "PROMPT");
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[1]["image_url"]["url"], "https://e.example/i.png");
    }

    #[test]
    fn build_vl_batch_body_responses_multi_image_uses_batch_prompt() {
        let cfg = VlmConfig { model: "m".into(), ..Default::default() };
        let urls = vec!["u1".into(), "u2".into()];
        let body = build_vl_batch_body_responses(&urls, "P", &cfg);
        let text = body["input"][0]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("P\n请按顺序描述以下2张图片"));
        assert!(text.contains("[[图片K]]"));
        let content = body["input"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[2]["type"], "input_image");
    }

    #[test]
    fn extract_vl_text_responses_picks_output_text() {
        let body = serde_json::json!({
            "output": [{
                "type": "message",
                "content": [
                    {"type": "output_text", "text": "图片里是一只猫"},
                    {"type": "reasoning", "summary": []}
                ]
            }]
        });
        assert_eq!(extract_vl_text_responses(&body).as_deref(), Some("图片里是一只猫"));
    }

    #[test]
    fn extract_vl_text_responses_none_when_missing() {
        let body = serde_json::json!({"output": [{"content": [{"type": "reasoning"}]}]});
        assert_eq!(extract_vl_text_responses(&body), None);
    }

    // ── image_handling_mode ───────────────────────────────────────

    #[test]
    fn handling_mode_vlm_when_string_value() {
        assert_eq!(
            image_handling_mode("gpt-4", r#"{"gpt-4":"vlm"}"#),
            ImageHandling::Vlm
        );
    }

    #[test]
    fn handling_mode_strip_when_string_value() {
        assert_eq!(
            image_handling_mode("gpt-4", r#"{"gpt-4":"strip"}"#),
            ImageHandling::Strip
        );
    }

    #[test]
    fn handling_mode_defaults_to_send_as_is_when_model_not_in_json() {
        assert_eq!(
            image_handling_mode("claude-3", r#"{"gpt-4":"vlm"}"#),
            ImageHandling::SendAsIs
        );
    }

    #[test]
    fn handling_mode_defaults_to_send_as_is_for_empty_json() {
        assert_eq!(image_handling_mode("gpt-4", "{}"), ImageHandling::SendAsIs);
    }

    #[test]
    fn handling_mode_defaults_to_send_as_is_for_invalid_json() {
        assert_eq!(
            image_handling_mode("gpt-4", "not-json"),
            ImageHandling::SendAsIs
        );
    }

    #[test]
    fn handling_mode_defaults_to_send_as_is_for_empty_string() {
        assert_eq!(image_handling_mode("gpt-4", ""), ImageHandling::SendAsIs);
    }

    // ── strip_images_only ─────────────────────────────────────────

    #[test]
    fn strip_images_only_removes_image_url_block() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}},
            ]
        })];
        strip_images_only(&mut messages);
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "hello");
        assert_eq!(parts[1]["type"], "text");
        assert_eq!(parts[1]["text"], "[图片已省略]");
    }

    #[test]
    fn strip_images_only_removes_input_image_block() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "input_image", "image_url": "data:image/png;base64,abc"},
            ]
        })];
        strip_images_only(&mut messages);
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "[图片已省略]");
    }

    #[test]
    fn strip_images_only_does_not_affect_send_as_is_messages() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "text", "text": "world"},
            ]
        })];
        strip_images_only(&mut messages);
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "hello");
        assert_eq!(parts[1]["text"], "world");
    }

    #[test]
    fn collect_urls_extracts_image_url_from_chat_format() {
        let msg = serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image_url", "image_url": {"url": "https://example.com/img.png"}},
            ]
        });
        let urls = collect_urls(&msg);
        assert_eq!(urls, vec!["https://example.com/img.png"]);
    }

    #[test]
    fn collect_urls_handles_input_image_blocks() {
        let msg = serde_json::json!({
            "role": "user",
            "content": [
                {"type": "input_image", "image_url": {"url": "data:image/png;base64,abc"}},
                {"type": "text", "text": "desc"},
            ]
        });
        let urls = collect_urls(&msg);
        assert_eq!(urls, vec!["data:image/png;base64,abc"]);
    }

    #[test]
    fn collect_urls_returns_empty_when_no_images() {
        let msg = serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hello"}]
        });
        let urls = collect_urls(&msg);
        assert!(urls.is_empty());
    }

    #[test]
    fn strip_all_images_removes_all_image_blocks() {
        let mut messages = vec![
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "old image"},
                    {"type": "image_url", "image_url": {"url": "https://old.com/img.png"}},
                ]
            }),
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "new image"},
                    {"type": "image_url", "image_url": {"url": "https://new.com/img.png"}},
                ]
            }),
        ];
        strip_all_images(&mut messages);
        assert_eq!(messages[0]["content"].as_array().unwrap().len(), 1);
        assert_eq!(messages[0]["content"][0]["type"], "text");
        assert_eq!(messages[1]["content"].as_array().unwrap().len(), 1);
        assert_eq!(messages[1]["content"][0]["type"], "text");
    }

    #[test]
    fn inject_analysis_adds_text_to_last_user_message() {
        let mut messages = vec![
            serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "ok"}]}),
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hi"}]}),
        ];
        inject_analysis(&mut messages, &Ok("image description".to_string()));
        let parts = messages[1]["content"].as_array().unwrap();
        assert_eq!(parts.last().unwrap()["type"], "text");
        assert_eq!(parts.last().unwrap()["text"], "image description");
    }

    #[test]
    fn inject_analysis_adds_placeholder_on_error() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })];
        inject_analysis(&mut messages, &Err("failed".to_string()));
        let parts = messages[0]["content"].as_array().unwrap();
        let last = parts.last().unwrap();
        assert_eq!(last["type"], "text");
        assert!(last["text"].as_str().unwrap().contains("Router VLM"));
    }

    #[test]
    fn inject_analysis_handles_string_content_by_wrapping_in_array() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": "a plain string message"
        })];
        inject_analysis(&mut messages, &Ok("vlm result".to_string()));
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "a plain string message");
        assert_eq!(parts[1]["text"], "vlm result");
    }

    #[test]
    fn url_hash_produces_consistent_output() {
        let h1 = url_hash("https://example.com/img.png");
        let h2 = url_hash("https://example.com/img.png");
        assert_eq!(h1, h2);
    }

    #[test]
    fn url_hash_differs_for_different_urls() {
        let h1 = url_hash("https://a.com/1.png");
        let h2 = url_hash("https://a.com/2.png");
        assert_ne!(h1, h2);
    }

    #[test]
    fn url_question_hash_produces_consistent_output() {
        let h1 = url_question_hash("https://example.com/img.png", "Q1");
        let h2 = url_question_hash("https://example.com/img.png", "Q1");
        assert_eq!(h1, h2);
    }

    #[test]
    fn url_question_hash_differs_for_different_questions() {
        let h1 = url_question_hash("https://a.com/1.png", "Q1");
        let h2 = url_question_hash("https://a.com/1.png", "Q2");
        assert_ne!(h1, h2);
    }

    #[test]
    fn collect_recent_image_messages_respects_depth_limit() {
        let msgs: Vec<Value> = (0..5)
            .map(|i| {
                serde_json::json!({
                    "role": "user",
                    "content": [
                        {"type": "image_url", "image_url": {"url": format!("https://x.com/{i}.png")}},
                    ]
                })
            })
            .collect();
        let result = collect_recent_image_messages(&msgs, 2);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, 4); // newest
        assert_eq!(result[1].0, 3);
    }

    #[test]
    fn resolve_context_window_uses_model_windows_first() {
        let w = resolve_context_window(r#"{"gpt-4":"100000"}"#, "200000", "gpt-4");
        assert_eq!(w, 100000);
    }

    #[test]
    fn resolve_context_window_falls_back_to_global() {
        let w = resolve_context_window("{}", "200000", "gpt-4");
        assert_eq!(w, 200000);
    }

    #[test]
    fn resolve_context_window_falls_back_to_hard_default() {
        let w = resolve_context_window("{}", "0", "unknown");
        assert_eq!(w, 272_000);
    }

    #[test]
    fn resolve_context_window_strips_provider_prefix() {
        let w = resolve_context_window(r#"{"gpt-4":"100000"}"#, "200000", "openai/gpt-4");
        assert_eq!(w, 100000);
    }

    #[test]
    fn estimate_tokens_is_proportional_to_input_size() {
        let small: Vec<Value> = vec![serde_json::json!({"role":"user","content":"hi"})];
        let large: Vec<Value> = vec![
            serde_json::json!({"role":"user","content":"hi"}),
            serde_json::json!({"role":"assistant","content":"a very long response with lots of text"}),
            serde_json::json!({"role":"user","content":"another message"}),
        ];
        let s = estimate_tokens(&small);
        let l = estimate_tokens(&large);
        assert!(s > 0);
        assert!(l > s, "larger input should produce larger estimate");
    }

    // ── collect_recent_image_messages ─────────────────────────────

    #[test]
    fn collect_recent_image_messages_skips_assistant() {
        let msgs: Vec<Value> = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "image_url", "image_url": {"url": "https://a.com/img.png"}}]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "image_url", "image_url": {"url": "https://u.com/img.png"}}]
            }),
        ];
        let result = collect_recent_image_messages(&msgs, 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 1); // only user message
    }

    #[test]
    fn collect_recent_image_messages_skips_messages_without_images() {
        let msgs: Vec<Value> = vec![
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hi"}]}),
            serde_json::json!({"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://x.com/img.png"}}]}),
        ];
        let result = collect_recent_image_messages(&msgs, 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 1);
    }

    // ── cache ─────────────────────────────────────────────────────

    #[test]
    fn cache_put_and_get_roundtrip() {
        cache_clear_for_tests();
        let key = url_hash("https://example.com/cache-test.png");
        cache_put(key.clone(), "cached description".to_string());
        let got = cache_get(&key);
        assert_eq!(got, Some("cached description".to_string()));
    }

    #[test]
    fn cache_contains_returns_false_for_missing_key() {
        cache_clear_for_tests();
        let key = url_hash("https://example.com/missing.png");
        assert!(!cache_contains(&key));
    }

    #[test]
    fn cache_put_evicts_oldest_when_full() {
        // 填满缓存（500 条）后继续插入会触发驱逐。
        // NOTE：此测试依赖全局 VLM_CACHE，与其他并行测试共享状态。
        // 通过独立 key 前缀避免冲突，启动时清空缓存避免交叉干扰。
        cache_clear_for_tests();
        for i in 0..CACHE_CAPACITY {
            let key = url_hash(&format!("https://evict-test.example.com/{i:04x}.png"));
            cache_put(key, format!("desc-{i}"));
        }
        // 确认第 0 条仍在
        let key0 = url_hash("https://evict-test.example.com/0000.png");
        assert!(cache_contains(&key0));
        // 插入第 501 条 → 触发驱逐（删最旧的 1 条）
        let overflow_key = url_hash("https://evict-test.example.com/overflow.png");
        cache_put(overflow_key.clone(), "overflow-desc".to_string());
        // 最旧的应已被驱逐
        assert!(!cache_contains(&key0));
        // 新插入的存在
        assert!(cache_contains(&overflow_key));
    }


    // ── CacheKey 结构键测试 ─────────────────────────────────────

    #[test]
    fn cachekey_url_and_url_question_do_not_collide() {
        cache_clear_for_tests();
        // 同 URL，Tier1 key vs Tier2 key 应互不命中
        let tier1 = url_hash("https://example.com/img.png");
        let tier2 = url_question_hash("https://example.com/img.png", "问题A");
        cache_put(tier1, "tier1-desc".to_string());
        // Tier2 key 不应命中 Tier1 的描述
        assert_eq!(cache_get(&tier2), None);
        assert_eq!(cache_get(&url_hash("https://example.com/img.png")), Some("tier1-desc".to_string()));
    }

    #[test]
    fn cachekey_different_questions_do_not_collide() {
        cache_clear_for_tests();
        let k1 = url_question_hash("https://example.com/img.png", "问题A");
        let k2 = url_question_hash("https://example.com/img.png", "问题B");
        cache_put(k1, "desc-A".to_string());
        assert_eq!(cache_get(&k2), None);
    }

    #[test]
    fn cachekey_empty_question_uses_tier1() {
        cache_clear_for_tests();
        // user_text 为空时 use_tier2=false，走 url_hash (Tier1)
        let key = url_hash("https://example.com/solo.png");
        cache_put(key.clone(), "solo-desc".to_string());
        assert_eq!(cache_get(&key), Some("solo-desc".to_string()));
    }

    // ── helpers ───────────────────────────────────────────────────

    #[test]
    fn truncate_char_safe_truncates_correctly() {
        assert_eq!(truncate_char_safe("hello world", 5), "hello");
        assert_eq!(truncate_char_safe("你好世界", 2), "你好");
        assert_eq!(truncate_char_safe("short", 100), "short");
    }

    #[test]
    fn collect_input_text_gets_latest_user_text() {
        let messages = vec![
            serde_json::json!({"role": "system", "content": "system prompt"}),
            serde_json::json!({"role": "user", "content": [
                {"type": "text", "text": "Q1"},
                {"type": "image_url", "image_url": {"url": "https://x.com/img.png"}},
            ]}),
            serde_json::json!({"role": "assistant", "content": "A1"}),
            serde_json::json!({"role": "user", "content": [
                {"type": "text", "text": "Q2"},
            ]}),
        ];
        assert_eq!(collect_input_text(&messages), "Q2");
    }

    #[test]
    fn collect_input_text_empty_when_no_user_text() {
        let messages = vec![serde_json::json!({"role": "system", "content": "sys"})];
        assert_eq!(collect_input_text(&messages), "");
    }

    #[test]
    fn latest_user_message_has_image_detects_image() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "hi"},
                {"type": "image_url", "image_url": {"url": "https://x.com/img.png"}},
            ]
        })];
        assert!(latest_user_message_has_image(&messages));
    }

    #[test]
    fn latest_user_message_has_image_returns_false_for_text_only() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })];
        assert!(!latest_user_message_has_image(&messages));
    }

    // ── F4 followup helpers ──────────────────────────────────────

    #[test]
    fn window_has_history_image_true_when_history_has_image() {
        // 当前轮 = index 1（纯文本），历史 = index 0（带图）→ 窗口内有历史图。
        let messages = vec![
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "Q1"},
                    {"type": "image_url", "image_url": {"url": "https://x.com/a.png"}},
                ]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": "Q2"}]
            }),
        ];
        assert!(window_has_history_image(&messages, Some(1)));
    }

    #[test]
    fn window_has_history_image_false_when_no_history_images() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "Q1"}]}),
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "Q2"}]}),
        ];
        assert!(!window_has_history_image(&messages, Some(1)));
    }

    #[test]
    fn window_has_history_image_false_when_only_current_has_image() {
        // 只有当前轮有图，历史轮无图 → false（不应被算作"历史图"）。
        let messages = vec![
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "Q1"}]}),
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "Q2"},
                    {"type": "image_url", "image_url": {"url": "https://x.com/curr.png"}},
                ]
            }),
        ];
        assert!(!window_has_history_image(&messages, Some(1)));
    }

    #[test]
    fn window_has_history_image_handles_none_current_idx() {
        // current_idx = None 时，所有 user 消息都算"历史"。
        let messages = vec![
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "Q1"},
                    {"type": "image_url", "image_url": {"url": "https://x.com/a.png"}},
                ]
            }),
        ];
        assert!(window_has_history_image(&messages, None));
    }

    #[test]
    fn inject_followup_note_prepends_to_last_user_message() {
        let mut messages = vec![
            serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "ok"}]}),
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "followup question"},
                ]
            }),
        ];
        inject_followup_note(&mut messages, 3);
        let parts = messages[1]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        // 追问提示被插入到首位
        let first = &parts[0];
        assert_eq!(first["type"], "text");
        let text = first["text"].as_str().unwrap();
        assert!(text.contains("3 张图片"));
        assert!(text.contains("重新发送图片"));
        assert!(text.contains("优先从"));
        // 原 user 文本保留在第二位
        assert_eq!(parts[1]["text"], "followup question");
    }

    #[test]
    fn inject_followup_note_noop_when_no_user_message() {
        let mut messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "ok"}]
        })];
        inject_followup_note(&mut messages, 5);
        // 没有 user 消息，注入了 0 个文本块
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "ok");
    }

    // ── strip_image_blocks (tokio::test) ──────────────────────────

    #[tokio::test]
    async fn strip_image_blocks_all_cache_hits_no_vlm_call() {
        let img_url = "https://test.example.com/cached.png";
        let user_text = "看这张图";
        // 预填充缓存：当前轮使用 url_question_hash (Tier2) 键
        cache_put(
            url_hash(img_url),
            "缓存的图片描述".to_string(),
        );
        cache_put(
            url_question_hash(img_url, user_text),
            "缓存的图片描述".to_string(),
        );

        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": user_text},
                {"type": "image_url", "image_url": {"url": img_url}},
            ]
        })];

        let vlm_config = VlmConfig {
            api_key: String::new(),
            model: String::new(),
            base_url: String::new(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        strip_image_blocks(&mut messages, &vlm_config, "{}", "272000", "gpt-4", &client).await;

        // 图片已被删除
        let parts = messages[0]["content"].as_array().unwrap();
        let has_image = parts
            .iter()
            .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"));
        assert!(!has_image, "image should be stripped");

        // 缓存描述已注入
        let last_text = parts.last().unwrap()["text"].as_str().unwrap();
        assert!(
            last_text.contains("缓存的图片描述"),
            "cached description not found in: {last_text}"
        );
    }

    #[tokio::test]
    async fn strip_image_blocks_context_overflow_strips_images() {
        // 上下文已满（available <= 1）时，剥离图片释放空间，注入占位符告知模型图片被跳过。
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "hi"},
                {"type": "image_url", "image_url": {"url": "https://test.example.com/img.png"}},
            ]
        })];

        let vlm_config = VlmConfig {
            api_key: String::new(),
            model: String::new(),
            base_url: String::new(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        strip_image_blocks(
            &mut messages,
            &vlm_config,
            "{}",
            "1", // 上下文窗口 = 1 token → 必然溢出
            "gpt-4",
            &client,
        )
        .await;

        // 图片已被剥离（防止图片叠加纯文本导致溢出更严重）
        let parts = messages[0]["content"].as_array().unwrap();
        let has_image = parts
            .iter()
            .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"));
        assert!(
            !has_image,
            "image should be stripped on overflow to free space"
        );
        // 注入跳过占位符
        let texts: Vec<&str> = parts.iter().filter_map(|p| p["text"].as_str()).collect();
        let joined = texts.join(" ");
        assert!(
            joined.contains("上下文已满"),
            "should contain overflow placeholder: {joined}"
        );
        assert!(
            joined.contains("1 张图片"),
            "should mention image count: {joined}"
        );
    }

    #[tokio::test]
    async fn strip_image_blocks_no_images_in_messages() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "just text"}]
        })];

        let vlm_config = VlmConfig {
            api_key: String::new(),
            model: String::new(),
            base_url: String::new(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        strip_image_blocks(&mut messages, &vlm_config, "{}", "272000", "gpt-4", &client).await;

        // 消息应保持不变
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "just text");
    }

    #[tokio::test]
    async fn strip_image_blocks_unanalyzed_gets_placeholder() {
        // VLM 服务不可达时 → fail-open：剥离当前轮图片 + 注入"看不到图"提示。
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "test"},
                {"type": "image_url", "image_url": {"url": "https://nonexistent.example.com/img.png"}},
            ]
        })];

        let vlm_config = VlmConfig {
            api_key: "invalid-key".to_string(),
            model: "invalid-model".to_string(),
            base_url: "https://127.0.0.1:1".to_string(), // 故意不可达
            ..Default::default()
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(100))
            .build()
            .unwrap();

        strip_image_blocks(&mut messages, &vlm_config, "{}", "272000", "gpt-4", &client).await;

        // fail-open：图片被剥离（不再保留）
        let parts = messages[0]["content"].as_array().unwrap();
        let has_image = parts
            .iter()
            .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"));
        assert!(
            !has_image,
            "image should be stripped when VLM is unreachable (fail-open)"
        );
        // 注入"看不到图"提示
        let texts: Vec<&str> = parts.iter().filter_map(|p| p["text"].as_str()).collect();
        let joined = texts.join(" ");
        assert!(
            joined.contains("无法看到") || joined.contains("视觉模型"),
            "should contain cannot-see note: {joined}"
        );
    }

    /// 混合缓存命中/未命中 + VLM 不可达 → fail-open。
    /// 前 8 张在缓存中，后 7 张不在，VLM 不可达时当前轮图片被剥离 + 注入提示。
    /// 注意：当前轮 = 最后一条 user 消息（messages[1]），历史轮 = messages[0]。
    #[tokio::test]
    async fn strip_image_blocks_mixed_cache_vlm_unreachable_fail_open() {
        let mut messages: Vec<Value> = Vec::new();
        // 历史消息（15 张图）— 先 push，index 0
        let mut history_parts: Vec<Value> =
            vec![serde_json::json!({"type": "text", "text": "history"})];
        for i in 0..15 {
            history_parts.push(serde_json::json!({
                "type": "image_url",
                "image_url": {"url": format!("https://test.example.com/limit/hist-{i}.png")}
            }));
        }
        // 预填充前 8 张的缓存
        for i in 0..8 {
            cache_put(
                url_hash(&format!("https://test.example.com/limit/hist-{i}.png")),
                format!("hist-desc-{i}"),
            );
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": history_parts
        }));
        // 当前轮消息（1 张图）— 后 push，index 1（最后一条 user 消息）
        messages.push(serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "current"},
                {"type": "image_url", "image_url": {"url": "https://test.example.com/limit/current.png"}},
            ]
        }));

        let vlm_config = VlmConfig {
            api_key: "unused".to_string(),
            model: "unused".to_string(),
            base_url: "https://127.0.0.1:1".to_string(), // VLM 不可达
            ..Default::default()
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(100))
            .build()
            .unwrap();

        strip_image_blocks(&mut messages, &vlm_config, "{}", "900000", "gpt-4", &client).await;

        // fail-open：当前轮（messages[1]）图片在 Phase 5a 剥离，历史轮（messages[0]）在 step 8 剥离。
        let hist_parts = messages[0]["content"].as_array().unwrap();
        let image_count = hist_parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("image_url"))
            .count();
        assert_eq!(
            image_count, 0,
            "history images stripped in step 8"
        );
        let curr_parts = messages[1]["content"].as_array().unwrap();
        let curr_has_image = curr_parts
            .iter()
            .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"));
        assert!(!curr_has_image, "current round images stripped in Phase 5a");
        // 当前轮应有"无法看到图"提示
        let curr_texts: Vec<&str> = curr_parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect();
        let curr_joined = curr_texts.join(" ");
        assert!(
            curr_joined.contains("无法看到") || curr_joined.contains("视觉模型"),
            "current round should contain cannot-see note: {curr_joined}"
        );
    }

    // ── multi-round history test ─────────────────────────────────

    /// 25 轮对话（每轮 15 张图），全部预填充缓存。context_window=900000 → X=8093。
    /// 验证 plan_v2 X-governed 注入：
    /// - 当前轮（round 24）：不限量，15 张全注入
    /// - 黄金窗口（rounds 15-23）：cap=min(N,10)=10，仅 round 23 的前 10 张注入
    /// - 深层（rounds 0-14）：缓存命中注入，15×15=225 张全注入（远小于 X 余量）
    /// - 截断未触发（描述总字符数远小于 available×2）
    #[tokio::test]
    async fn strip_image_blocks_multi_round_depth_and_per_round_limit() {
        const ROUNDS: usize = 25;
        const IMGS_PER_ROUND: usize = 15;

        // 预填充缓存：全部 25×15=375 张图片
        for round in 0..ROUNDS {
            for img in 0..IMGS_PER_ROUND {
                let url = format!("https://multi.example.com/r{round}-i{img}.png");
                let desc = format!("round{round}-img{img}-desc");
                cache_put(url_hash(&url), desc.clone());
                // 当前轮（round 24）额外缓存 Tier2 键
                if round == ROUNDS - 1 {
                    cache_put(url_question_hash(&url, "round 24"), desc);
                }
            }
        }

        let mut messages: Vec<Value> = (0..ROUNDS)
            .map(|round| {
                let mut parts: Vec<Value> =
                    vec![serde_json::json!({"type": "text", "text": format!("round {round}")})];
                for img in 0..IMGS_PER_ROUND {
                    parts.push(serde_json::json!({
                        "type": "image_url",
                        "image_url": {"url": format!("https://multi.example.com/r{round}-i{img}.png")}
                    }));
                }
                serde_json::json!({"role": "user", "content": parts})
            })
            .collect();

        let vlm_config = VlmConfig {
            api_key: String::new(),
            model: String::new(),
            base_url: String::new(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        strip_image_blocks(&mut messages, &vlm_config, "{}", "900000", "gpt-4", &client).await;

        // 所有图片已删除
        for msg in &messages {
            if let Some(parts) = msg["content"].as_array() {
                let has_image = parts
                    .iter()
                    .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"));
                assert!(!has_image, "all images should be stripped");
            }
        }

        let collect_text = |idx: usize| -> String {
            messages[idx]["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };

        // Round 24（当前轮）：不限量，全部 15 张图片描述注入
        let current = collect_text(24);
        for img in 0..IMGS_PER_ROUND {
            assert!(
                current.contains(&format!("round24-img{img}-desc")),
                "current round missing desc for img {img}"
            );
        }

        // Rounds 15-23（黄金窗口）：cap=min(135,10)=10，仅 round 23 的前 10 张注入
        for round in 15..=22 {
            let text = collect_text(round);
            for img in 0..IMGS_PER_ROUND {
                assert!(
                    !text.contains(&format!("round{round}-img{img}-desc")),
                    "round {round} img {img}: golden cap exhausted, should NOT be injected"
                );
            }
        }
        // round 23: 前 10 张注入
        let r23_text = collect_text(23);
        for img in 0..10 {
            assert!(
                r23_text.contains(&format!("round23-img{img}-desc")),
                "round 23 img {img}: within golden cap, should be injected"
            );
        }
        for img in 10..IMGS_PER_ROUND {
            assert!(
                !r23_text.contains(&format!("round23-img{img}-desc")),
                "round 23 img {img}: beyond golden cap, should NOT be injected"
            );
        }

        // Rounds 0-14（深层）：全部缓存命中注入（225 张 < X 余量 8083）
        for round in 0..=14 {
            let text = collect_text(round);
            for img in 0..IMGS_PER_ROUND {
                assert!(
                    text.contains(&format!("round{round}-img{img}-desc")),
                    "round {round} img {img}: deep cache hit, should be injected"
                );
            }
        }
    }

    /// X ≤ 10 场景：context_window=800 → X=6。黄金窗口有 12 张缓存图。
    /// 验证：golden capped at min(N, X)=6，深层 0 张，后台不触发。
    #[tokio::test]
    async fn strip_image_blocks_tight_window_x_lte_10_golden_capped() {
        // 预填充缓存：12 张历史 + 1 张当前
        for i in 0..12 {
            cache_put(
                url_hash(&format!("https://tight.example.com/hist-{i}.png")),
                format!("hist-desc-{i}"),
            );
        }
        cache_put(
            url_hash("https://tight.example.com/curr.png"),
            "curr-desc".to_string(),
        );
        // 当前轮使用 Tier2 键（有 user_text "current"）
        cache_put(
            url_question_hash("https://tight.example.com/curr.png", "current"),
            "curr-desc".to_string(),
        );

        let mut history_parts = vec![serde_json::json!({"type": "text", "text": "history"})];
        for i in 0..12 {
            history_parts.push(serde_json::json!({
                "type": "image_url",
                "image_url": {"url": format!("https://tight.example.com/hist-{i}.png")}
            }));
        }

        let mut messages = vec![
            serde_json::json!({"role": "user", "content": history_parts}),
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "current"},
                    {"type": "image_url", "image_url": {"url": "https://tight.example.com/curr.png"}},
                ]
            }),
        ];

        let vlm_config = VlmConfig {
            api_key: String::new(),
            model: String::new(),
            base_url: String::new(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        strip_image_blocks(&mut messages, &vlm_config, "{}", "800", "gpt-4", &client).await;

        // 所有图片已删除
        for msg in &messages {
            let parts = msg["content"].as_array().unwrap();
            assert!(
                !parts
                    .iter()
                    .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"))
            );
        }

        let collect_text = |idx: usize| -> String {
            messages[idx]["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };

        // 当前轮：1 张缓存命中 → 注入（不计入 X）
        let curr_text = collect_text(1);
        assert!(
            curr_text.contains("curr-desc"),
            "current round cached desc should be injected"
        );

        // 黄金窗口（X=6 ≤ 10）：cap = min(12, 6) = 6
        // 图片按 content 顺序处理（img0→img11），前 6 张注入
        let hist_text = collect_text(0);
        for i in 0..6 {
            assert!(
                hist_text.contains(&format!("hist-desc-{i}")),
                "history img {i}: within X cap, should be injected"
            );
        }
        for i in 6..12 {
            assert!(
                !hist_text.contains(&format!("hist-desc-{i}")),
                "history img {i}: beyond X cap, should NOT be injected"
            );
        }
    }

    // ── new helpers tests ─────────────────────────────────────────

    #[test]
    fn count_images_counts_all_images_across_messages() {
        let messages = vec![
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "hi"},
                    {"type": "image_url", "image_url": {"url": "https://x.com/a.png"}},
                    {"type": "image_url", "image_url": {"url": "https://x.com/b.png"}},
                ]
            }),
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "input_image", "image_url": "data:image/png;base64,abc"},
                ]
            }),
        ];
        assert_eq!(count_images(&messages), 3);
    }

    #[test]
    fn count_images_returns_zero_for_no_images() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })];
        assert_eq!(count_images(&messages), 0);
    }

    #[test]
    fn strip_all_images_counted_returns_correct_count() {
        let mut messages = vec![
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "hi"},
                    {"type": "image_url", "image_url": {"url": "https://x.com/a.png"}},
                    {"type": "image_url", "image_url": {"url": "https://x.com/b.png"}},
                ]
            }),
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "input_image", "image_url": "data:image/png;base64,abc"},
                ]
            }),
        ];
        let n = strip_all_images_counted(&mut messages);
        assert_eq!(n, 3);
        // 确认图片已全部删除
        for msg in &messages {
            let parts = msg["content"].as_array().unwrap();
            let has_image = parts.iter().any(|p| {
                matches!(
                    p.get("type").and_then(Value::as_str),
                    Some("image_url") | Some("input_image")
                )
            });
            assert!(!has_image);
        }
    }

    #[test]
    fn strip_all_images_counted_returns_zero_when_no_images() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })];
        let n = strip_all_images_counted(&mut messages);
        assert_eq!(n, 0);
    }

    #[test]
    fn strip_images_in_message_strips_from_single_message() {
        let mut msg = serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "hi"},
                {"type": "image_url", "image_url": {"url": "https://x.com/a.png"}},
                {"type": "input_image", "image_url": "data:image/png;base64,abc"},
            ]
        });
        let n = strip_images_in_message(&mut msg);
        assert_eq!(n, 2);
        let parts = msg["content"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "hi");
    }

    #[test]
    fn strip_images_in_message_returns_zero_for_text_only() {
        let mut msg = serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        });
        let n = strip_images_in_message(&mut msg);
        assert_eq!(n, 0);
    }

    #[test]
    fn strip_images_only_counted_returns_count_and_replaces_placeholders() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}},
                {"type": "input_image", "image_url": "data:image/png;base64,abc"},
            ]
        })];
        let n = strip_images_only_counted(&mut messages);
        assert_eq!(n, 2);
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0]["text"], "hello");
        assert_eq!(parts[1]["text"], "[图片已省略]");
        assert_eq!(parts[2]["text"], "[图片已省略]");
    }

    #[test]
    fn inject_cannot_see_note_slice_messages_format() {
        let mut arr: Vec<Value> = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })];
        inject_cannot_see_note_slice(&mut arr, 3, "测试原因");
        assert_eq!(arr.len(), 2);
        // 第一条是系统消息
        let sys = &arr[0];
        assert_eq!(sys["role"], "system");
        let sys_text = sys["content"][0]["text"].as_str().unwrap();
        assert!(sys_text.contains("无法看到"));
        assert!(sys_text.contains("测试原因"));
        assert!(sys_text.contains("3 张图片"));
    }

    #[test]
    fn inject_cannot_see_note_slice_input_format() {
        let mut arr: Vec<Value> = vec![serde_json::json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}]
        })];
        inject_cannot_see_note_slice(&mut arr, 2, "测试原因");
        assert_eq!(arr.len(), 2);
        // 第一条是系统消息
        let sys = &arr[0];
        assert_eq!(sys["type"], "message");
        assert_eq!(sys["role"], "system");
        let sys_text = sys["content"][0]["text"].as_str().unwrap();
        assert!(sys_text.contains("无法看到"));
        assert!(sys_text.contains("测试原因"));
        assert!(sys_text.contains("2 张图片"));
    }

    #[test]
    fn inject_cannot_see_note_slice_noop_when_n_is_zero() {
        let mut arr: Vec<Value> = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })];
        inject_cannot_see_note_slice(&mut arr, 0, "测试原因");
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn inject_cannot_see_note_injects_into_last_user_message() {
        let mut messages = vec![
            serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "ok"}]}),
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hi"}]}),
        ];
        inject_cannot_see_note(&mut messages, 5, "测试原因");
        let parts = messages[1]["content"].as_array().unwrap();
        let first = &parts[0];
        assert_eq!(first["type"], "text");
        let text = first["text"].as_str().unwrap();
        assert!(text.contains("无法看到"));
        assert!(text.contains("测试原因"));
        assert!(text.contains("5 张图片"));
    }

    #[test]
    fn inject_cannot_see_note_noop_when_n_is_zero() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })];
        inject_cannot_see_note(&mut messages, 0, "测试原因");
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
    }

    // ── VLM failure fail-open test ────────────────────────────────

    /// VLM 永远返回 500 → 当前轮图片被剥离 + 注入"看不到图"提示 + 继续处理历史轮。
    #[tokio::test]
    async fn vlm_failure_failopen_strips_and_injects_note() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "error": {"message": "internal server error"}
            })))
            .mount(&mock_server)
            .await;

        let vlm_config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "Q1"},
                {"type": "image_url", "image_url": {"url": "https://test.example.com/img1.png"}},
            ]
        })];

        strip_image_blocks(&mut messages, &vlm_config, "{}", "272000", "gpt-4", &client).await;

        let parts = messages[0]["content"].as_array().unwrap();
        let has_image = parts
            .iter()
            .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"));
        assert!(
            !has_image,
            "VLM failure should strip images (fail-open)"
        );
        let texts: Vec<&str> = parts.iter().filter_map(|p| p["text"].as_str()).collect();
        let joined = texts.join(" ");
        assert!(
            joined.contains("无法看到") || joined.contains("视觉模型"),
            "should inject cannot-see note: {joined}"
        );
    }

    // ── wiremock integration tests ───────────────────────────────

    // ── R2 BATCH_SIZE 分批测试 ──────────────────────────────────

    /// 7 张图（>BATCH_SIZE=5）应触发 2 次 VLM 调用（5+2），描述按顺序对应。
    #[tokio::test]
    async fn call_vlm_batch_chunks_by_batch_size() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "[[图片1]]batch1-1\n[[图片2]]batch1-2\n[[图片3]]batch1-3\n[[图片4]]batch1-4\n[[图片5]]batch1-5"}}]
            })))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "[[图片1]]batch2-1\n[[图片2]]batch2-2"}}]
            })))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "k".into(), model: "m".into(), base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let urls: Vec<String> = (0..7).map(|i| format!("https://chunk.example.com/img{i}.png")).collect();

        let descs = call_vlm_batch(&urls, TIER1_PROMPT, &config, &client, BATCH_MAX_ATTEMPTS)
            .await
            .unwrap();

        assert_eq!(descs.len(), 7, "7 张图应返回 7 个描述");
        assert!(descs[0].contains("batch1-1"), "第 1 张描述对应第 1 批第 1 图");
        assert!(descs[4].contains("batch1-5"), "第 5 张描述对应第 1 批第 5 图");
        assert!(descs[5].contains("batch2-1"), "第 6 张描述对应第 2 批第 1 图");
        assert!(descs[6].contains("batch2-2"), "第 7 张描述对应第 2 批第 2 图");
    }

    /// 恰好 5 张（=BATCH_SIZE）应只触发 1 次调用。
    #[tokio::test]
    async fn call_vlm_batch_exact_batch_size_single_call() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "[[图片1]]d1\n[[图片2]]d2\n[[图片3]]d3\n[[图片4]]d4\n[[图片5]]d5"}}]
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "k".into(), model: "m".into(), base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let urls: Vec<String> = (0..5).map(|i| format!("https://exact.example.com/img{i}.png")).collect();

        let descs = call_vlm_batch(&urls, TIER1_PROMPT, &config, &client, BATCH_MAX_ATTEMPTS)
            .await
            .unwrap();
        assert_eq!(descs.len(), 5);
    }

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// strip_image_blocks 端到端：mock VLM 可用时正常注入描述。
    #[tokio::test]
    async fn strip_image_blocks_with_mock_vlm_injects_descriptions() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "mock: E2E network call"}}]
            })))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "describe this"},
                {"type": "image_url", "image_url": {"url": "https://wiremock-e2e.example.com/img.png"}},
            ]
        })];

        strip_image_blocks(&mut messages, &config, "{}", "272000", "gpt-4", &client).await;

        let parts = messages[0]["content"].as_array().unwrap();
        let has_image = parts
            .iter()
            .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"));
        assert!(!has_image, "image should be stripped");

        let last_text = parts.last().unwrap()["text"].as_str().unwrap();
        assert!(
            last_text.contains("mock: E2E network call"),
            "VLM result not injected: {last_text}"
        );
    }

    /// Plain Responses 模式：input_image 类型块 + 直接字符串 image_url。
    /// 验证 strip_image_blocks 正确处理非 CC 格式的图片块。
    #[tokio::test]
    async fn strip_image_blocks_with_responses_format_input_images() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "mock: responses format"}}]
            })))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        // Responses 格式：input_image 类型，image_url 为直接字符串
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "describe"},
                {"type": "input_image", "image_url": "https://responses.example.com/img.png"},
            ]
        })];

        strip_image_blocks(&mut messages, &config, "{}", "272000", "gpt-4", &client).await;

        let parts = messages[0]["content"].as_array().unwrap();
        let has_image = parts.iter().any(|p| {
            p.get("type")
                .and_then(Value::as_str)
                .map_or(false, |t| t == "image_url" || t == "input_image")
        });
        assert!(!has_image, "input_image should be stripped");

        let last_text = parts.last().unwrap()["text"].as_str().unwrap();
        assert!(
            last_text.contains("mock: responses format"),
            "VLM result not injected for input_image: {last_text}"
        );
    }

    /// 多轮历史图片：当前轮 + 历史轮均有图片，VLM 分析两轮并注入各自描述。
    /// 验证历史轮次的图片描述不会错误注入到当前轮。
    #[tokio::test]
    async fn strip_image_blocks_two_rounds_both_vlm_analyzed() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "mock: analyzed"}}]
            })))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
            ..Default::default()
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let mut messages = vec![
            // 历史轮：1 张图
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "historical"},
                    {"type": "image_url", "image_url": {"url": "https://two-round.example.com/hist.png"}},
                ]
            }),
            // 当前轮：1 张图
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "current"},
                    {"type": "image_url", "image_url": {"url": "https://two-round.example.com/curr.png"}},
                ]
            }),
        ];

        strip_image_blocks(&mut messages, &config, "{}", "900000", "gpt-4", &client).await;

        // 两轮图片均应被剥离
        for (i, label) in ["historical", "current"].iter().enumerate() {
            let parts = messages[i]["content"].as_array().unwrap();
            let has_image = parts
                .iter()
                .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"));
            assert!(!has_image, "{label} round: image should be stripped");
        }

        // 两轮均应注入 VLM 描述
        for (i, label) in ["historical", "current"].iter().enumerate() {
            let text = messages[i]["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                text.contains("mock: analyzed"),
                "{label} round: VLM description not injected: {text}"
            );
        }
    }
}
