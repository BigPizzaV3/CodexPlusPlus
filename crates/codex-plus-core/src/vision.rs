//! VL 视觉模型中转：纯文本模型图片理解。
//!
//! 当目标模型不支持图片输入时，把请求中的 `input_image` 调 VL（视觉语言）模型
//! 翻译为文字描述，替换为 `input_text` 后再走协议转换。context_window 控制可见
//! 窗口，窗口外的图片直接 strip。VL 失败时降级为 strip，不阻断用户。
//!
//! 本模块从 `protocol_proxy.rs` 抽出（PR #1468 Bug 4.1），行为保持不变；后续
//! 两层缓存 / 并发批次 / 重试 / 超时 / 两 prompt 在此模块内迭代。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::protocol_proxy::{chat_completions_url, model_supports_image, responses_url};
use crate::settings::{RelayProtocol, VisionRelayConfig};

/// 单次请求中 VL 处理的图片数量上限。超出部分直接 strip，不调 VL API。
const VL_IMAGE_LIMIT: usize = 10;
/// 多图一批调用（Bug 4.3）：每批最多 5 张图，一个 messages 含 5 个 image_url + 1 个 prompt。
const BATCH_SIZE: usize = 5;
/// 并发上限（Bug 4.2）：最多 5 个 VL 调用同时飞（Semaphore 零新依赖）。
const MAX_CONCURRENCY: usize = 5;
static VL_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(MAX_CONCURRENCY));
/// 混合重试（Bug 4.6）：批量 2 次（治瞬时故障）-> 失败拆单张 1 次（不折腾慢图）。
/// SINGLE_MAX_ATTEMPTS 从 3 降为 1：VL 响应约 32s，慢是稳定行为非瞬时故障，
/// 重试 3 次全 timeout 浪费（每张图 5 次 timeout × 23s ≈ 115s 白费）。
const BATCH_MAX_ATTEMPTS: u32 = 2;
const SINGLE_MAX_ATTEMPTS: u32 = 1;
/// 总超时硬截断（Bug 4.5）：180s 兜底，适配 VL 上游响应约 32s 的场景。
/// 单张重试 2 次 + 退避 3/6s = 45+45+9 ≈ 99s，留余量给多图分批。
const VL_TOTAL_TIMEOUT: Duration = Duration::from_secs(180);
/// 描述 char-safe 截断上限（Bug 4.7 廉价兜底）：>2000 字符截断，避免重蹈 Bug 3 覆辙。
const DESC_MAX_CHARS: usize = 2000;

/// 测试用总超时覆盖（生产 120s 太慢，测试设短值验证降级）。
static VL_TOTAL_TIMEOUT_OVERRIDE: Mutex<Option<Duration>> = Mutex::new(None);
#[doc(hidden)]
pub fn set_vl_total_timeout_for_tests(d: Option<Duration>) {
    *VL_TOTAL_TIMEOUT_OVERRIDE.lock().unwrap() = d;
}
fn vl_total_timeout() -> Duration {
    VL_TOTAL_TIMEOUT_OVERRIDE
        .lock()
        .unwrap()
        .or(Some(VL_TOTAL_TIMEOUT))
        .unwrap_or(VL_TOTAL_TIMEOUT)
}

/// 单批次超时（Bug 4.5）：35 + 10n 秒（n=批内图数）。reqwest `.timeout` 应用。
/// 实测 VL 上游（mimo-v2.5）响应 ≈32s，原 15+8n=23s 偏紧致全超时 strip。
fn per_batch_timeout(n: usize) -> Duration {
    Duration::from_secs(35 + 10 * n as u64)
}

