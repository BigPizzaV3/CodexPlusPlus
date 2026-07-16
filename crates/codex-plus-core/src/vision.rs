/// VLM (Vision Language Model) analysis for send-as-is models.
/// Batches images into groups, sends each batch as one API call.
/// Includes image-description cache, retry, concurrency limits,
/// round-depth control, dynamic context-window overflow protection,
/// and two-phase (sync + background) analysis with X-governed injection.
use serde_json::Value;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const BATCH_SIZE: usize = 5;
/// 黄金窗口：Phase 1 同步补全的最近 N 轮 user 消息。
const GOLDEN_WINDOW_DEPTH: usize = 10;
/// Phase 2 后台可推进的最大深度（user 消息轮数）。
const ANALYZE_DEPTH_LIMIT: usize = 50;
/// 每条描述的平均 token 预算（~200 chars → estimate_tokens = 200/2 = 100）。
const AVG_DESC_BUDGET: u64 = 100;
/// per-batch 最大重试次数（共 3 次尝试）。
const MAX_RETRIES: u32 = 2;
/// 全局 VLM 并发上限（跨请求）。
const MAX_GLOBAL_VLM_CONCURRENCY: usize = 5;
/// analyze_all 总超时。
const ANALYZE_ALL_TIMEOUT: Duration = Duration::from_secs(120);
/// VLM 返回的错误文本截断长度。
const ERROR_BODY_TRUNCATE: usize = 256;
/// 上下文窗口安全余量（0.9 = 留 10% 给上游 tokenizer 差异）。
const CONTEXT_SAFETY_MARGIN: f64 = 0.9;

// ── Global state ──────────────────────────────────────────────────────

/// 缓存容量上限。
const MAX_CACHE_CAPACITY: usize = 500;
/// 缓存 TTL（24 小时）。
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

/// 缓存条目：(描述文本, 写入时间)。
type CacheEntry = (String, Instant);

// ── Global state ──────────────────────────────────────────────────────

/// 图片描述缓存：key=URL 的 SHA256 前 16 字节 hex，value=(描述, 写入时间)。
static VLM_CACHE: LazyLock<Mutex<HashMap<String, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 全局 VLM 信号量，限制跨请求并发数。
static VLM_SEMAPHORE: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(MAX_GLOBAL_VLM_CONCURRENCY));

fn vlm_semaphore() -> &'static tokio::sync::Semaphore {
    &VLM_SEMAPHORE
}

/// 查缓存（自动清理过期条目）。
fn cache_get(key: &str) -> Option<String> {
    let mut cache = VLM_CACHE.lock().unwrap();
    let entry = cache.get(key)?;
    if entry.1.elapsed() > CACHE_TTL {
        cache.remove(key);
        return None;
    }
    Some(entry.0.clone())
}

/// 写缓存（达到容量上限时清理所有过期条目，若仍满则删除 1/4 旧条目）。
fn cache_insert(key: String, value: String) {
    let mut cache = VLM_CACHE.lock().unwrap();
    if cache.len() >= MAX_CACHE_CAPACITY {
        // 先清理过期条目
        cache.retain(|_, (_, ts)| ts.elapsed() <= CACHE_TTL);
        // 若仍满，删除最旧的 1/4
        if cache.len() >= MAX_CACHE_CAPACITY {
            let remove_count = MAX_CACHE_CAPACITY / 4;
            let mut entries: Vec<_> = cache.iter().map(|(k, (_, ts))| (k.clone(), *ts)).collect();
            entries.sort_by_key(|(_, ts)| *ts);
            for (k, _) in entries.iter().take(remove_count) {
                cache.remove(k);
            }
        }
    }
    cache.insert(key, (value, Instant::now()));
}

/// 判断 key 是否在缓存中且未过期。
#[allow(dead_code)]
fn cache_contains(key: &str) -> bool {
    let cache = VLM_CACHE.lock().unwrap();
    cache
        .get(key)
        .map(|(_, ts)| ts.elapsed() <= CACHE_TTL)
        .unwrap_or(false)
}

// ── Configuration ─────────────────────────────────────────────────────

#[derive(Clone)]
pub struct VlmConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
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

// ── URL hashing ───────────────────────────────────────────────────────

