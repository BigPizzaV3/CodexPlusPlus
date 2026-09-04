use std::time::Duration;

use anyhow::Context;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::settings::BackendSettings;

const MAX_PROMPT_LENGTH: usize = 420;
const MAX_LABEL_LENGTH: usize = 36;
const MAX_SUMMARY_LENGTH: usize = 72;
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepwiseProtocol {
    ChatCompletions,
    Responses,
    AnthropicMessages,
}

impl StepwiseProtocol {
    fn from_setting(value: &str) -> Self {
        match value {
            "responses" => Self::Responses,
            "anthropic_messages" => Self::AnthropicMessages,
            _ => Self::ChatCompletions,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

struct StepwiseUpstreamRequest {
    endpoint: String,
    headers: HeaderMap,
    body: Value,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepwiseRequest {
    #[serde(default)]
    pub last_user_message: String,
    #[serde(default)]
    pub last_assistant_message: String,
    #[serde(default)]
    pub thread_title: String,
    #[serde(default)]
    pub page_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepwiseItem {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepwisePublicSettings {
    pub enabled: bool,
    pub generation_mode: String,
    pub answer_outline_enabled: bool,
    pub direct_send: bool,
    pub base_url_configured: bool,
    pub api_key_configured: bool,
    pub api_key_env: String,
    pub api_key_env_configured: bool,
    pub protocol: String,
    pub model: String,
    pub max_items: u8,
    pub max_input_chars: u32,
    pub max_output_tokens: u32,
    pub timeout_ms: u64,
}

pub fn public_settings(settings: &BackendSettings) -> StepwisePublicSettings {
    StepwisePublicSettings {
        enabled: settings.codex_app_stepwise_enabled,
        generation_mode: settings.codex_app_stepwise_generation_mode.clone(),
        answer_outline_enabled: settings.codex_app_answer_outline_enabled,
        direct_send: settings.codex_app_stepwise_direct_send,
        base_url_configured: !settings.codex_app_stepwise_base_url.trim().is_empty(),
        api_key_configured: !stepwise_api_key(settings).is_empty(),
        api_key_env: settings.codex_app_stepwise_api_key_env.clone(),
        api_key_env_configured: std::env::var(settings.codex_app_stepwise_api_key_env.trim())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        protocol: settings.codex_app_stepwise_protocol.clone(),
        model: settings.codex_app_stepwise_model.clone(),
        max_items: settings.codex_app_stepwise_max_items,
        max_input_chars: settings.codex_app_stepwise_max_input_chars,
        max_output_tokens: settings.codex_app_stepwise_max_output_tokens,
        timeout_ms: settings.codex_app_stepwise_timeout_ms,
    }
}

pub fn settings_with_payload(mut settings: BackendSettings, payload: &Value) -> BackendSettings {
    let Some(raw_settings) = payload.get("settings").and_then(Value::as_object) else {
        return settings;
    };
    if let Some(value) = raw_settings
        .get("codexAppStepwiseEnabled")
        .and_then(Value::as_bool)
    {
        settings.codex_app_stepwise_enabled = value;
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseGenerationMode")
        .and_then(Value::as_str)
    {
        settings.codex_app_stepwise_generation_mode =
            crate::settings::normalize_stepwise_generation_mode(value);
    }
    if let Some(value) = raw_settings
        .get("codexAppAnswerOutlineEnabled")
        .and_then(Value::as_bool)
    {
        settings.codex_app_answer_outline_enabled = value;
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseDirectSend")
        .and_then(Value::as_bool)
    {
        settings.codex_app_stepwise_direct_send = value;
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseBaseUrl")
        .and_then(Value::as_str)
    {
        settings.codex_app_stepwise_base_url = value.trim().trim_end_matches('/').to_string();
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseApiKey")
        .and_then(Value::as_str)
    {
        settings.codex_app_stepwise_api_key = value.trim().to_string();
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseApiKeyEnv")
        .and_then(Value::as_str)
    {
        settings.codex_app_stepwise_api_key_env = if value.trim().is_empty() {
            crate::settings::default_stepwise_api_key_env()
        } else {
            value.trim().to_string()
        };
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseProtocol")
        .and_then(Value::as_str)
    {
        settings.codex_app_stepwise_protocol = crate::settings::normalize_stepwise_protocol(value);
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseModel")
        .and_then(Value::as_str)
    {
        settings.codex_app_stepwise_model = value.trim().to_string();
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseMaxItems")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
    {
        settings.codex_app_stepwise_max_items = crate::settings::clamp_stepwise_max_items(value);
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseMaxInputChars")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    {
        settings.codex_app_stepwise_max_input_chars =
            crate::settings::clamp_stepwise_max_input_chars(value);
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseMaxOutputTokens")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    {
        settings.codex_app_stepwise_max_output_tokens =
            crate::settings::clamp_stepwise_max_output_tokens(value);
    }
    if let Some(value) = raw_settings
        .get("codexAppStepwiseTimeoutMs")
        .and_then(Value::as_u64)
    {
        settings.codex_app_stepwise_timeout_ms = crate::settings::clamp_stepwise_timeout_ms(value);
    }
    settings
}

pub async fn generate(
    request: StepwiseRequest,
    settings: &BackendSettings,
) -> anyhow::Result<Value> {
    let configured_protocol =
        crate::settings::normalize_stepwise_protocol(&settings.codex_app_stepwise_protocol);
    if !settings.codex_app_stepwise_enabled {
        return Ok(json!({
            "status": "ok",
            "disabled": true,
            "protocol": configured_protocol,
            "items": []
        }));
    }

    let base_url = settings
        .codex_app_stepwise_base_url
        .trim()
        .trim_end_matches('/');
    let api_key = stepwise_api_key(settings);
    let model = settings.codex_app_stepwise_model.trim();
    let max_items = settings.codex_app_stepwise_max_items;

    if max_items == 0 {
        return Ok(json!({
            "status": "ok",
            "protocol": configured_protocol,
            "items": []
        }));
    }
    if base_url.is_empty() || model.is_empty() {
        return Ok(failed_result(
            &configured_protocol,
            "Stepwise Base URL or Model is not configured",
        ));
    }
    if api_key.is_empty() {
        return Ok(failed_result(
            &configured_protocol,
            "Stepwise API Key is not configured",
        ));
    }

    let client = crate::http_client::proxied_client("")?;
    let timeout = Duration::from_millis(settings.codex_app_stepwise_timeout_ms);
    let protocols = stepwise_protocols(&configured_protocol);
    let auto_protocol = configured_protocol == "auto";
    let mut protocol_errors = Vec::new();

    for (index, protocol) in protocols.iter().copied().enumerate() {
        let has_next_protocol = index + 1 < protocols.len();
        let upstream =
            match build_upstream_request(protocol, base_url, &api_key, model, &request, settings) {
                Ok(upstream) => upstream,
                Err(error) => {
                    return Ok(failed_result(
                        protocol.as_str(),
                        format!(
                            "failed to build Stepwise {} request: {error}",
                            protocol.as_str()
                        ),
                    ));
                }
            };
        let response = match client
            .post(&upstream.endpoint)
            .headers(upstream.headers)
            .timeout(timeout)
            .json(&upstream.body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return Ok(failed_result(
                    protocol.as_str(),
                    format!(
                        "failed to request Stepwise {} API: {error}",
                        protocol.as_str()
                    ),
                ));
            }
        };

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if auto_protocol
            && matches!(
                status,
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            )
        {
            protocol_errors.push(format!(
                "{} returned upstream {}",
                protocol.as_str(),
                status.as_u16()
            ));
            if has_next_protocol {
                continue;
            }
            break;
        }
        if !status.is_success() {
            return Ok(failed_result(
                protocol.as_str(),
                format!(
                    "Stepwise upstream {}: {}",
                    status.as_u16(),
                    redact_secret(&text, &api_key)
                ),
            ));
        }

        let data: Value = match serde_json::from_str(&text) {
            Ok(data) => data,
            Err(error) => {
                if auto_protocol {
                    protocol_errors.push(format!(
                        "{} returned invalid JSON: {error}",
                        protocol.as_str()
                    ));
                    if has_next_protocol {
                        continue;
                    }
                    break;
                }
                return Ok(failed_result(
                    protocol.as_str(),
                    format!("failed to parse Stepwise API response: {error}"),
                ));
            }
        };
        if auto_protocol && !matches_stepwise_protocol_response(protocol, &data) {
            protocol_errors.push(format!(
                "{} returned an incompatible response shape",
                protocol.as_str()
            ));
            if has_next_protocol {
                continue;
            }
            break;
        }
        return Ok(json!({
            "status": "ok",
            "protocol": protocol.as_str(),
            "items": extract_stepwise_items(&data, max_items)
        }));
    }

    let details = if protocol_errors.is_empty() {
        String::new()
    } else {
        format!(": {}", protocol_errors.join("; "))
    };
    Ok(failed_result(
        &configured_protocol,
        format!("Stepwise could not find a supported upstream protocol{details}"),
    ))
}

fn stepwise_protocols(value: &str) -> Vec<StepwiseProtocol> {
    if value == "auto" {
        vec![
            StepwiseProtocol::ChatCompletions,
            StepwiseProtocol::Responses,
            StepwiseProtocol::AnthropicMessages,
        ]
    } else {
        vec![StepwiseProtocol::from_setting(value)]
    }
}

fn matches_stepwise_protocol_response(protocol: StepwiseProtocol, data: &Value) -> bool {
    if stepwise_items_value(data).is_some() {
        return true;
    }
    match protocol {
        StepwiseProtocol::ChatCompletions => data.get("choices").is_some_and(Value::is_array),
        StepwiseProtocol::Responses => {
            data.get("output_text").is_some() || data.get("output").is_some_and(Value::is_array)
        }
        StepwiseProtocol::AnthropicMessages => data.get("content").is_some_and(Value::is_array),
    }
}

fn build_upstream_request(
    protocol: StepwiseProtocol,
    base_url: &str,
    api_key: &str,
    model: &str,
    request: &StepwiseRequest,
    settings: &BackendSettings,
) -> anyhow::Result<StepwiseUpstreamRequest> {
    let messages = build_messages(request, settings);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let (endpoint, body) = match protocol {
        StepwiseProtocol::ChatCompletions => {
            insert_bearer_header(&mut headers, api_key)?;
            (
                format!("{base_url}/chat/completions"),
                json!({
                    "model": model,
                    "messages": messages,
                    "temperature": 0.2,
                    "max_tokens": settings.codex_app_stepwise_max_output_tokens,
                    "response_format": { "type": "json_object" },
                }),
            )
        }
        StepwiseProtocol::Responses => {
            insert_bearer_header(&mut headers, api_key)?;
            (
                format!("{base_url}/responses"),
                json!({
                    "model": model,
                    "input": messages,
                    "max_output_tokens": settings.codex_app_stepwise_max_output_tokens,
                }),
            )
        }
        StepwiseProtocol::AnthropicMessages => {
            headers.insert(
                HeaderName::from_static("x-api-key"),
                HeaderValue::from_str(api_key)
                    .context("failed to build Stepwise API key header")?,
            );
            headers.insert(
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_static(ANTHROPIC_VERSION),
            );
            let system = messages
                .first()
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let messages = messages
                .into_iter()
                .filter(|message| message.get("role").and_then(Value::as_str) != Some("system"))
                .collect::<Vec<_>>();
            (
                format!("{base_url}/messages"),
                json!({
                    "model": model,
                    "system": system,
                    "messages": messages,
                    "max_tokens": settings.codex_app_stepwise_max_output_tokens,
                }),
            )
        }
    };

    Ok(StepwiseUpstreamRequest {
        endpoint,
        headers,
        body,
    })
}

fn insert_bearer_header(headers: &mut HeaderMap, api_key: &str) -> anyhow::Result<()> {
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}"))
            .context("failed to build Stepwise authorization header")?,
    );
    Ok(())
}

fn failed_result(protocol: &str, error: impl Into<String>) -> Value {
    json!({
        "status": "failed",
        "protocol": protocol,
        "items": [],
        "error": error.into()
    })
}

fn redact_secret(value: &str, secret: &str) -> String {
    let secret = secret.trim();
    let value = if secret.is_empty() {
        value.to_string()
    } else {
        value.replace(secret, "[redacted]")
    };
    value.chars().take(240).collect()
}

pub async fn test_connection(settings: &BackendSettings) -> anyhow::Result<Value> {
    generate(
        StepwiseRequest {
            last_user_message: "测试 Stepwise 配置。".to_string(),
            last_assistant_message: "Stepwise 应返回 0 到 6 条可直接发送的后续建议。".to_string(),
            thread_title: "Codex++ Stepwise test".to_string(),
            page_url: String::new(),
        },
        settings,
    )
    .await
}

pub fn build_messages(request: &StepwiseRequest, settings: &BackendSettings) -> Vec<Value> {
    let limit = settings.codex_app_stepwise_max_input_chars as usize;
    let last_user_message = short_text(&request.last_user_message, limit.saturating_mul(35) / 100);
    let last_assistant_message = short_text(
        &request.last_assistant_message,
        limit.saturating_mul(60) / 100,
    );
    let system_content = [
        "You generate concise Codex Stepwise actions.",
        "Return strict JSON only, no markdown.",
        "Schema: {\"items\":[{\"label\":\"short action name\",\"summary\":\"one concise preview sentence\",\"prompt\":\"complete user message\"}]}",
        &format!(
            "Generate 1 to {} items when the assistant result is non-empty.",
            settings.codex_app_stepwise_max_items
        ),
        "Every prompt must be directly sendable by the user.",
        "Keep label compact and summary within 72 characters when natural; prompt may be detailed and must not omit necessary context.",
        "Use the latest user intent and assistant result. Avoid generic filler.",
        "Order items by expected usefulness.",
        "The first item must be the single most recommended next step for the user.",
        "Prioritize unresolved user intent first, useful verification second, and optional improvements or exploration last.",
        "Each item must represent a meaningfully different direction. Do not return duplicates, paraphrases, or near-duplicates.",
        "Language policy: infer the dominant natural language from lastUserMessage, falling back to lastAssistantMessage, and write Stepwise prompts in that language.",
        "Ignore technical terms, file names, commands, APIs, and product names when detecting language; keep them in their original language when natural.",
        "Return {\"items\":[]} only when both the user intent and assistant result are empty or unusable.",
    ]
    .join("\n");
    vec![
        json!({
            "role": "system",
            "content": system_content
        }),
        json!({
            "role": "user",
            "content": json!({
                "lastUserMessage": last_user_message,
                "lastAssistantMessage": last_assistant_message,
                "threadTitle": short_text(&request.thread_title, 240),
                "pageUrl": short_text(&request.page_url, 240),
                "maxItems": settings.codex_app_stepwise_max_items,
            }).to_string()
        }),
    ]
}

pub fn clamp_items(value: Value, max_items: u8) -> Vec<StepwiseItem> {
    let mut seen = std::collections::BTreeSet::new();
    let mut items = Vec::new();
    let max_items = usize::from(max_items);
    let Some(raw_items) = value.as_array() else {
        return items;
    };

    for raw in raw_items {
        let prompt = first_string_field(raw, &["prompt", "text", "action", "content", "message"])
            .or_else(|| raw.as_str())
            .unwrap_or("");
        let prompt = normalize_text(prompt);
        let dedupe_key = normalize_spaces(&prompt);
        if prompt.is_empty() || seen.contains(&dedupe_key) {
            continue;
        }
        seen.insert(dedupe_key);
        let label = first_string_field(raw, &["label", "title", "name"])
            .map(normalize_spaces)
            .unwrap_or_default();
        let summary = first_string_field(raw, &["summary", "preview", "description"])
            .map(normalize_spaces)
            .filter(|summary| !summary.is_empty())
            .unwrap_or_else(|| summary_for_prompt(&prompt));
        items.push(StepwiseItem {
            label: leading_text(&label, MAX_LABEL_LENGTH),
            summary: leading_text(&summary, MAX_SUMMARY_LENGTH),
            prompt,
        });
        if items.len() >= max_items {
            break;
        }
    }

    items
}

pub fn extract_stepwise_items(data: &Value, max_items: u8) -> Vec<StepwiseItem> {
    for candidate in stepwise_payload_candidates(data) {
        if let Some(items) = stepwise_items_value(&candidate) {
            let items = clamp_items(items, max_items);
            if !items.is_empty() {
                return items;
            }
        }
    }
    Vec::new()
}

fn stepwise_payload_candidates(data: &Value) -> Vec<Value> {
    let mut candidates = vec![data.clone()];

    if let Some(content) = data
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
    {
        if let Some(parts) = content.as_array() {
            for part in parts {
                if let Some(text) = part.get("text") {
                    push_payload_candidate(&mut candidates, text);
                }
            }
        } else {
            push_payload_candidate(&mut candidates, content);
        }
    }

    if let Some(output_text) = data.get("output_text") {
        push_payload_candidate(&mut candidates, output_text);
    }

    if let Some(output) = data.get("output").and_then(Value::as_array) {
        for item in output {
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for part in content {
                    if let Some(text) = part.get("text") {
                        push_payload_candidate(&mut candidates, text);
                    }
                }
            }
        }
    }

    if let Some(content) = data.get("content").and_then(Value::as_array) {
        for part in content {
            if let Some(text) = part.get("text") {
                push_payload_candidate(&mut candidates, text);
            }
        }
    }

    for key in ["output", "response", "data", "result"] {
        if let Some(value) = data.get(key) {
            push_payload_candidate(&mut candidates, value);
        }
    }

    candidates
}

fn push_payload_candidate(candidates: &mut Vec<Value>, value: &Value) {
    candidates.push(value.clone());
    if let Some(parsed) = parse_json_value(value) {
        candidates.push(parsed);
    }
}

fn stepwise_items_value(value: &Value) -> Option<Value> {
    if value.as_array().is_some() {
        return Some(value.clone());
    }
    for key in [
        "items",
        "suggestions",
        "next_steps",
        "nextSteps",
        "actions",
        "prompts",
    ] {
        if let Some(items) = value.get(key).filter(|items| items.as_array().is_some()) {
            return Some(items.clone());
        }
    }
    None
}

fn parse_json_value(value: &Value) -> Option<Value> {
    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    serde_json::from_str(text).ok()
}

fn first_string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return Some(text);
        }
    }
    None
}

fn stepwise_api_key(settings: &BackendSettings) -> String {
    let direct = settings.codex_app_stepwise_api_key.trim();
    if !direct.is_empty() {
        return direct.to_string();
    }
    std::env::var(settings.codex_app_stepwise_api_key_env.trim())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn short_text(value: &str, limit: usize) -> String {
    let text = normalize_text(value);
    if text.chars().count() <= limit {
        return text;
    }
    text.chars()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn normalize_text(value: &str) -> String {
    value
        .replace('\u{a0}', " ")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .split("\n\n\n")
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

fn normalize_spaces(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn leading_text(value: &str, limit: usize) -> String {
    let text = normalize_text(value);
    if text.chars().count() <= limit {
        return text;
    }
    let mut result = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}

fn summary_for_prompt(prompt: &str) -> String {
    leading_text(&normalize_spaces(prompt), MAX_SUMMARY_LENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_API_KEY: &str = "sk-stepwise-test";

    fn test_request() -> StepwiseRequest {
        StepwiseRequest {
            last_user_message: "请继续检查协议兼容性。".to_string(),
            last_assistant_message: "已完成基础实现。".to_string(),
            thread_title: "协议兼容测试".to_string(),
            page_url: "https://example.test/thread".to_string(),
        }
    }

    fn test_settings(base_url: String, protocol: &str) -> BackendSettings {
        BackendSettings {
            codex_app_stepwise_enabled: true,
            codex_app_stepwise_base_url: base_url,
            codex_app_stepwise_api_key: TEST_API_KEY.to_string(),
            codex_app_stepwise_protocol: protocol.to_string(),
            codex_app_stepwise_model: "stepwise-test".to_string(),
            codex_app_stepwise_timeout_ms: 2000,
            ..BackendSettings::default()
        }
    }

    #[test]
    fn clamp_items_dedupes_and_limits() {
        let items = clamp_items(
            json!([
                {"label": "继续", "summary": "检查当前失败路径", "prompt": "继续排查\n并保留换行"},
                {"label": "重复", "prompt": "继续排查 并保留换行"},
                {"prompt": "补测试"},
                "更新文档"
            ]),
            2,
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "继续");
        assert_eq!(items[0].summary, "检查当前失败路径");
        assert_eq!(items[0].prompt, "继续排查\n并保留换行");
        assert_eq!(items[1].summary, "补测试");
        assert_eq!(items[1].prompt, "补测试");
    }

    #[test]
    fn clamp_items_keeps_long_prompt_and_backfills_summary() {
        let long_prompt = format!("第一段\n\n{}", "完整上下文".repeat(120));
        let items = clamp_items(json!([{"prompt": long_prompt}]), 6);

        assert_eq!(items.len(), 1);
        assert!(items[0].prompt.starts_with("第一段\n\n完整上下文"));
        assert!(items[0].prompt.chars().count() > 420);
        assert!(!items[0].summary.is_empty());
        assert!(items[0].summary.chars().count() <= MAX_SUMMARY_LENGTH);
    }

    #[test]
    fn extracts_items_from_common_stepwise_response_shapes() {
        let response = json!({
            "choices": [{
                "message": {
                    "content": "{\"suggestions\":[{\"title\":\"继续排查\",\"text\":\"请继续检查 Stepwise 返回内容\"},{\"action\":\"补一个解析测试\"}]}"
                }
            }]
        });

        let items = extract_stepwise_items(&response, 6);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "继续排查");
        assert_eq!(items[0].summary, "请继续检查 Stepwise 返回内容");
        assert_eq!(items[0].prompt, "请继续检查 Stepwise 返回内容");
        assert_eq!(items[1].prompt, "补一个解析测试");
    }

    #[test]
    fn extracts_items_from_chat_completions_text_blocks() {
        let response = json!({
            "choices": [{
                "message": {
                    "content": [{
                        "type": "text",
                        "text": "{\"items\":[{\"prompt\":\"解析文本块里的建议\"}]}"
                    }]
                }
            }]
        });

        let items = extract_stepwise_items(&response, 6);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].prompt, "解析文本块里的建议");
    }

    #[test]
    fn prompt_infers_language_without_duplicate_input() {
        let settings = BackendSettings {
            codex_app_stepwise_max_items: 4,
            ..BackendSettings::default()
        };
        let messages = build_messages(
            &StepwiseRequest {
                last_user_message: "请补一个 directSend selftest，覆盖 ProseMirror。".to_string(),
                last_assistant_message: "已完成实现。".to_string(),
                thread_title: String::new(),
                page_url: String::new(),
            },
            &settings,
        );
        let system = messages[0].get("content").and_then(Value::as_str).unwrap();
        let user = messages[1].get("content").and_then(Value::as_str).unwrap();
        let user_payload: Value = serde_json::from_str(user).unwrap();

        assert!(system.contains("dominant natural language"));
        assert!(system.contains("lastUserMessage"));
        assert!(system.contains("falling back to lastAssistantMessage"));
        assert!(system.contains("Generate 1 to 4 items when the assistant result is non-empty."));
        assert!(system.contains("summary within 72 characters"));
        assert!(system.contains("prompt may be detailed"));
        assert!(system.contains("Order items by expected usefulness."));
        assert!(system.contains("The first item must be the single most recommended next step"));
        assert!(system.contains("Do not return duplicates, paraphrases, or near-duplicates."));
        assert_eq!(
            user_payload["lastUserMessage"],
            "请补一个 directSend selftest，覆盖 ProseMirror。"
        );
        assert_eq!(user_payload["lastAssistantMessage"], "已完成实现。");
        assert!(user_payload.get("languageInput").is_none());
    }

    #[test]
    fn settings_with_payload_clamps_values() {
        let settings = settings_with_payload(
            BackendSettings::default(),
            &json!({
                "settings": {
                    "codexAppStepwiseEnabled": true,
                    "codexAppStepwiseGenerationMode": "manual",
                    "codexAppAnswerOutlineEnabled": false,
                    "codexAppStepwiseDirectSend": true,
                    "codexAppStepwiseBaseUrl": "https://api.example.test/v1/",
                    "codexAppStepwiseApiKey": " sk-test ",
                    "codexAppStepwiseApiKeyEnv": "",
                    "codexAppStepwiseProtocol": "responses",
                    "codexAppStepwiseModel": " stepwise-mini ",
                    "codexAppStepwiseMaxItems": 9,
                    "codexAppStepwiseMaxInputChars": 999999,
                    "codexAppStepwiseMaxOutputTokens": 10,
                    "codexAppStepwiseTimeoutMs": 999999
                }
            }),
        );

        assert!(settings.codex_app_stepwise_enabled);
        assert_eq!(settings.codex_app_stepwise_generation_mode, "manual");
        assert!(!settings.codex_app_answer_outline_enabled);
        assert!(settings.codex_app_stepwise_direct_send);
        assert_eq!(
            settings.codex_app_stepwise_base_url,
            "https://api.example.test/v1"
        );
        assert_eq!(settings.codex_app_stepwise_api_key, "sk-test");
        assert_eq!(
            settings.codex_app_stepwise_api_key_env,
            crate::settings::default_stepwise_api_key_env()
        );
        assert_eq!(settings.codex_app_stepwise_protocol, "responses");
        assert_eq!(settings.codex_app_stepwise_model, "stepwise-mini");
        assert_eq!(settings.codex_app_stepwise_max_items, 6);
        assert_eq!(settings.codex_app_stepwise_max_input_chars, 24000);
        assert_eq!(settings.codex_app_stepwise_max_output_tokens, 100);
        assert_eq!(settings.codex_app_stepwise_timeout_ms, 60000);
    }

    #[tokio::test]
    async fn generate_uses_chat_completions_protocol_and_parses_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": {
                        "content": "{\"items\":[{\"label\":\"继续\",\"prompt\":\"继续检查\"}]}"
                    }
                }]
            })))
            .mount(&server)
            .await;

        let result = generate(
            test_request(),
            &test_settings(server.uri(), "chat_completions"),
        )
        .await
        .unwrap();

        assert_eq!(result["status"], "ok");
        assert_eq!(result["protocol"], "chat_completions");
        assert_eq!(result["items"][0]["label"], "继续");
        assert_eq!(result["items"][0]["prompt"], "继续检查");

        let requests = server.received_requests().await.unwrap();
        let request = &requests[0];
        assert_eq!(
            request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer sk-stepwise-test")
        );
        let body: Value = request.body_json().unwrap();
        assert_eq!(body["model"], "stepwise-test");
        assert_eq!(body["response_format"]["type"], "json_object");
        assert!(body["messages"].is_array());
    }

    #[tokio::test]
    async fn generate_uses_responses_protocol_and_parses_output_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output_text": "{\"items\":[{\"prompt\":\"检查 Responses 接口\"}]}"
            })))
            .mount(&server)
            .await;

        let result = generate(test_request(), &test_settings(server.uri(), "responses"))
            .await
            .unwrap();

        assert_eq!(result["status"], "ok");
        assert_eq!(result["protocol"], "responses");
        assert_eq!(result["items"][0]["prompt"], "检查 Responses 接口");

        let requests = server.received_requests().await.unwrap();
        let request = &requests[0];
        assert_eq!(
            request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer sk-stepwise-test")
        );
        let body: Value = request.body_json().unwrap();
        assert_eq!(body["model"], "stepwise-test");
        assert!(body["input"].is_array());
        assert_eq!(body["max_output_tokens"], 500);
    }

    #[tokio::test]
    async fn generate_uses_anthropic_messages_protocol_and_parses_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{
                    "type": "text",
                    "text": "{\"items\":[{\"prompt\":\"检查 Anthropic Messages 接口\"}]}"
                }]
            })))
            .mount(&server)
            .await;

        let result = generate(
            test_request(),
            &test_settings(server.uri(), "anthropic_messages"),
        )
        .await
        .unwrap();

        assert_eq!(result["status"], "ok");
        assert_eq!(result["protocol"], "anthropic_messages");
        assert_eq!(result["items"][0]["prompt"], "检查 Anthropic Messages 接口");

        let requests = server.received_requests().await.unwrap();
        let request = &requests[0];
        assert_eq!(
            request
                .headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some(TEST_API_KEY)
        );
        assert_eq!(
            request
                .headers
                .get("anthropic-version")
                .and_then(|value| value.to_str().ok()),
            Some("2023-06-01")
        );
        assert!(request.headers.get("authorization").is_none());
        let body: Value = request.body_json().unwrap();
        assert_eq!(body["model"], "stepwise-test");
        assert!(
            body["system"]
                .as_str()
                .is_some_and(|value| value.contains("strict JSON"))
        );
        assert!(body["messages"].is_array());
        assert_eq!(body["max_tokens"], 500);
    }

    #[tokio::test]
    async fn auto_protocol_falls_back_on_unsupported_endpoint_statuses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{
                    "type": "text",
                    "text": "{\"items\":[{\"prompt\":\"自动兼容成功\"}]}"
                }]
            })))
            .mount(&server)
            .await;

        let result = generate(test_request(), &test_settings(server.uri(), "auto"))
            .await
            .unwrap();

        assert_eq!(result["status"], "ok");
        assert_eq!(result["protocol"], "anthropic_messages");
        assert_eq!(result["items"][0]["prompt"], "自动兼容成功");

        let requests = server.received_requests().await.unwrap();
        let paths = requests
            .iter()
            .map(|request| request.url.path())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["/chat/completions", "/responses", "/messages"]);
    }

    #[tokio::test]
    async fn auto_protocol_falls_back_on_success_with_empty_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output_text": "{\"items\":[{\"prompt\":\"Responses 回退成功\"}]}"
            })))
            .mount(&server)
            .await;

        let result = generate(test_request(), &test_settings(server.uri(), "auto"))
            .await
            .unwrap();

        assert_eq!(result["status"], "ok");
        assert_eq!(result["protocol"], "responses");
        assert_eq!(result["items"][0]["prompt"], "Responses 回退成功");

        let requests = server.received_requests().await.unwrap();
        let paths = requests
            .iter()
            .map(|request| request.url.path())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["/chat/completions", "/responses"]);
    }

    #[tokio::test]
    async fn auto_protocol_falls_back_on_incompatible_response_shape() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "unexpected": true
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output_text": "{\"items\":[{\"prompt\":\"协议结构回退成功\"}]}"
            })))
            .mount(&server)
            .await;

        let result = generate(test_request(), &test_settings(server.uri(), "auto"))
            .await
            .unwrap();

        assert_eq!(result["status"], "ok");
        assert_eq!(result["protocol"], "responses");
        assert_eq!(result["items"][0]["prompt"], "协议结构回退成功");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn auto_protocol_does_not_fallback_on_auth_rate_limit_or_server_errors() {
        for status in [401, 403, 429, 500] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_string(format!("upstream rejected {TEST_API_KEY}")),
                )
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/responses"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "output_text": "{\"items\":[{\"prompt\":\"不应被调用\"}]}"
                })))
                .mount(&server)
                .await;

            let result = generate(test_request(), &test_settings(server.uri(), "auto"))
                .await
                .unwrap();

            assert_eq!(result["status"], "failed");
            assert_eq!(result["protocol"], "chat_completions");
            let error = result["error"].as_str().unwrap();
            assert!(error.contains(&status.to_string()));
            assert!(!error.contains(TEST_API_KEY));
            assert!(error.contains("[redacted]"));
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }
}