/// char-safe 截断（Bug 4.7）：按字符数取前 `max` 个，避免字节截断在汉字中间 panic。
fn truncate_char_safe(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// 指数退避（Bug 4.6）：3 / 6 / 12s ... + 简单 hash 抖动（±20%，免 rand 依赖）。
/// 适配 VL 上游响应约 32s 的场景（原 0.3/0.6s 相对 32s 可忽略）。
/// `attempt` 为重试序号（1=第 2 次尝试前的等待）。
fn backoff_delay(attempt: u32, salt: &str) -> Duration {
    let base = 3.0 * 2u32.pow(attempt.saturating_sub(1)) as f64;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    attempt.hash(&mut h);
    salt.hash(&mut h);
    let jitter = (h.finish() % 20) as f64 / 100.0; // [0, 0.2)
    Duration::from_secs_f64(base * (0.8 + jitter))
}

// ── 两层缓存（Bug 4.4）──────────────────────────────────────────
//
// Tier 1（历史图）：URL key + 全面描述 prompt（不含问题）。图从"最新"变"历史"后
//   每轮命中，question-invariant -> URL 缓存稳定。省调用大头。
// Tier 2（最新/重发图）：(URL, 问题) key + 全面+侧重 prompt（含问题）。重发图+新问题
//   = 入口，触发新调用拿新信息；重复问题命中。
//
// 检测：最新 user 消息里的图 + 非空问题 -> Tier 2；其余（历史图 / 无问题最新图）-> Tier 1。
// 容量 500 / TTL 24h / 写入时淘汰最旧；DefaultHasher（u64，std，快，缓存 key 不需密码学强度）。

const CACHE_CAPACITY: usize = 500;
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

type CacheEntry = (String, Instant);
static VL_CACHE: LazyLock<Mutex<HashMap<u64, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn url_hash(url: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    h.finish()
}

fn url_question_hash(url: &str, question: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    question.hash(&mut h);
    h.finish()
}

/// 取缓存（TTL 未过期）；过期则剔除返回 None。
fn cache_get(key: u64) -> Option<String> {
    let mut cache = VL_CACHE.lock().unwrap();
    if let Some((desc, written)) = cache.get(&key).cloned() {
        if written.elapsed() < CACHE_TTL {
            return Some(desc);
        }
        cache.remove(&key);
    }
    None
}

/// 写缓存；满容量时淘汰最旧条目。
fn cache_put(key: u64, desc: String) {
    let mut cache = VL_CACHE.lock().unwrap();
    if cache.len() >= CACHE_CAPACITY {
        if let Some((&oldest_key, _)) = cache.iter().min_by_key(|(_, (_, t))| *t) {
            cache.remove(&oldest_key);
        }
    }
    cache.insert(key, (desc, Instant::now()));
}

/// 清空 VL 缓存（测试隔离用）。生产代码不应调用。
#[doc(hidden)]
pub fn cache_clear() {
    VL_CACHE.lock().unwrap().clear();
}

/// Tier 1 prompt：全面描述（不含问题），question-invariant 保证 URL 缓存稳定。
const TIER1_PROMPT: &str = "请详细描述这张图片，涵盖所有视觉信息：文字（逐字 OCR）、UI 元素、颜色、形状、布局结构、错误信息等。请用中文回复。";

/// Tier 2 prompt：全面描述 + 针对用户当前问题做更详细说明（侧重深度，入口语义）。
fn tier2_prompt(question: &str) -> String {
    format!("{TIER1_PROMPT}\n用户当前问题：{question}\n在全面描述基础上，对与上述问题相关的内容做更详细说明。")
}

/// 判断 `item_idx` 是否为最新一条 role=user 消息（用于 Tier 2 检测）。
fn is_latest_message_image(input: &[Value], item_idx: usize) -> bool {
    let latest_user = input
        .iter()
        .enumerate()
        .rev()
        .find(|(_, it)| it.get("role").and_then(Value::as_str) == Some("user"))
        .map(|(i, _)| i);
    latest_user == Some(item_idx)
}

/// 最新一条 user 消息是否含 input_image（用于判断「纯文本追问」场景）。
fn latest_user_message_has_image(input: &[Value]) -> bool {
    input
        .iter()
        .rev()
        .find(|it| it.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(|it| it.get("content").and_then(Value::as_array))
        .map(|parts| {
            parts
                .iter()
                .any(|p| p.get("type").and_then(Value::as_str) == Some("input_image"))
        })
        .unwrap_or(false)
}

/// 估算 input item 的 token 数（粗估：文本字符数 / 4，跳过图片 base64 数据）。
fn estimate_item_tokens(item: &Value) -> usize {
    let Some(content) = item.get("content") else {
        return 0;
    };
    if let Some(s) = content.as_str() {
        return s.len() / 4;
    }
    let Some(parts) = content.as_array() else {
        return 0;
    };
    let mut chars = 0;
    for part in parts {
        let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(part_type, "input_text" | "output_text" | "text") {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                chars += text.len();
            }
        }
    }
    chars / 4
}

/// 从末尾遍历 input items，累积 token 直到超过 context_window。
/// 返回需要 VL 处理的 item 索引列表（已按原始顺序排列）。
/// 跳过 role=system（injected 提示），不计入 token 预算。
/// context_window=0 表示不限制，返回全部索引。
fn items_within_vl_window(input: &[Value], context_window: u64) -> Vec<usize> {
    if context_window == 0 {
        return (0..input.len())
            .filter(|&i| input.get(i).and_then(|it| it.get("role").and_then(Value::as_str)) != Some("system"))
            .collect();
    }
    let mut tokens: u64 = 0;
    let mut indices = vec![];
    for i in (0..input.len()).rev() {
        if input[i].get("role").and_then(Value::as_str) == Some("system") {
            continue;
        }
        tokens += estimate_item_tokens(&input[i]) as u64;
        if tokens > context_window {
            break;
        }
        indices.push(i);
    }
    indices.reverse();
    indices
}

/// 从 input items 中收集用户原文（role=user 消息里的 input_text）。
/// 最新 user 消息有文字 -> 取最新文字；
/// 最新 user 消息无文字（纯发图）-> 回溯最近一条有文字的 user 消息（追问上下文）。
/// 跳过 role=system（injected 提示）和 role=assistant。
fn collect_input_text(input: &[Value]) -> String {
    for item in input.iter().rev() {
        if item.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let Some(content) = item.get("content") else {
            continue;
        };
        let Some(parts) = content.as_array() else {
            continue;
        };
        for part in parts {
            if part.get("type").and_then(Value::as_str) == Some("input_text") {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    return text.to_string();
                }
            }
        }
        // 最新消息无 input_text（纯发图）-> 继续回溯
    }
    String::new()
}