fn url_hash(url: &str) -> String {
    let hash = Sha256::digest(url.as_bytes());
    hash[..16].iter().map(|b| format!("{b:02x}")).collect()
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

// ── VLM API call ──────────────────────────────────────────────────────

/// VLM HTTP 请求超时：测试中用短超时以便 wiremock 触发超时路径。
#[cfg(not(test))]
const VLM_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
#[cfg(test)]
const VLM_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// 单 batch VLM 调用（含错误详情截断）。
async fn call_vlm_batch(urls: &[String], config: &VlmConfig) -> Result<String, String> {
    let client = crate::http_client::vlm_http_client_with_timeout(
        std::time::Duration::from_secs(5),
        VLM_REQUEST_TIMEOUT,
    )
    .map_err(|e| format!("client: {e}"))?;
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut parts: Vec<Value> = urls
        .iter()
        .map(|u| serde_json::json!({"type": "image_url", "image_url": {"url": u}}))
        .collect();
    parts.push(serde_json::json!({
        "type": "text",
        "text": "请描述图片内容。如包含文字，请精确提取图片中的文字。"
    }));
    let body = serde_json::json!({
        "model": config.model,
        "messages": [{"role": "user", "content": parts}],
        "stream": false,
    });
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        let truncated: String = body_text.chars().take(ERROR_BODY_TRUNCATE).collect();
        return Err(format!("VLM API {}: {}", status, truncated));
    }
    let data: Value = resp
        .json()
        .await
        .map_err(|e| format!("parse failed: {e}"))?;
    data["choices"][0]["message"]["content"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "no content".to_string())
}

/// 判断错误是否可重试（网络/服务端/限流），4xx 不重试。
fn is_retryable(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("timeout")
        || lower.contains("request failed")
        || lower.contains("vlm api 502")
        || lower.contains("vlm api 503")
        || lower.contains("vlm api 504")
        || lower.contains("vlm api 429")
}

/// 带重试的单 batch 调用。
async fn call_vlm_batch_with_retry(urls: &[String], config: &VlmConfig) -> Result<String, String> {
    let mut last_err = String::new();
    for attempt in 0..=MAX_RETRIES {
        match call_vlm_batch(urls, config).await {
            Ok(text) => return Ok(text),
            Err(e) => {
                last_err = e;
                if !is_retryable(&last_err) || attempt == MAX_RETRIES {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500 * 2_u64.pow(attempt))).await;
            }
        }
    }
    Err(last_err)
}

/// 单请求内最大并发 batch 数。
const PER_REQUEST_CONCURRENCY: usize = 3;

// ── Batch analysis ────────────────────────────────────────────────────

