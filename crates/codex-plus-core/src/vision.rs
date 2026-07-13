//! VL 视觉模型中转：纯文本模型图片理解。
//!
//! 当目标模型不支持图片输入时，把请求中的 `input_image` 调 VL（视觉语言）模型
//! 翻译为文字描述，替换为 `input_text` 后再走协议转换。context_window 控制可见
//! 窗口，窗口外的图片直接 strip。VL 失败时降级为 strip，不阻断用户。
//!
//! 本模块从 `protocol_proxy.rs` 抽出（PR #1468 Bug 4.1），行为保持不变；后续
//! 两层缓存 / 并发批次 / 重试 / 超时 / 两 prompt 在此模块内迭代。

use std::time::Duration;

use serde_json::{Value, json};

use crate::protocol_proxy::{chat_completions_url, model_supports_image, responses_url};
use crate::settings::{RelayProtocol, VisionRelayConfig};

/// 单次请求中 VL 处理的图片数量上限。超出部分直接 strip，不调 VL API。
const VL_IMAGE_LIMIT: usize = 10;
/// 单次 VL API 调用的超时时间。
const VL_SINGLE_TIMEOUT: Duration = Duration::from_secs(30);

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
/// 按 vl_config.protocol 适配 image_url 格式和 token 参数名。
async fn describe_image_with_vl(
    image_url: &str,
    user_text: &str,
    vl_config: &crate::settings::VisionRelayConfig,
    client: &reqwest::Client,
) -> anyhow::Result<String> {
    let prompt = if user_text.is_empty() {
        "简要描述这张图片".to_string()
    } else {
        format!(
            "用户想了解：{user_text}\n请根据图片详细描述与用户问题相关的内容，包括文字、UI 元素、错误信息、布局结构。请用中文回复。"
        )
    };

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

            let description = describe_image_with_vl(&img_url, &user_text, vl_config, client).await?;

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
