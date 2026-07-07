/// VLM (Vision Language Model) analysis for text-only models.
/// Batches images into groups, sends each batch as one API call.
use serde_json::Value;

pub struct VlmConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

const BATCH_SIZE: usize = 5;

pub fn should_process(model: &str, model_vlm_json: &str) -> bool {
    let Ok(map) = serde_json::from_str::<std::collections::BTreeMap<String, bool>>(model_vlm_json) else {
        return false;
    };
    map.get(model).copied().unwrap_or(false)
}

/// Collect all image URLs, remove image blocks.
pub fn collect_and_strip(messages: &mut Vec<Value>) -> Vec<String> {
    let mut urls = Vec::new();
    for msg in messages.iter_mut() {
        let Some(content) = msg.get_mut("content") else { continue };
        let Some(parts) = content.as_array_mut() else { continue };
        let mut i = 0;
        while i < parts.len() {
            let kind = parts[i].get("type").and_then(Value::as_str).unwrap_or("");
            if kind == "image_url" || kind == "input_image" {
                if let Some(url) = parts[i].pointer("/image_url/url").or_else(|| parts[i].pointer("/image_url")).and_then(Value::as_str) {
                    if !url.is_empty() { urls.push(url.to_string()); }
                }
                parts.remove(i);
            } else { i += 1; }
        }
    }
    urls
}

/// Call VLM API with a batch of images. Reuses http_client for proxy/timeout.
async fn call_vlm_batch(urls: &[String], config: &VlmConfig) -> Result<String, String> {
    let client = crate::http_client::proxied_client("").map_err(|e| format!("client: {}", e))?;
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut parts: Vec<Value> = urls.iter()
        .map(|u| serde_json::json!({"type": "image_url", "image_url": {"url": u}}))
        .collect();
    parts.push(serde_json::json!({"type": "text", "text": "Describe all images in detail."}));
    let body = serde_json::json!({
        "model": config.model,
        "messages": [{"role": "user", "content": parts}],
        "stream": false, "max_tokens": 2048,
    });
    let resp = client.post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&body).send().await
        .map_err(|e| format!("request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("VLM API {}: {}", resp.status(), resp.text().await.unwrap_or_default()));
    }
    let data: Value = resp.json().await.map_err(|e| format!("parse failed: {}", e))?;
    data["choices"][0]["message"]["content"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "no content".to_string())
}

/// Analyze all images in batches of BATCH_SIZE.
pub async fn analyze_all(urls: &[String], config: &VlmConfig) -> Result<String, String> {
    let mut results: Vec<String> = Vec::new();
    let mut failures: usize = 0;
    for batch in urls.chunks(BATCH_SIZE) {
        match call_vlm_batch(batch, config).await {
            Ok(text) => results.push(text),
            Err(e) => {
                failures += 1;
                results.push(format!("[Batch of {} images: VLM failed - {}]", batch.len(), e));
            }
        }
    }
    if results.is_empty() {
        return Err("all VLM calls failed".to_string());
    }
    if failures > 0 {
        results.push(format!("其中 {} 批图片 Router VLM 调用失败。请在回复中包含 \"Router VLM 调用失败，未能识别图片内容\"", failures));
    }
    Ok(results.join("\n\n---\n\n"))
}

/// Inject analysis result into the last user message.
pub fn inject_analysis(messages: &mut Vec<Value>, result: &Result<String, String>) {
    let text = match result {
        Ok(c) => c.clone(),
        Err(_) => "用户发送了图片，但是 Router VLM 调用失败。请在回复中包含 \"Router VLM 调用失败，未能识别图片内容\"".to_string(),
    };
    for msg in messages.iter_mut().rev() {
        if msg.get("role").and_then(Value::as_str) == Some("user") {
            if let Some(Value::Array(parts)) = msg.get_mut("content") {
                parts.push(serde_json::json!({"type": "text", "text": text}));
            }
            break;
        }
    }
}