/// 从 input_image part 中提取 image_url。
/// 支持字符串格式（Responses）和对象格式 `{url, detail}`（ChatCompletions）。
fn extract_image_url(part: &Value) -> Option<String> {
    let iu = part.get("image_url")?;
    if let Some(s) = iu.as_str() {
        return Some(s.to_string());
    }
    iu.get("url").and_then(Value::as_str).map(|s| s.to_string())
}

/// 构造 VL 请求体。1 张图时为单图请求（与历史 describe_image_with_vl 格式一致，
/// 保留单图测试的请求体断言）；>1 张图时附加 `[[图片K]]` 标注指令，便于按段拆分。
fn build_vl_batch_body(urls: &[String], prompt: &str, config: &VisionRelayConfig) -> Value {
    let final_prompt = if urls.len() > 1 {
        format!(
            "{prompt}\n请按顺序描述以下{}张图片，每张图片的描述以 [[图片K]] 开头（K=1..{n}），每张单独描述。",
            urls.len(),
            n = urls.len()
        )
    } else {
        prompt.to_string()
    };
    match config.protocol {
        RelayProtocol::ChatCompletions => {
            let mut content = vec![json!({"type":"text","text":final_prompt})];
            for u in urls {
                content.push(json!({"type":"image_url","image_url":{"url":u}}));
            }
            json!({
                "model": config.model,
                "messages": [{"role":"user","content":content}],
                "max_tokens": config.max_tokens,
            })
        }
        RelayProtocol::Responses => {
            let mut content = vec![json!({"type":"input_text","text":final_prompt})];
            for u in urls {
                content.push(json!({"type":"input_image","image_url":u}));
            }
            json!({
                "model": config.model,
                "input": [{"role":"user","content":content}],
                "max_output_tokens": config.max_tokens,
            })
        }
    }
}