/// 并发分析全部图片 batch（带全局信号量、JoinSet 并发、总超时、fail-closed）。
/// 超时时保留已完成 batch 的结果，未完成的丢弃。
/// 返回 `Vec<Option<String>>`：`Some(text)` 为成功 batch 的描述，`None` 为失败 batch。
/// 全部失败返回 Err。
pub async fn analyze_all(
    urls: &[String],
    config: &VlmConfig,
) -> Result<Vec<Option<String>>, String> {
    if urls.is_empty() {
        return Ok(Vec::new());
    }
    let batches: Vec<Vec<String>> = urls
        .chunks(BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect();

    let batch_count = batches.len();
    let local_sem = std::sync::Arc::new(tokio::sync::Semaphore::new(PER_REQUEST_CONCURRENCY));
    // 预分配按 batch index 写入，解决 JoinSet 完成顺序 ≠ batch 顺序的排序问题。
    let outcomes: std::sync::Arc<std::sync::Mutex<Vec<Option<Result<String, String>>>>> =
        std::sync::Arc::new(std::sync::Mutex::new(
            (0..batch_count).map(|_| None).collect(),
        ));

    let work = {
        let outcomes = outcomes.clone();
        let local_sem = local_sem.clone();
        async move {
            let mut set = tokio::task::JoinSet::new();
            for (batch_idx, batch) in batches.iter().enumerate() {
                let batch = batch.clone();
                let config = config.clone();
                let gs = vlm_semaphore();
                let ls = local_sem.clone();
                let outcomes = outcomes.clone();
                set.spawn(async move {
                    let _l = ls.acquire().await;
                    let _g = gs.acquire().await;
                    let result = call_vlm_batch_with_retry(&batch, &config).await;
                    outcomes.lock().unwrap()[batch_idx] = Some(result);
                });
            }
            while (set.join_next().await).is_some() {}
        }
    };

    tokio::select! {
        _ = work => {},
        _ = tokio::time::sleep(ANALYZE_ALL_TIMEOUT) => {
            // work future 被 drop → JoinSet 被 drop → 剩余 task 被取消
            // outcomes 中已完成的 task 保留在对应 batch_idx，未完成的保持 None
        }
    }

    let outcomes: Vec<Option<Result<String, String>>> =
        std::mem::take(&mut *outcomes.lock().unwrap());

    // 按 batch 原始顺序收集结果（None 视为失败）。
    let mut success_count = 0usize;
    let mut results: Vec<Option<String>> = Vec::with_capacity(batch_count);
    for outcome in &outcomes {
        match outcome {
            Some(Ok(text)) => {
                success_count += 1;
                results.push(Some(text.clone()));
            }
            _ => results.push(None),
        }
    }

    if success_count == 0 {
        return Err("all VLM calls failed (fail-closed: images preserved)".to_string());
    }

    Ok(results)
}

// ── Phase 2 后台分析 ────────────────────────────────────────────────────

/// 后台分析图片并写入缓存（不注入到消息中）。
/// 失败静默跳过，缓存保持未命中状态供后续请求重试。
async fn background_analyze_and_cache(urls: &[String], config: &VlmConfig) {
    if urls.is_empty() {
        return;
    }
    match analyze_all(urls, config).await {
        Ok(batch_results) => {
            let mut url_offset = 0usize;
            for batch_opt in &batch_results {
                let batch_end = (url_offset + BATCH_SIZE).min(urls.len());
                if let Some(text) = batch_opt {
                    for url in &urls[url_offset..batch_end] {
                        cache_insert(url_hash(url), text.clone());
                    }
                }
                url_offset = batch_end;
            }
        }
        Err(_) => {
            // 后台失败 → 静默跳过，缓存保持未命中。
        }
    }
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
pub async fn strip_image_blocks(
    messages: &mut [Value],
    vlm_config: &VlmConfig,
    model_windows_json: &str,
    context_window_str: &str,
    request_model: &str,
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
        // 上下文已满：剥离图片释放空间，注入占位符告知模型图片被跳过（而非静默丢弃）。
        let image_count: usize = messages
            .iter()
            .rev()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("user"))
            .take(1)
            .flat_map(|m| collect_urls(m))
            .count();
        strip_all_images(messages);
        if image_count > 0 {
            inject_text_into_user_message(
                messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
                    .expect("at least one user message exists"),
                &format!(
                    "\n[系统：当前轮次有 {} 张图片因上下文已满未完成 VLM 分析，图片已被清理以释放空间]",
                    image_count
                ),
            );
        }
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "vlm_context_overflow",
            json!({
                "context_window": context_window,
                "text_only_estimated_tokens": current_tokens,
                "skipped_images": image_count,
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
    ) -> Result<(), ()> {
        if round_urls.is_empty() {
            return Ok(());
        }
        match analyze_all(round_urls, vlm_config).await {
            Ok(batch_results) => {
                let mut url_offset = 0usize;
                for batch_opt in &batch_results {
                    let batch_end = (url_offset + BATCH_SIZE).min(round_urls.len());
                    let batch_text = match batch_opt {
                        Some(t) => t.clone(),
                        None => "[部分图片无法识别]".to_string(),
                    };
                    for url in &round_urls[url_offset..batch_end] {
                        if batch_opt.is_some() {
                            cache_insert(url_hash(url), batch_text.clone());
                        }
                        descriptions
                            .entry(msg_idx)
                            .or_default()
                            .push_str(&format!("\n[图片描述] {batch_text}"));
                    }
                    url_offset = batch_end;
                }
                Ok(())
            }
            Err(_) => Err(()),
        }
    }

    // 5a. 当前轮：不限量 VLM 同步分析，不计入 X 预算。
    for (_, (msg_idx, urls)) in all_image_msgs.iter().enumerate() {
        if Some(*msg_idx) != current_round_msg_idx {
            continue;
        }
        let mut round_urls: Vec<String> = Vec::new();
        for url in urls {
            let key = url_hash(url);
            if let Some(cached) = cache_get(&key) {
                descriptions
                    .entry(*msg_idx)
                    .or_default()
                    .push_str(&format!("\n[图片描述] {cached}"));
            } else {
                round_urls.push(url.clone());
            }
        }
        if analyze_and_inject(&round_urls, vlm_config, &mut descriptions, *msg_idx)
            .await
            .is_err()
        {
            // 当前轮 VLM 全部失败 → fail-closed。
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "vlm_current_round_fail_closed",
                json!({
                    "round_url_count": round_urls.len(),
                    "is_current": true,
                }),
            );
            return;
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
        let _ = analyze_and_inject(&round_urls, vlm_config, &mut descriptions, *msg_idx).await;
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
    let bg_config_opt = if x_budget > 10 {
        let bg_target = x_budget.saturating_sub(golden_injected);
        if bg_target > 0 {
            let mut bg_urls: Vec<String> = Vec::new();
            // 6a. 黄金窗口中未被 Phase 1 覆盖的未缓存 URL（N > cap 场景）。
            for (_, (msg_idx, urls)) in all_image_msgs.iter().enumerate() {
                if Some(*msg_idx) == current_round_msg_idx || *msg_idx < golden_user_cutoff {
                    continue;
                }
                if bg_urls.len() >= bg_target {
                    break;
                }
                for url in urls {
                    if bg_urls.len() >= bg_target {
                        break;
                    }
                    let key = url_hash(url);
                    if !cache_contains(&key) {
                        bg_urls.push(url.clone());
                    }
                }
            }
            // 6b. 深层历史中未缓存的 URL（N ≤ X 场景，推进到 50 轮边界）。
            if bg_urls.len() < bg_target {
                for (msg_idx, urls) in all_image_msgs.iter() {
                    if Some(*msg_idx) == current_round_msg_idx || *msg_idx >= golden_user_cutoff {
                        continue;
                    }
                    if bg_urls.len() >= bg_target {
                        break;
                    }
                    for url in urls {
                        if bg_urls.len() >= bg_target {
                            break;
                        }
                        let key = url_hash(url);
                        if !cache_contains(&key) {
                            bg_urls.push(url.clone());
                        }
                    }
                }
            }
            if !bg_urls.is_empty() {
                Some((vlm_config.clone(), bg_urls))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

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
            *desc = desc.chars().take(keep.max(1)).collect::<String>() + "\n[历史图片描述已省略]";
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

    // 10. Phase 2 后台：异步分析未缓存图片写入缓存（X > 10 时触发）。
    if let Some((bg_config, bg_urls)) = bg_config_opt {
        tokio::spawn(async move {
            let _ = background_analyze_and_cache(&bg_urls, &bg_config).await;
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "vlm_phase2_done",
                json!({
                    "urls_analyzed": bg_urls.len(),
                }),
            );
        });
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── is_retryable ──────────────────────────────────────────────

    #[test]
    fn is_retryable_detects_timeout() {
        assert!(is_retryable("request failed: timeout"));
        assert!(is_retryable("connection TIMEOUT"));
    }

    #[test]
    fn is_retryable_detects_5xx_and_429() {
        assert!(is_retryable("VLM API 502 Bad Gateway: ..."));
        assert!(is_retryable("VLM API 503 Service Unavailable"));
        assert!(is_retryable("VLM API 504 Gateway Timeout"));
        assert!(is_retryable("VLM API 429 Too Many Requests"));
    }

    #[test]
    fn is_retryable_rejects_4xx() {
        assert!(!is_retryable("VLM API 401 Unauthorized"));
        assert!(!is_retryable("VLM API 400 Bad Request"));
        assert!(!is_retryable("VLM API 404 Not Found"));
    }

    #[test]
    fn is_retryable_rejects_other_errors() {
        assert!(!is_retryable("parse failed: invalid json"));
        assert!(!is_retryable("no content"));
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

    // ── cache eviction ────────────────────────────────────────────

    #[test]
    fn cache_insert_evicts_oldest_when_full() {
        // 填满缓存（500 条）后继续插入会触发驱逐。
        for i in 0..MAX_CACHE_CAPACITY {
            cache_insert(format!("evict-test-{i:04x}"), format!("desc-{i}"));
        }
        // 确认第 0 条仍在
        assert!(cache_contains("evict-test-0000"));
        // 插入第 501 条 → 触发驱逐（删最旧的 125 条）
        cache_insert(
            "evict-test-overflow".to_string(),
            "overflow-desc".to_string(),
        );
        // 最旧的应已被驱逐
        assert!(!cache_contains("evict-test-0000"));
        // 新插入的存在
        assert!(cache_contains("evict-test-overflow"));
    }

    // ── strip_image_blocks (tokio::test) ──────────────────────────

    #[tokio::test]
    async fn strip_image_blocks_all_cache_hits_no_vlm_call() {
        // 预填充缓存
        cache_insert(
            url_hash("https://test.example.com/cached.png"),
            "缓存的图片描述".to_string(),
        );

        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "看这张图"},
                {"type": "image_url", "image_url": {"url": "https://test.example.com/cached.png"}},
            ]
        })];

        let vlm_config = VlmConfig {
            api_key: String::new(),
            model: String::new(),
            base_url: String::new(),
        };

        strip_image_blocks(&mut messages, &vlm_config, "{}", "272000", "gpt-4").await;

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
        };

        strip_image_blocks(
            &mut messages,
            &vlm_config,
            "{}",
            "1", // 上下文窗口 = 1 token → 必然溢出
            "gpt-4",
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
        };

        strip_image_blocks(&mut messages, &vlm_config, "{}", "272000", "gpt-4").await;

        // 消息应保持不变
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "just text");
    }

    #[tokio::test]
    async fn strip_image_blocks_unanalyzed_gets_placeholder() {
        // 历史消息有大量图片但 VLM 服务不可达 → fail-closed → 图片保留但不注入占位符
        // 注：VLM 服务不可达时 analyze_all 会返回 Err → strip_image_blocks 会 early return
        // 所以这里验证的是：当 VLM 完全不可用时，图片不被删除。
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
        };

        strip_image_blocks(&mut messages, &vlm_config, "{}", "272000", "gpt-4").await;

        // fail-closed：图片保留
        let parts = messages[0]["content"].as_array().unwrap();
        let has_image = parts
            .iter()
            .any(|p| p.get("type").and_then(Value::as_str) == Some("image_url"));
        assert!(
            has_image,
            "image should be preserved when VLM is unreachable (fail-closed)"
        );
    }

    /// 混合缓存命中/未命中 + VLM 不可达 → fail-closed。
    /// 前 8 张在缓存中，后 7 张不在，VLM 不可达时全部图片保留。
    #[tokio::test]
    async fn strip_image_blocks_mixed_cache_vlm_unreachable_fail_closed() {
        let mut messages: Vec<Value> = Vec::new();
        // 当前轮消息（1 张图）
        messages.push(serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "current"},
                {"type": "image_url", "image_url": {"url": "https://test.example.com/limit/current.png"}},
            ]
        }));
        // 历史消息（15 张图，模拟一条大消息）
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
            cache_insert(
                url_hash(&format!("https://test.example.com/limit/hist-{i}.png")),
                format!("hist-desc-{i}"),
            );
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": history_parts
        }));

        let vlm_config = VlmConfig {
            api_key: "unused".to_string(),
            model: "unused".to_string(),
            base_url: "https://127.0.0.1:1".to_string(), // VLM 不可达，触发 fail-closed 路径
        };

        strip_image_blocks(&mut messages, &vlm_config, "{}", "900000", "gpt-4").await;

        // VLM 不可达 → analyze_all 返回 Err → strip_image_blocks early return
        // → fail-closed：全部图片保留，不注入任何描述。
        let hist_parts = messages[1]["content"].as_array().unwrap();
        let image_count = hist_parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("image_url"))
            .count();
        assert_eq!(
            image_count, 15,
            "images preserved (fail-closed when VLM unreachable)"
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
                cache_insert(
                    url_hash(&format!("https://multi.example.com/r{round}-i{img}.png")),
                    format!("round{round}-img{img}-desc"),
                );
            }
        }

        let mut messages: Vec<Value> = (0..ROUNDS)
            .map(|round| {
                let mut parts: Vec<Value> = vec![
                    serde_json::json!({"type": "text", "text": format!("round {round}")})
                ];
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
        };

        strip_image_blocks(&mut messages, &vlm_config, "{}", "900000", "gpt-4").await;

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
        // round 23 最先处理（最新优先），10 张后达到 cap，rounds 22-15 无注入
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
            cache_insert(
                url_hash(&format!("https://tight.example.com/hist-{i}.png")),
                format!("hist-desc-{i}"),
            );
        }
        cache_insert(
            url_hash("https://tight.example.com/curr.png"),
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
        };

        strip_image_blocks(&mut messages, &vlm_config, "{}", "800", "gpt-4").await;

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

    // ── wiremock integration tests ───────────────────────────────

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// 真实 HTTP：mock VLM 返回 200，验证 analyze_all 拿到描述。
    #[tokio::test]
    async fn analyze_all_success_with_mock_vlm() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "mock: a beautiful sunset"}}]
            })))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
        };

        let result = analyze_all(&["https://example.com/img.png".to_string()], &config).await;

        let descriptions = result.expect("should succeed");
        assert_eq!(descriptions.len(), 1);
        assert_eq!(descriptions[0].as_deref(), Some("mock: a beautiful sunset"));
    }

    /// 全部失败：mock VLM 返回 500 → fail-closed。
    #[tokio::test]
    async fn analyze_all_all_fail_with_mock_vlm() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
        };

        let result = analyze_all(&["https://example.com/img.png".to_string()], &config).await;

        assert!(result.is_err(), "all 500 should return Err (fail-closed)");
    }

    /// 部分失败：仅 1 次 200，其余 500 → Ok + Some/None 混合。
    #[tokio::test]
    async fn analyze_all_partial_failure_with_mock_vlm() {
        let mock_server = MockServer::start().await;

        // 第一次请求返回 200（up_to_n_times 限制匹配次数）
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "mock: first success"}}]
            })))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        // 后续请求返回 500
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
        };

        // BATCH_SIZE=5，6 个 URL → 2 个 batch
        let urls: Vec<String> = (0..6)
            .map(|i| format!("https://example.com/partial/img-{i}.png"))
            .collect();
        let result = analyze_all(&urls, &config).await;

        let descriptions = result.expect("partial failure should return Ok");
        assert_eq!(descriptions.len(), 2);
        // 并发 batch 顺序非确定性，用计数代替下标检查避免竞态条件
        let some_count = descriptions.iter().filter(|d| d.is_some()).count();
        let none_count = descriptions.iter().filter(|d| d.is_none()).count();
        assert_eq!(some_count, 1, "one batch should succeed");
        assert_eq!(none_count, 1, "one batch should fail");
    }

    /// strip_image_blocks 端到端：mock VLM 可用时正常注入描述。
    #[tokio::test]
    async fn strip_image_blocks_with_mock_vlm_injects_descriptions() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "mock: E2E network call"}}]
            })))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
        };

        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "describe this"},
                {"type": "image_url", "image_url": {"url": "https://wiremock-e2e.example.com/img.png"}},
            ]
        })];

        strip_image_blocks(&mut messages, &config, "{}", "272000", "gpt-4").await;

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

    /// 超时 + 重试：mock 延迟 3s > test timeout(2s) → 超时重试后 500 → fail-closed。
    #[tokio::test]
    async fn analyze_all_timeout_and_retry_with_mock_vlm() {
        let mock_server = MockServer::start().await;

        // 第一次请求延迟 3s，触发客户端超时（test timeout=2s）
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({
                        "choices": [{"message": {"content": "this should not arrive"}}]
                    }))
                    .set_delay(Duration::from_secs(3)),
            )
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        // 第一次重试 → 500
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("retry 500"))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        // 第二次重试 → 500
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("retry 500"))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
        };

        let result = analyze_all(&["https://example.com/img.png".to_string()], &config).await;

        assert!(result.is_err(), "timeout + retry exhaust should return Err");
    }

    /// Plain Responses 模式：input_image 类型块 + 直接字符串 image_url。
    /// 验证 strip_image_blocks 正确处理非 CC 格式的图片块。
    #[tokio::test]
    async fn strip_image_blocks_with_responses_format_input_images() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "mock: responses format"}}]
            })))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
        };

        // Responses 格式：input_image 类型，image_url 为直接字符串
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "describe"},
                {"type": "input_image", "image_url": "https://responses.example.com/img.png"},
            ]
        })];

        strip_image_blocks(&mut messages, &config, "{}", "272000", "gpt-4").await;

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

    /// 聚合重试：首次请求 503 触发重试，第二次 200 成功。
    #[tokio::test]
    async fn analyze_all_retry_then_succeed() {
        let mock_server = MockServer::start().await;

        // 第一次请求 → 503（可重试）
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        // 重试 → 200
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "mock: retry success"}}]
            })))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
        };

        let result = analyze_all(&["https://example.com/img.png".to_string()], &config).await;

        let descriptions = result.expect("should succeed after retry");
        assert_eq!(descriptions.len(), 1);
        assert_eq!(
            descriptions[0].as_deref(),
            Some("mock: retry success"),
            "should get result from successful retry"
        );
    }

    /// 多轮历史图片：当前轮 + 历史轮均有图片，VLM 分析两轮并注入各自描述。
    /// 验证历史轮次的图片描述不会错误注入到当前轮。
    #[tokio::test]
    async fn strip_image_blocks_two_rounds_both_vlm_analyzed() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "mock: analyzed"}}]
            })))
            .mount(&mock_server)
            .await;

        let config = VlmConfig {
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: mock_server.uri(),
        };

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

        strip_image_blocks(&mut messages, &config, "{}", "900000", "gpt-4").await;

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
