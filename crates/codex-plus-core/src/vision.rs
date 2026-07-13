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

use crate::protocol_proxy::{chat_completions_url, model_supports_image, responses_url};
use crate::settings::{RelayProtocol, VisionRelayConfig};

/// 单次请求中 VL 处理的图片数量上限。超出部分直接 strip，不调 VL API。
const VL_IMAGE_LIMIT: usize = 10;
/// 单次 VL API 调用的超时时间。
const VL_SINGLE_TIMEOUT: Duration = Duration::from_secs(30);

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
/// context_window=0 表示不限制，返回全部索引。
fn items_within_vl_window(input: &[Value], context_window: u64) -> Vec<usize> {
    if context_window == 0 {
        return (0..input.len()).collect();
    }
    let mut tokens: u64 = 0;
    let mut indices = vec![];
    for i in (0..input.len()).rev() {
        tokens += estimate_item_tokens(&input[i]) as u64;
        if tokens > context_window {
            break;
        }
        indices.push(i);
    }
    indices.reverse();
    indices
}

/// 从 input items 中收集用户原文（同一条消息里的 input_text）。
fn collect_input_text(input: &[Value]) -> String {
    for item in input.iter().rev() {
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

/// 调用 VL API 描述单张图。
/// 按 vl_config.protocol 适配 image_url 格式和 token 参数名；prompt 由调用方按
/// Tier 1/Tier 2 预计算后传入（Bug 4.8 两 prompt）。
async fn describe_image_with_vl(
    image_url: &str,
    prompt: &str,
    vl_config: &crate::settings::VisionRelayConfig,
    client: &reqwest::Client,
) -> anyhow::Result<String> {
    let body = match vl_config.protocol {
        RelayProtocol::ChatCompletions => json!({
            "model": vl_config.model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": prompt},
                    {"type": "image_url", "image_url": {"url": image_url}}
                ]
            }],
            "max_tokens": vl_config.max_tokens,
        }),
        RelayProtocol::Responses => json!({
            "model": vl_config.model,
            "input": [{
                "role": "user",
                "content": [
                    {"type": "input_text", "text": prompt},
                    {"type": "input_image", "image_url": image_url}
                ]
            }],
            "max_output_tokens": vl_config.max_tokens,
        }),
    };

    let endpoint = match vl_config.protocol {
        RelayProtocol::ChatCompletions => chat_completions_url(&vl_config.base_url),
        RelayProtocol::Responses => responses_url(&vl_config.base_url),
    };

    let response = client
        .post(&endpoint)
        .bearer_auth(&vl_config.api_key)
        .json(&body)
        .timeout(VL_SINGLE_TIMEOUT)
        .send()
        .await?;
    let status = response.status();
    let response_body: Value = response.json().await?;

    if !status.is_success() {
        anyhow::bail!(
            "VL API returned {}: {}",
            status.as_u16(),
            serde_json::to_string(&response_body).unwrap_or_default()
        );
    }

    // 提取文字内容
    let text = match vl_config.protocol {
        RelayProtocol::ChatCompletions => {
            response_body["choices"][0]["message"]["content"]
                .as_str()
                .map(|s| s.to_string())
        }
        RelayProtocol::Responses => {
            response_body["output"][0]["content"][0]["text"]
                .as_str()
                .map(|s| s.to_string())
        }
    };

    text.ok_or_else(|| anyhow::anyhow!("VL API returned no text content"))
}

/// 遍历 input 中的 input_image 块，调 VL API 翻译为文字描述替换为 input_text。
/// context_window=0 不限制窗口；>0 时窗口外的图片直接 strip。
pub async fn analyze_images_with_vl(
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

    // VL 处理窗口内的图（最多 VL_IMAGE_LIMIT 张）
    let mut vl_count = 0;
    for &idx in &window_indices {
        let is_latest = is_latest_message_image(input, idx);
        let Some(item) = input.get_mut(idx) else {
            continue;
        };
        let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts.iter_mut() {
            if part.get("type").and_then(Value::as_str) != Some("input_image") {
                continue;
            }
            let Some(img_url) = extract_image_url(part) else {
                continue;
            };

            vl_count += 1;
            if vl_count > VL_IMAGE_LIMIT {
                // 超出上限：标记为空对象，后续 retain 统一清理
                *part = json!({});
                continue;
            }

            // 两层缓存检测（Bug 4.4/4.8）：
            //   最新消息里的图 + 非空问题 -> Tier 2（(URL,问题) key，含问题 prompt，入口）
            //   其余（历史图 / 无问题最新图）-> Tier 1（URL key，无问题 prompt）
            let (cache_key, prompt): (u64, String) =
                if is_latest && !user_text.is_empty() {
                    (
                        url_question_hash(&img_url, &user_text),
                        tier2_prompt(&user_text),
                    )
                } else {
                    (url_hash(&img_url), TIER1_PROMPT.to_string())
                };

            let description = if let Some(cached) = cache_get(cache_key) {
                cached
            } else {
                let desc =
                    describe_image_with_vl(&img_url, &prompt, vl_config, client).await?;
                cache_put(cache_key, desc.clone());
                desc
            };

            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.vl_described",
                json!({
                    "vlModel": vl_config.model,
                    "image_url_len": img_url.len(),
                    "image_url_is_data": img_url.starts_with("data:"),
                    "description_len": description.len(),
                    "description_chars": description.chars().count()
                }),
            );

            // 替换 input_image -> input_text
            *part = json!({
                "type": "input_text",
                "text": format!("# 图片内容描述\n\n{description}")
            });
        }
        // 清理超限标记的空对象
        parts.retain(|p| !p.as_object().map_or(false, |o| o.is_empty()));
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