/// 从 VL 响应中提取文字内容（ChatCompletions: choices[0].message.content；Responses: output[0].content[0].text）。
fn extract_vl_text(response_body: &Value, protocol: RelayProtocol) -> anyhow::Result<String> {
    let text = match protocol {
        RelayProtocol::ChatCompletions => response_body["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string()),
        RelayProtocol::Responses => response_body["output"][0]["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string()),
    };
    text.ok_or_else(|| anyhow::anyhow!("VL API returned no text content"))
}

/// 把多图批次的 VL 响应按 `[[图片K]]` 标注拆成 n 段描述。任一标注缺失返回 None。
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

/// 单次批量调 VL（不含重试）：一组同 tier 图片（≤BATCH_SIZE）一次 API 调用，
/// Semaphore 限并发，返回每图描述（按 urls 顺序）。1 张图直接取整段文本；
/// >1 张图按 `[[图片K]]` 标注拆分；拆分失败返回 Err（调用方回退单张）。
async fn call_vlm_batch_once(
    urls: &[String],
    prompt: &str,
    config: &VisionRelayConfig,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<String>> {
    let _permit = VL_SEMAPHORE.acquire().await.expect("VL_SEMAPHORE 未关闭");
    let endpoint = match config.protocol {
        RelayProtocol::ChatCompletions => chat_completions_url(&config.base_url),
        RelayProtocol::Responses => responses_url(&config.base_url),
    };
    let body = build_vl_batch_body(urls, prompt, config);
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
            // 区分超时 vs 其他网络错误（reqwest is_timeout）
            let status = if e.is_timeout() { "timeout" } else { "send_error" };
            log_vl_call(config, n, started, status, None, Some(&e.to_string()));
            return Err(e.into());
        }
    };
    let http_code = response.status().as_u16();
    let response_body: Value = match response.json().await {
        Ok(v) => v,
        Err(e) => {
            log_vl_call(config, n, started, "json_error", Some(http_code), Some(&e.to_string()));
            return Err(e.into());
        }
    };
    if !status_is_success(http_code) {
        let msg = format!(
            "VL API returned {}: {}",
            http_code,
            serde_json::to_string(&response_body).unwrap_or_default()
        );
        log_vl_call(config, n, started, "http_error", Some(http_code), Some(&msg));
        anyhow::bail!(msg);
    }
    let text = match extract_vl_text(&response_body, config.protocol) {
        Ok(t) => t,
        Err(e) => {
            log_vl_call(config, n, started, "no_text", Some(http_code), Some(&e.to_string()));
            return Err(e);
        }
    };
    if n <= 1 {
        log_vl_call(config, n, started, "ok", Some(http_code), None);
        return Ok(vec![text]);
    }
    match parse_batch_descriptions(&text, n) {
        Some(descs) => {
            log_vl_call(config, n, started, "ok", Some(http_code), None);
            Ok(descs)
        }
        None => {
            let msg = format!(
                "VL 批量响应未按 [[图片K]] 标注返回 {} 段（期望 {} 段）",
                text.matches("[[图片").count(),
                n
            );
            log_vl_call(config, n, started, "parse_error", Some(http_code), Some(&msg));
            Err(anyhow::anyhow!(msg))
        }
    }
}

fn status_is_success(code: u16) -> bool {
    (200..300).contains(&code)
}

/// 记单次 VL HTTP 调用（取证用）：状态(ok/timeout/http_error/...)/耗时/错误。
fn log_vl_call(
    config: &VisionRelayConfig,
    n_urls: usize,
    started: Instant,
    status: &str,
    http_code: Option<u16>,
    error: Option<&str>,
) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.vl_call",
        json!({
            "vlModel": config.model,
            "n_urls": n_urls,
            "duration_ms": started.elapsed().as_millis() as u64,
            "status": status,
            "http_code": http_code,
            "error": error,
        }),
    );
}

/// 带混合重试的批量调 VL（Bug 4.6）：失败按指数退避重试 `max_attempts` 次。
/// 批量用 BATCH_MAX_ATTEMPTS（治瞬时故障）；单张回退用 SINGLE_MAX_ATTEMPTS（隔离坏图）。
/// 每次重试前 sleep `backoff_delay`（0.3/0.6s + 抖动）。
async fn call_vlm_batch(
    urls: &[String],
    prompt: &str,
    config: &VisionRelayConfig,
    client: &reqwest::Client,
    max_attempts: u32,
) -> anyhow::Result<Vec<String>> {
    let salt = urls.first().map(String::as_str).unwrap_or("");
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..max_attempts {
        if attempt > 0 {
            tokio::time::sleep(backoff_delay(attempt, salt)).await;
        }
        match call_vlm_batch_once(urls, prompt, config, client).await {
            Ok(v) => return Ok(v),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("max_attempts 为 0")))
}

/// VL 入口（公开）：总超时硬截断兜底（Bug 4.5）。超时或内部错误 -> strip 剩余图片，
/// 返回 Ok（不阻断用户）。
pub async fn analyze_images_with_vl(
    body: &mut Value,
    vl_config: &VisionRelayConfig,
    client: &reqwest::Client,
) -> anyhow::Result<()> {
    if !vl_config.enabled {
        return Ok(());
    }
    let outcome = tokio::time::timeout(
        vl_total_timeout(),
        analyze_images_with_vl_inner(body, vl_config, client),
    )
    .await;
    match outcome {
        Ok(Ok(())) => Ok(()),
        // 内部错误或总超时：降级 strip 剩余 input_image，不阻断用户。
        // 记 vl_strip（区分 strip 与 vl_described 成功）+ 注入「看不到图」系统提示防胡说。
        Ok(Err(e)) => {
            let n = count_input_images(body);
            let err_msg = e.to_string();
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.vl_strip",
                json!({ "reason": "inner_error", "n_images": n, "error": err_msg }),
            );
            let stripped = strip_all_input_images(body);
            if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
                inject_image_strip_note(input, stripped, "VL 内部错误");
            }
            Ok(())
        }
        Err(_) => {
            let n = count_input_images(body);
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.vl_strip",
                json!({ "reason": "total_timeout", "n_images": n, "timeout_sec": VL_TOTAL_TIMEOUT.as_secs() }),
            );
            let stripped = strip_all_input_images(body);
            if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
                inject_image_strip_note(input, stripped, "VL 总超时");
            }
            Ok(())
        }
    }
}

/// 移除 body 中所有 input_image 块（超时/失败降级 strip 用）。
/// 同时清理 Phase 1 超限标记的空对象（{}），避免超时时残留畸形 part 上游。
fn strip_all_input_images(body: &mut Value) -> usize {
    let mut stripped = 0;
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input.iter_mut() {
            if let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) {
                let before = parts.len();
                parts.retain(|p| {
                    p.get("type").and_then(Value::as_str) != Some("input_image")
                        && !p.as_object().map_or(false, |o| o.is_empty())
                });
                stripped += before - parts.len();
            }
        }
    }
    stripped
}

/// 统计 body 中剩余 input_image 数量。
fn count_input_images(body: &Value) -> usize {
    body.get("input")
        .and_then(Value::as_array)
        .map(|input| {
            input
                .iter()
                .filter_map(|it| it.get("content").and_then(Value::as_array))
                .map(|parts| {
                    parts
                        .iter()
                        .filter(|p| p.get("type").and_then(Value::as_str) == Some("input_image"))
                        .count()
                })
                .sum()
        })
        .unwrap_or(0)
}

/// strip 发生时往 input 数组注入系统提示，告知模型看不到图片（防胡说误导用户）。
/// role:"system" 在 input 数组里，Chat 路径转 system 消息、Responses 透传均支持。
fn inject_image_strip_note(input: &mut Vec<Value>, stripped_count: usize, reason: &str) {
    if stripped_count == 0 {
        return;
    }
    let note = format!(
        "[系统提示：用户发送了 {stripped_count} 张图片，但视觉模型当前不可用（{reason}），图片内容未被处理。你无法看到这些图片。请如实告知用户你暂时无法查看图片，不要猜测或编造图片内容。]"
    );
    input.insert(
        0,
        json!({
            "type": "message",
            "role": "system",
            "content": [{"type": "input_text", "text": note}]
        }),
    );
}

/// 文本追问时注入提示：历史图已有描述，能答直接答，需要细节让用户重发图+问题。
fn inject_followup_note(input: &mut Vec<Value>, n_history_images: usize) {
    let note = format!(
        "[系统提示：用户之前发送了 {n_history_images} 张图片，这些图片的文字描述已在上面（# 图片内容描述）。\n请优先从这些描述回答用户的问题。\n\n⚠️ 重要限制：如果你认为用户追问的是描述中没有明确覆盖的细节，必须如实告知用户「你的问题需要我重新查看原始图片，请重新发送图片并附上问题」。\n绝对不要猜测或编造图片中未描述的细节。]"
    );
    input.insert(
        0,
        json!({
            "type": "message",
            "role": "system",
            "content": [{"type": "input_text", "text": note}]
        }),
    );
}

/// 若 body 含 input_image，注入「看不到图」系统提示（用于 VL 未启用等 strip 路径）。
/// 返回被 strip 的图片数（0 表示无图、不注入）。供 protocol_proxy 转换层 strip 路径调用。
pub fn inject_strip_note_if_images(body: &mut Value, reason: &str) -> usize {
    let count = count_input_images(body);
    if count > 0 {
        if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
            inject_image_strip_note(input, count, reason);
        }
    }
    count
}

/// 遍历 input 中的 input_image 块，调 VL API 翻译为文字描述替换为 input_text。
/// context_window=0 不限制窗口；>0 时窗口外的图片直接 strip。
async fn analyze_images_with_vl_inner(
    body: &mut Value,
    vl_config: &VisionRelayConfig,
    client: &reqwest::Client,
) -> anyhow::Result<()> {
    if !vl_config.enabled {
        return Ok(());
    }

    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return Ok(());
    };

    let window_indices = items_within_vl_window(input, vl_config.context_window);
    let user_text = collect_input_text(input);
    // 方案 3：Phase 0 — 在替换图片为 text 前，判「最新消息是否含图」。
    // 若最新消息无图（纯文本追问）+ 窗口内有历史图 -> Phase 4 注入追问提示。
    // 必须在 Phase 1 前捕获，因为 Phase 1 会替换掉 input_image。
    let is_followup_query = !user_text.is_empty() && !latest_user_message_has_image(input)
        && window_indices.iter().any(|&idx| {
            input.get(idx)
                .and_then(|it| it.get("content").and_then(Value::as_array))
                .map(|parts| parts.iter().any(|p| p.get("type").and_then(Value::as_str) == Some("input_image")))
                .unwrap_or(false)
        });

    // Phase 1：遍历窗口内图片。命中缓存 -> 立即替换为 input_text；超限 -> 标记空对象；
    // 未命中 -> 收集为批量任务（保留 input_image 占位，索引稳定，Phase 3 按索引回填）。
    #[derive(Clone)]
    struct VlTask {
        item_idx: usize,
        part_idx: usize,
        url: String,
        key: u64,
        is_tier2: bool,
    }
    let mut tasks: Vec<VlTask> = Vec::new();
    let mut vl_count = 0;
    let mut over_limit_count: usize = 0;
    for &idx in &window_indices {
        let is_latest = is_latest_message_image(input, idx);
        let Some(parts) = input
            .get_mut(idx)
            .and_then(|it| it.get_mut("content").and_then(Value::as_array_mut))
        else {
            continue;
        };
        for (pi, part) in parts.iter_mut().enumerate() {
            if part.get("type").and_then(Value::as_str) != Some("input_image") {
                continue;
            }
            let Some(img_url) = extract_image_url(part) else {
                continue;
            };
            vl_count += 1;
            if vl_count > VL_IMAGE_LIMIT {
                // 超出上限：标记为空对象，Phase 4 统一清理；计数 + 记日志用于注入提示
                over_limit_count += 1;
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "protocol_proxy.vl_strip",
                    json!({ "reason": "over_limit", "scope": "image", "limit": VL_IMAGE_LIMIT }),
                );
                *part = json!({});
                continue;
            }
            // 两层缓存检测（Bug 4.4/4.8）：
            //   最新消息里的图 + 非空问题 -> Tier 2（(URL,问题) key，含问题 prompt，入口）
            //   其余（历史图 / 无问题最新图）-> Tier 1（URL key，无问题 prompt）
            let (cache_key, is_tier2) = if is_latest && !user_text.is_empty() {
                (url_question_hash(&img_url, &user_text), true)
            } else {
                (url_hash(&img_url), false)
            };
            if let Some(cached) = cache_get(cache_key) {
                *part = json!({
                    "type": "input_text",
                    "text": format!("# 图片内容描述\n\n{cached}")
                });
            } else {
                // 占位保留 input_image，Phase 3 按索引回填（不改数组长度，索引稳定）
                tasks.push(VlTask {
                    item_idx: idx,
                    part_idx: pi,
                    url: img_url,
                    key: cache_key,
                    is_tier2,
                });
            }
        }
    }

    // Phase 2：按 tier 分组 -> 分批（BATCH_SIZE）-> JoinSet 并发（Semaphore 限流）。
    // 同 tier 共享 prompt，故可同批；Tier 1 用 TIER1_PROMPT，Tier 2 用 tier2_prompt。
    let tier2_prompt_str = tier2_prompt(&user_text);
    let mut set: JoinSet<(Vec<VlTask>, anyhow::Result<Vec<String>>)> = JoinSet::new();
    for tier2 in [false, true] {
        let group: Vec<VlTask> = tasks.iter().filter(|t| t.is_tier2 == tier2).cloned().collect();
        let prompt = if tier2 {
            tier2_prompt_str.clone()
        } else {
            TIER1_PROMPT.to_string()
        };
        for chunk in group.chunks(BATCH_SIZE) {
            let urls: Vec<String> = chunk.iter().map(|t| t.url.clone()).collect();
            let cfg = vl_config.clone();
            let cl = client.clone();
            let p = prompt.clone();
            let chunk_owned = chunk.to_vec();
            let max_att = BATCH_MAX_ATTEMPTS;
            set.spawn(async move {
                let r = call_vlm_batch(&urls, &p, &cfg, &cl, max_att).await;
                (chunk_owned, r)
            });
        }
    }

    // Phase 3：join 各批结果。批次成功 -> 缓存 + 回填；批次失败 -> 回退单张逐个调
    // （单张重试 3 次；单张仍失败则该图描述为空，Phase 4 strip + 计入 vl_failed_count）。
    let mut vl_failed_count: usize = 0;
    while let Some(joined) = set.join_next().await {
        let (chunk_tasks, result) = joined.expect("VL 批次任务 panic");
        let descriptions: Vec<String> = match result {
            Ok(d) if d.len() == chunk_tasks.len() => d,
            _ => {
                // 批次失败或段数不符 -> 单张回退
                let prompt_for = |is_tier2: bool| {
                    if is_tier2 {
                        tier2_prompt_str.clone()
                    } else {
                        TIER1_PROMPT.to_string()
                    }
                };
                let mut descs = Vec::with_capacity(chunk_tasks.len());
                for t in &chunk_tasks {
                    let p = prompt_for(t.is_tier2);
                    match call_vlm_batch(&[t.url.clone()], &p, vl_config, client, SINGLE_MAX_ATTEMPTS)
                        .await
                    {
                        Ok(d) if !d.is_empty() => descs.push(d[0].clone()),
                        _ => descs.push(String::new()),
                    }
                }
                descs
            }
        };
        for (t, raw_desc) in chunk_tasks.iter().zip(descriptions.iter()) {
            if raw_desc.is_empty() {
                // 单张重试仍失败 -> 标记 strip（空对象，Phase 4 清理）+ 记日志 + 计数
                vl_failed_count += 1;
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "protocol_proxy.vl_strip",
                    json!({
                        "reason": "vl_failed",
                        "scope": "image",
                        "vlModel": vl_config.model,
                        "image_url_len": t.url.len(),
                        "image_url_is_data": t.url.starts_with("data:")
                    }),
                );
                if let Some(part) = input
                    .get_mut(t.item_idx)
                    .and_then(|it| it.get_mut("content"))
                    .and_then(Value::as_array_mut)
                    .and_then(|arr| arr.get_mut(t.part_idx))
                {
                    *part = json!({});
                }
                continue;
            }
            // Bug 4.7：char-safe 截断（>2000 字符），避免重蹈 Bug 3 字节截断 panic 覆辙
            let desc = truncate_char_safe(raw_desc, DESC_MAX_CHARS);
            cache_put(t.key, desc.clone());
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.vl_described",
                json!({
                    "vlModel": vl_config.model,
                    "image_url_len": t.url.len(),
                    "image_url_is_data": t.url.starts_with("data:"),
                    "description_len": desc.len(),
                    "description_chars": desc.chars().count()
                }),
            );
            if let Some(part) = input
                .get_mut(t.item_idx)
                .and_then(|it| it.get_mut("content"))
                .and_then(Value::as_array_mut)
                .and_then(|arr| arr.get_mut(t.part_idx))
            {
                *part = json!({
                    "type": "input_text",
                    "text": format!("# 图片内容描述\n\n{desc}")
                });
            }
        }
    }

    // Phase 4：清理超限/失败标记的空对象（窗口内）
    for &idx in &window_indices {
        if let Some(parts) = input
            .get_mut(idx)
            .and_then(|it| it.get_mut("content").and_then(Value::as_array_mut))
        {
            parts.retain(|p| !p.as_object().map_or(false, |o| o.is_empty()));
        }
    }

    // 防御纵深：若有图片被 strip（超限 / VL 调用失败），注入「看不到图」系统提示，
    // 避免基座模型拿不到图片信息却胡编（用户被误导）。窗口外 strip 不注入（本就不该见）。
    let total_stripped = over_limit_count + vl_failed_count;
    if total_stripped > 0 {
        let reason = match (over_limit_count, vl_failed_count) {
            (o, 0) if o > 0 => format!("超出单次处理上限({VL_IMAGE_LIMIT}张)"),
            (0, v) if v > 0 => "视觉模型调用失败".to_string(),
            (o, v) => format!("{o} 张超出上限、{v} 张视觉模型调用失败"),
        };
        inject_image_strip_note(input, total_stripped, &reason);
    }

    // 方案 3：纯文本追问 + 窗口内有历史图 -> 注入提示（描述已给出，能答直接答，
    // 需要细节让用户重发图+问题）。只注入一次，strip 路径优先（strip 时模型本就看不了图）。
    if total_stripped == 0 && is_followup_query {
        inject_followup_note(input, window_indices.len());
    }

    // 窗口外的图片 strip 掉
    let window_set: std::collections::HashSet<usize> = window_indices.into_iter().collect();
    for (idx, item) in input.iter_mut().enumerate() {
        if window_set.contains(&idx) {
            continue;
        }
        if let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) {
            parts.retain(|part| part.get("type").and_then(Value::as_str) != Some("input_image"));
        }
    }

    Ok(())
}

/// VL 入口：判断是否需要 VL 预处理，失败时降级为 strip（不阻断用户）。
/// 返回 `(supports_image, body)` -- 调用方据此决定后续 strip 行为。
pub async fn apply_vl_with_fallback(
    relay: &crate::settings::RelayProfile,
    request_json: Value,
    vision_relay: &VisionRelayConfig,
    user_agent: &str,
) -> anyhow::Result<(bool, Value)> {
    let model = request_json
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("");
    let base_supports_image = model_supports_image(relay, model);

    if base_supports_image || !vision_relay.enabled {
        return Ok((base_supports_image, request_json));
    }

    let client = crate::http_client::proxied_client(user_agent)?;
    let mut vl_body = request_json.clone();
    match analyze_images_with_vl(&mut vl_body, vision_relay, &client).await {
        Ok(()) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.vl_preprocess_ok",
                json!({
                    "relayId": relay.id,
                    "relayName": relay.name,
                    "model": model,
                    "vlModel": vision_relay.model
                }),
            );
            Ok((true, vl_body))
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.vl_preprocess_failed",
                json!({
                    "relayId": relay.id,
                    "relayName": relay.name,
                    "model": model,
                    "vlModel": vision_relay.model,
                    "error": error.to_string()
                }),
            );
            Ok((false, request_json))
        }
    }
}
