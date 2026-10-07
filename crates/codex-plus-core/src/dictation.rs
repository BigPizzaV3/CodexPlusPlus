//! 输入框听写的独立 OpenAI 兼容适配器。音频和密钥不写入诊断日志。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use tokio::sync::watch;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};

use crate::settings::{BackendSettings, DictationSettings};

/// 总开关只控制运行时能力，不修改用户保存的语音开关或独立密钥。
pub(crate) fn effective_settings(settings: &BackendSettings) -> DictationSettings {
    let mut effective = settings.dictation.clone();
    effective.enabled &= settings.enhancements_enabled;
    effective
}

pub const MAX_AUDIO_BODY_BYTES: usize = 25 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const TOKEN_HEADER: &str = "X-Codex-Plus-Dictation-Token";
static HELPER_TOKEN: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

/// 只通过特权 bridge 发给当前应用；公开 HTTP 状态接口不能返回这个 capability。
pub(crate) fn helper_token() -> &'static str {
    &HELPER_TOKEN
}

pub(crate) fn valid_helper_token(token: Option<&str>) -> bool {
    token.is_some_and(|token| token == helper_token())
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct DictationError {
    pub status: u16,
    pub message: String,
}

impl DictationError {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

fn endpoint(settings: &DictationSettings) -> Result<reqwest::Url, DictationError> {
    let url = reqwest::Url::parse(settings.base_url.trim())
        .map_err(|_| DictationError::new(400, "语音服务地址无效"))?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DictationError::new(
            400,
            "语音服务需要 HTTP(S) 地址，不能在 URL 中放凭据或查询参数",
        ));
    }
    reqwest::Url::parse(&crate::protocol_proxy::audio_transcriptions_url(
        url.as_str(),
    ))
    .map_err(|_| DictationError::new(400, "语音服务地址无效"))
}

fn api_key(settings: &DictationSettings) -> Result<String, DictationError> {
    if !settings.api_key.trim().is_empty() {
        return Ok(settings.api_key.trim().to_string());
    }
    let name = settings.api_key_env.trim();
    if name.is_empty() {
        return Ok(String::new());
    }
    if !name.bytes().enumerate().all(|(index, byte)| {
        byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
    }) {
        return Err(DictationError::new(400, "语音 API Key 环境变量名称无效"));
    }
    // 不读取 OPENAI_API_KEY 等隐式默认值，也不回退到编码供应商密钥。
    std::env::var(name)
        .map(|key| key.trim().to_string())
        .map_err(|_| DictationError::new(400, "语音 API Key 环境变量未设置"))
}

fn configuration(settings: &DictationSettings) -> Result<(reqwest::Url, String), DictationError> {
    let url = endpoint(settings)?;
    if settings.model.trim().is_empty()
        || settings.model.len() > 256
        || settings.model.chars().any(char::is_control)
    {
        return Err(DictationError::new(400, "请配置有效的语音转写模型"));
    }
    let key = api_key(settings)?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
    );
    if key.is_empty() && !local {
        return Err(DictationError::new(400, "请配置独立的语音 API Key"));
    }
    if !key.is_empty() && HeaderValue::from_str(&format!("Bearer {key}")).is_err() {
        return Err(DictationError::new(400, "语音 API Key 格式无效"));
    }
    Ok((url, key))
}

pub fn public_status(settings: &DictationSettings) -> Value {
    let provider = endpoint(settings)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_default();
    let result = configuration(settings);
    let error = result.as_ref().err().map(|error| error.message.as_str());
    json!({ "enabled": settings.enabled, "configured": result.is_ok(), "provider": provider, "error": error, "message": error })
}

struct Recording<'a> {
    audio: &'a [u8],
    filename: String,
    mime_type: String,
    language: String,
}

fn parameter(header: &str, key: &str) -> Option<String> {
    header.split(';').skip(1).find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name.trim().eq_ignore_ascii_case(key)).then(|| value.trim().trim_matches('"').to_string())
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|value| value == needle)
}

fn find_part_boundary(haystack: &[u8], delimiter: &[u8]) -> Option<usize> {
    haystack
        .windows(delimiter.len())
        .enumerate()
        .find_map(|(index, value)| {
            if value != delimiter {
                return None;
            }
            let suffix = &haystack[index + delimiter.len()..];
            // 音频内可能出现 boundary 前缀；只有完整的分隔行才结束当前 part。
            (suffix.starts_with(b"\r\n") || suffix == b"--" || suffix.starts_with(b"--\r\n"))
                .then_some(index)
        })
}

fn recording<'a>(body: &'a [u8], content_type: &str) -> Result<Recording<'a>, DictationError> {
    if body.len() > MAX_AUDIO_BODY_BYTES {
        return Err(DictationError::new(413, "语音上传超过 25 MiB 限制"));
    }
    let malformed = || DictationError::new(400, "语音上传需要包含 file 的有效 multipart/form-data");
    if !content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return Err(malformed());
    }
    let boundary = parameter(content_type, "boundary")
        .filter(|boundary| {
            !boundary.is_empty()
                && boundary.len() <= 70
                && boundary
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
        .ok_or_else(malformed)?;
    let delimiter = format!("--{boundary}");
    let mut remaining = body
        .strip_prefix(delimiter.as_bytes())
        .ok_or_else(malformed)?;
    let next_part = format!("\r\n{delimiter}").into_bytes();
    let mut audio = None;
    let mut filename = "dictation.webm".to_string();
    let mut mime_type = "audio/webm".to_string();
    let mut language = String::new();
    let mut parts = 0;
    loop {
        if let Some(tail) = remaining.strip_prefix(b"--") {
            if !tail.is_empty() && tail != b"\r\n" {
                return Err(malformed());
            }
            break;
        }
        parts += 1;
        if parts > 8 {
            return Err(malformed());
        }
        remaining = remaining.strip_prefix(b"\r\n").ok_or_else(malformed)?;
        let headers_end = find_bytes(remaining, b"\r\n\r\n")
            .filter(|length| *length <= 8192)
            .ok_or_else(malformed)?;
        let headers = std::str::from_utf8(&remaining[..headers_end]).map_err(|_| malformed())?;
        let data = &remaining[headers_end + 4..];
        let end = find_part_boundary(data, &next_part).ok_or_else(malformed)?;
        let value = &data[..end];
        let disposition = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-disposition")
                    .then_some(value.trim())
            })
            .ok_or_else(malformed)?;
        match parameter(disposition, "name").as_deref() {
            Some("file") => {
                if audio.is_some() || value.is_empty() {
                    return Err(malformed());
                }
                audio = Some(value);
                if let Some(name) = parameter(disposition, "filename") {
                    let name: String = name
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
                        .take(128)
                        .collect();
                    if !name.is_empty() {
                        filename = name;
                    }
                }
                if let Some(mime) = headers.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-type")
                        .then_some(value.trim())
                }) {
                    if mime.len() > 128 || !mime.is_ascii() || mime.chars().any(char::is_control) {
                        return Err(malformed());
                    }
                    mime_type = mime.to_string();
                }
            }
            Some("language") => {
                if value.len() > 32 {
                    return Err(malformed());
                }
                language = std::str::from_utf8(value)
                    .map_err(|_| malformed())?
                    .trim()
                    .to_string();
            }
            _ => {}
        }
        remaining = &data[end + next_part.len()..];
    }
    Ok(Recording {
        audio: audio.ok_or_else(malformed)?,
        filename,
        mime_type,
        language,
    })
}

fn add_field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
        .as_bytes(),
    );
}

pub async fn transcribe(
    settings: &DictationSettings,
    body: &[u8],
    content_type: &str,
) -> Result<Value, DictationError> {
    if !settings.enabled {
        return Err(DictationError::new(403, "语音输入未启用"));
    }
    let (endpoint, key) = configuration(settings)?;
    let recording = recording(body, content_type)?;
    transcribe_recording(settings, recording, endpoint, key).await
}

async fn transcribe_recording(
    settings: &DictationSettings,
    recording: Recording<'_>,
    endpoint: reqwest::Url,
    key: String,
) -> Result<Value, DictationError> {
    let language = if settings.language.trim().is_empty() {
        recording.language.as_str()
    } else {
        settings.language.trim()
    };
    if language.len() > 32 || language.chars().any(char::is_control) {
        return Err(DictationError::new(400, "语音语言参数无效"));
    }
    let boundary = format!("codex-plus-dictation-{}", uuid::Uuid::new_v4());
    let mut multipart = Vec::with_capacity(recording.audio.len() + 1024);
    add_field(&mut multipart, &boundary, "model", settings.model.trim());
    add_field(&mut multipart, &boundary, "response_format", "json");
    if !language.is_empty() {
        add_field(&mut multipart, &boundary, "language", language);
    }
    multipart.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n", recording.filename, recording.mime_type).as_bytes());
    multipart.extend_from_slice(recording.audio);
    multipart.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(settings.timeout_seconds.clamp(1, 600)))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| DictationError::new(502, "无法创建语音服务连接"))?;
    let mut request = client
        .post(endpoint)
        .header(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(multipart);
    if !key.is_empty() {
        request = request.header(AUTHORIZATION, format!("Bearer {key}"));
    }
    let mut response = request.send().await.map_err(|error| {
        DictationError::new(
            if error.is_timeout() { 504 } else { 502 },
            if error.is_timeout() {
                "语音转写超时，请重试或调整超时设置"
            } else {
                "无法连接语音转写服务"
            },
        )
    })?;
    if !response.status().is_success() {
        // 不回显上游响应体，避免服务商反射请求密钥、音频或 URL。
        return Err(DictationError::new(
            502,
            format!("语音转写服务返回 HTTP {}", response.status().as_u16()),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        if error.is_timeout() {
            DictationError::new(504, "语音转写超时，请重试或调整超时设置")
        } else {
            DictationError::new(502, "语音转写响应读取失败")
        }
    })? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(DictationError::new(502, "语音转写响应过大"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let result: Value = serde_json::from_slice(&bytes)
        .map_err(|_| DictationError::new(502, "语音转写服务返回了无效 JSON"))?;
    let text = result
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| DictationError::new(502, "语音转写结果缺少 text"))?;
    Ok(json!({ "text": text }))
}

// 原生 renderer 的 connect-src 不允许 localhost；音频只经已有特权 bridge 传输。
// 注册表仅保存 UUID 和取消信号，不保存音频、token 或供应商凭据。
pub const MAX_AUDIO_BASE64_BYTES: usize = ((MAX_AUDIO_BODY_BYTES + 2) / 3) * 4;
const MAX_BRIDGE_REQUESTS: usize = 4;
const MAX_FINISHED_REQUESTS: usize = 128;
const FINISHED_REQUEST_TTL: Duration = Duration::from_secs(120);

struct FinishedRequest {
    expires_at: Instant,
    cancelled: bool,
}

#[derive(Default)]
struct BridgeRequests {
    active: HashMap<uuid::Uuid, watch::Sender<bool>>,
    finished: HashMap<uuid::Uuid, FinishedRequest>,
}

impl BridgeRequests {
    fn prune(&mut self, now: Instant) {
        self.finished.retain(|_, request| request.expires_at > now);
    }

    fn remember(&mut self, id: uuid::Uuid, cancelled: bool, now: Instant) {
        self.prune(now);
        if !self.finished.contains_key(&id) && self.finished.len() >= MAX_FINISHED_REQUESTS {
            if let Some(oldest) = self
                .finished
                .iter()
                .min_by_key(|(_, request)| request.expires_at)
                .map(|(id, _)| *id)
            {
                self.finished.remove(&oldest);
            }
        }
        self.finished.insert(
            id,
            FinishedRequest {
                expires_at: now + FINISHED_REQUEST_TTL,
                cancelled,
            },
        );
    }

    fn register(&mut self, id: uuid::Uuid) -> Result<watch::Receiver<bool>, DictationError> {
        self.prune(Instant::now());
        if let Some(request) = self.finished.get(&id) {
            return Err(if request.cancelled {
                cancelled_error()
            } else {
                DictationError::new(409, "语音请求已结束，请重新发起")
            });
        }
        if self.active.contains_key(&id) {
            return Err(DictationError::new(409, "语音请求正在处理，请勿重复提交"));
        }
        if self.active.len() >= MAX_BRIDGE_REQUESTS {
            return Err(DictationError::new(
                429,
                "正在处理的语音请求过多，请稍后重试",
            ));
        }
        let (sender, receiver) = watch::channel(false);
        self.active.insert(id, sender);
        Ok(receiver)
    }

    fn cancel(&mut self, id: uuid::Uuid) -> bool {
        self.prune(Instant::now());
        if let Some(sender) = self.active.get(&id) {
            sender.send_replace(true);
            return true;
        }
        if let Some(request) = self.finished.get(&id) {
            return request.cancelled;
        }
        // 取消可能先于转写注册到达。短期 tombstone 拒绝晚到请求，数量和时间都有界。
        self.remember(id, true, Instant::now());
        true
    }

    fn finish(&mut self, id: uuid::Uuid) -> bool {
        let cancelled = self.active.remove(&id).is_some_and(|sender| {
            let cancelled = *sender.borrow();
            cancelled
        });
        self.remember(id, cancelled, Instant::now());
        cancelled
    }
}

static BRIDGE_REQUESTS: LazyLock<Mutex<BridgeRequests>> =
    LazyLock::new(|| Mutex::new(BridgeRequests::default()));

struct BridgeRequestGuard {
    id: uuid::Uuid,
    cancel: watch::Receiver<bool>,
    finished: bool,
}

impl BridgeRequestGuard {
    fn register(id: uuid::Uuid) -> Result<Self, DictationError> {
        let cancel = BRIDGE_REQUESTS
            .lock()
            .map_err(|_| DictationError::new(503, "语音请求状态不可用，请重试"))?
            .register(id)?;
        Ok(Self {
            id,
            cancel,
            finished: false,
        })
    }

    fn finish(mut self, result: Result<Value, DictationError>) -> Result<Value, DictationError> {
        // 在同一把锁下决定完成/取消，取消先取得锁时不能交付晚到成功结果。
        let cancelled = BRIDGE_REQUESTS
            .lock()
            .map_err(|_| DictationError::new(503, "语音请求状态不可用，请重试"))?
            .finish(self.id);
        self.finished = true;
        if cancelled {
            Err(cancelled_error())
        } else {
            result
        }
    }
}

impl Drop for BridgeRequestGuard {
    fn drop(&mut self) {
        if !self.finished {
            if let Ok(mut requests) = BRIDGE_REQUESTS.lock() {
                requests.finish(self.id);
            }
        }
    }
}

fn cancelled_error() -> DictationError {
    DictationError::new(499, "语音转写已取消")
}

fn bridge_request_id(payload: &Value) -> Result<uuid::Uuid, DictationError> {
    if !valid_helper_token(payload.get("helperToken").and_then(Value::as_str)) {
        return Err(DictationError::new(
            403,
            "语音请求未授权，请从 Codex++ 重新发起",
        ));
    }
    let value = payload
        .get("requestId")
        .and_then(Value::as_str)
        .filter(|value| value.len() == 36)
        .ok_or_else(|| DictationError::new(400, "语音请求 ID 无效"))?;
    uuid::Uuid::parse_str(value)
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or_else(|| DictationError::new(400, "语音请求 ID 无效"))
}

fn bridge_audio(payload: &Value) -> Result<(Vec<u8>, String, String, String), DictationError> {
    let encoded = payload
        .get("audioBase64")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DictationError::new(400, "语音上传缺少音频"))?;
    if encoded.len() > MAX_AUDIO_BASE64_BYTES {
        return Err(DictationError::new(413, "语音上传超过 25 MiB 限制"));
    }
    let mime = match payload.get("mimeType") {
        None => "audio/webm",
        Some(value) => value
            .as_str()
            .ok_or_else(|| DictationError::new(400, "语音音频格式无效"))?,
    };
    if mime.len() > 128 || !mime.is_ascii() || mime.chars().any(char::is_control) {
        return Err(DictationError::new(400, "语音音频格式无效"));
    }
    let mime = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let (mime, extension) = match mime.as_str() {
        "audio/webm" => ("audio/webm", "webm"),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => ("audio/mp4", "m4a"),
        "audio/wav" | "audio/x-wav" => ("audio/wav", "wav"),
        "audio/mpeg" | "audio/mp3" => ("audio/mpeg", "mp3"),
        "audio/ogg" => ("audio/ogg", "ogg"),
        "audio/flac" => ("audio/flac", "flac"),
        _ => return Err(DictationError::new(400, "语音音频格式无效")),
    };
    if let Some(filename) = payload.get("filename") {
        let valid = filename.as_str().is_some_and(|filename| {
            !filename.is_empty()
                && filename.len() <= 128
                && filename
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        });
        if !valid {
            return Err(DictationError::new(400, "语音文件名无效"));
        }
    }
    let language = match payload.get("language") {
        None => "",
        Some(value) => value
            .as_str()
            .ok_or_else(|| DictationError::new(400, "语音语言参数无效"))?,
    }
    .trim();
    if language.len() > 32 || language.chars().any(char::is_control) {
        return Err(DictationError::new(400, "语音语言参数无效"));
    }
    let audio = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| DictationError::new(400, "语音音频编码无效"))?;
    if audio.is_empty() {
        return Err(DictationError::new(400, "语音上传缺少音频"));
    }
    if audio.len() > MAX_AUDIO_BODY_BYTES {
        return Err(DictationError::new(413, "语音上传超过 25 MiB 限制"));
    }
    // 文件名由规范化后的 MIME 生成，不把不可信元数据拼接进上游 multipart 头。
    Ok((
        audio,
        format!("dictation.{extension}"),
        mime.to_string(),
        language.to_string(),
    ))
}

pub async fn transcribe_bridge(
    settings: &DictationSettings,
    payload: &Value,
) -> Result<Value, DictationError> {
    let id = bridge_request_id(payload)?;
    let mut request = BridgeRequestGuard::register(id)?;
    let transcription = async {
        if !settings.enabled {
            return Err(DictationError::new(403, "语音输入未启用"));
        }
        let (endpoint, key) = configuration(settings)?;
        let (audio, filename, mime_type, language) = bridge_audio(payload)?;
        let recording = Recording {
            audio: &audio,
            filename,
            mime_type,
            language,
        };
        transcribe_recording(settings, recording, endpoint, key).await
    };
    let result = tokio::select! {
        biased;
        _ = async {
            loop {
                if *request.cancel.borrow() { break; }
                if request.cancel.changed().await.is_err() { break; }
            }
        } => Err(cancelled_error()),
        result = transcription => result,
    };
    request.finish(result)
}

pub fn cancel_bridge(payload: &Value) -> Result<Value, DictationError> {
    let id = bridge_request_id(payload)?;
    let cancelled = BRIDGE_REQUESTS
        .lock()
        .map_err(|_| DictationError::new(503, "语音请求状态不可用，请重试"))?
        .cancel(id);
    Ok(json!({ "status": "ok", "cancelled": cancelled }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn upload(audio: &[u8], language: &str) -> (Vec<u8>, String) {
        let boundary = "codex-dictation-test";
        let mut body = Vec::new();
        if !language.is_empty() {
            add_field(&mut body, boundary, "language", language);
        }
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
        body.extend_from_slice(audio);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (body, format!("multipart/form-data; boundary={boundary}"))
    }

    fn configured(server: &MockServer) -> DictationSettings {
        DictationSettings {
            enabled: true,
            base_url: format!("{}/v1", server.uri()),
            api_key: "fake-asr-key".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn dictation_defaults_are_opt_in_and_partial_settings_preserve_key() {
        let settings: crate::settings::BackendSettings = serde_json::from_value(json!({})).unwrap();
        assert!(!settings.dictation.enabled);
        assert_eq!(settings.dictation.model, "whisper-large-v3-turbo");
        assert_eq!(settings.dictation.timeout_seconds, 120);
        let temp = tempfile::tempdir().unwrap();
        let store = crate::settings::SettingsStore::new(temp.path().join("settings.json"));
        store
            .update(json!({ "dictation": { "apiKey": "fake-asr-key", "enabled": true } }))
            .unwrap();
        store
            .update(json!({ "dictation": { "model": "whisper-large-v3", "timeoutSeconds": 700 } }))
            .unwrap();
        let saved = store.load().unwrap();
        assert!(saved.dictation.enabled);
        assert_eq!(saved.dictation.api_key, "fake-asr-key");
        assert_eq!(saved.dictation.model, "whisper-large-v3");
        assert_eq!(saved.dictation.timeout_seconds, 600);
        store
            .update(json!({ "dictation": { "apiKey": "" } }))
            .unwrap();
        assert!(store.load().unwrap().dictation.api_key.is_empty());
    }

    #[test]
    fn dictation_status_and_configuration_never_expose_secrets() {
        let mut settings = DictationSettings {
            api_key: "fake-asr-key".to_string(),
            ..Default::default()
        };
        let status = public_status(&settings);
        assert_eq!(status["configured"], true);
        assert!(!status.to_string().contains("fake-asr-key"));
        assert!(status.get("helperToken").is_none());
        settings.base_url = "https://fake-user:fake-password@api.example/v1".to_string();
        assert!(
            !public_status(&settings)
                .to_string()
                .contains("fake-password")
        );
        settings.base_url = DictationSettings::default().base_url;
        settings.api_key.clear();
        assert_eq!(public_status(&settings)["configured"], false);
        settings.api_key_env = format!(
            "CODEX_PLUS_DICTATION_MISSING_{}",
            uuid::Uuid::new_v4().simple()
        );
        assert!(configuration(&settings).is_err());
        settings.api_key = "fake-direct-key".to_string();
        assert_eq!(configuration(&settings).unwrap().1, "fake-direct-key");
        assert!(!valid_helper_token(None));
        assert!(!valid_helper_token(Some("untrusted")));
        assert!(valid_helper_token(Some(helper_token())));
    }

    #[test]
    fn dictation_upload_rejects_missing_duplicate_files_and_size_overflow() {
        assert!(recording(b"{}", "application/json").is_err());
        let (mut body, content_type) = upload(b"binary", "");
        body.truncate(body.len() - b"--codex-dictation-test--\r\n".len());
        body.extend_from_slice(&upload(b"second", "").0);
        assert!(recording(&body, &content_type).is_err());
        let oversized = vec![0; MAX_AUDIO_BODY_BYTES + 1];
        assert_eq!(
            recording(&oversized, &content_type).err().unwrap().status,
            413
        );
    }

    #[test]
    fn dictation_upload_preserves_incomplete_boundary_prefixes() {
        let audio = b"\x00\x80\xff\r\n--codex-dictation-testX\r\nbinary\r\n--codex-dictation-test--X\r\ntail";
        let (body, content_type) = upload(audio, "zh");
        let parsed = recording(&body, &content_type).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(parsed.audio, audio);
        assert_eq!(parsed.language, "zh");
    }

    #[tokio::test]
    async fn dictation_transcribe_forwards_binary_and_independent_asr_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .and(header("Authorization", "Bearer fake-asr-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "text": "你好 world", "private": "omit" })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let audio = b"\x00\x80\xffA\r\n--codex-dictation-testX\r\nmore";
        let (body, content_type) = upload(audio, "zh");
        let result = transcribe(&configured(&server), &body, &content_type)
            .await
            .unwrap();
        assert_eq!(result, json!({ "text": "你好 world" }));
        let requests = server.received_requests().await.unwrap();
        let request = &requests[0];
        assert!(find_bytes(&request.body, audio).is_some());
        let text = String::from_utf8_lossy(&request.body);
        assert!(text.contains("name=\"model\"\r\n\r\nwhisper-large-v3-turbo"));
        assert!(text.contains("name=\"response_format\"\r\n\r\njson"));
        assert!(text.contains("name=\"language\"\r\n\r\nzh"));
    }

    #[tokio::test]
    async fn dictation_disabled_rejects_without_upstream_and_local_can_omit_key() {
        let server = MockServer::start().await;
        let mut settings = configured(&server);
        settings.enabled = false;
        let (body, content_type) = upload(b"audio", "");
        assert_eq!(
            transcribe(&settings, &body, &content_type)
                .await
                .unwrap_err()
                .status,
            403
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        settings.enabled = true;
        settings.api_key.clear();
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "local" })))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            transcribe(&settings, &body, &content_type).await.unwrap()["text"],
            "local"
        );
        assert!(
            !server.received_requests().await.unwrap()[0]
                .headers
                .contains_key("authorization")
        );
    }

    #[tokio::test]
    async fn dictation_upstream_error_timeout_and_bad_response_are_sanitized() {
        let server = MockServer::start().await;
        let (body, content_type) = upload(b"audio", "");
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("fake-asr-key reflected"))
            .mount(&server)
            .await;
        let error = transcribe(&configured(&server), &body, &content_type)
            .await
            .unwrap_err();
        assert!(error.message.contains("401"));
        assert!(!error.message.contains("fake-asr-key"));
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "wrong": "result" })))
            .mount(&server)
            .await;
        assert!(
            transcribe(&configured(&server), &body, &content_type)
                .await
                .unwrap_err()
                .message
                .contains("text")
        );
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;
        let mut settings = configured(&server);
        settings.timeout_seconds = 1;
        assert_eq!(
            transcribe(&settings, &body, &content_type)
                .await
                .unwrap_err()
                .status,
            504
        );
    }

    #[tokio::test]
    async fn dictation_response_body_timeout_returns_gateway_timeout() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 8192];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            // 立即返回响应头和首字节，超时必须发生在后续 response.chunk()。
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{").await.unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
            let _ = stream.write_all(b"\"text\":\"ok\"}").await;
        });
        let settings = DictationSettings {
            enabled: true,
            base_url: format!("http://{addr}/v1"),
            timeout_seconds: 1,
            ..Default::default()
        };
        let (body, content_type) = upload(b"audio", "");
        let error = transcribe(&settings, &body, &content_type)
            .await
            .unwrap_err();
        assert_eq!(error.status, 504);
        assert!(error.message.contains("超时"));
        server.abort();
    }

    fn bridge_payload(audio: &[u8], mime: &str) -> Value {
        json!({
            "requestId": uuid::Uuid::new_v4().to_string(),
            "helperToken": helper_token(),
            "audioBase64": base64::engine::general_purpose::STANDARD.encode(audio),
            "mimeType": mime,
            "filename": "untrusted.wav",
            "language": "zh"
        })
    }

    #[test]
    fn dictation_bridge_input_bounds_and_metadata_are_checked() {
        for payload in [
            json!({}),
            json!({ "audioBase64": "not-base64!" }),
            json!({ "audioBase64": "" }),
        ] {
            assert_eq!(bridge_audio(&payload).err().unwrap().status, 400);
        }
        let mut payload = bridge_payload(b"binary", "audio/webm;codecs=opus");
        let (audio, filename, mime, language) = bridge_audio(&payload).unwrap();
        assert_eq!(audio, b"binary");
        assert_eq!(filename, "dictation.webm");
        assert_eq!(mime, "audio/webm");
        assert_eq!(language, "zh");
        for mime in ["text/html", "audio/webm\r\nAuthorization: fake-key", ""] {
            payload["mimeType"] = json!(mime);
            assert_eq!(bridge_audio(&payload).err().unwrap().status, 400);
        }
        payload["mimeType"] = json!("audio/wav");
        payload["filename"] = json!("../secret.wav");
        assert_eq!(bridge_audio(&payload).err().unwrap().status, 400);
        payload["filename"] = json!("safe.wav");
        payload["language"] = json!("zh\r\nsecret");
        assert_eq!(bridge_audio(&payload).err().unwrap().status, 400);
        payload["language"] = json!("");
        payload["audioBase64"] = json!("A".repeat(MAX_AUDIO_BASE64_BYTES + 4));
        assert_eq!(bridge_audio(&payload).err().unwrap().status, 413);
        payload["audioBase64"] = json!(
            base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_AUDIO_BODY_BYTES + 1])
        );
        assert_eq!(bridge_audio(&payload).err().unwrap().status, 413);
    }

    #[test]
    fn dictation_bridge_request_registry_is_bounded_and_expires() {
        let mut requests = BridgeRequests::default();
        let mut receivers = Vec::new();
        for _ in 0..MAX_BRIDGE_REQUESTS {
            receivers.push(requests.register(uuid::Uuid::new_v4()).unwrap());
        }
        assert_eq!(
            requests
                .register(uuid::Uuid::new_v4())
                .err()
                .unwrap()
                .status,
            429
        );
        let now = Instant::now();
        for _ in 0..MAX_FINISHED_REQUESTS + 20 {
            requests.remember(uuid::Uuid::new_v4(), true, now);
        }
        assert_eq!(requests.finished.len(), MAX_FINISHED_REQUESTS);
        requests.prune(now + FINISHED_REQUEST_TTL + Duration::from_secs(1));
        assert!(requests.finished.is_empty());
        assert_eq!(requests.active.len(), MAX_BRIDGE_REQUESTS);
    }

    #[tokio::test]
    async fn dictation_bridge_preserves_binary_and_sanitizes_upstream_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .and(header("Authorization", "Bearer fake-asr-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({ "text": "bridge transcript", "private": "fake-asr-key" }),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let audio = b"\x00\x80\xffA\r\n--binary-boundaryX\r\nmore";
        let payload = bridge_payload(audio, "audio/wav");
        let result = transcribe_bridge(&configured(&server), &payload)
            .await
            .unwrap();
        assert_eq!(result, json!({ "text": "bridge transcript" }));
        let requests = server.received_requests().await.unwrap();
        assert!(find_bytes(&requests[0].body, audio).is_some());
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("name=\"model\"\r\n\r\nwhisper-large-v3-turbo"));
        assert!(body.contains("name=\"language\"\r\n\r\nzh"));
        assert!(body.contains("filename=\"dictation.wav\""));
        assert_eq!(cancel_bridge(&payload).unwrap()["cancelled"], false);
        assert_eq!(
            transcribe_bridge(&configured(&server), &payload)
                .await
                .unwrap_err()
                .status,
            409
        );
    }

    #[tokio::test]
    async fn dictation_bridge_cancel_before_registration_rejects_late_audio() {
        let server = MockServer::start().await;
        let payload = bridge_payload(b"late secret audio", "audio/wav");
        assert_eq!(cancel_bridge(&payload).unwrap()["cancelled"], true);
        assert_eq!(cancel_bridge(&payload).unwrap()["cancelled"], true);
        let error = transcribe_bridge(&configured(&server), &payload)
            .await
            .unwrap_err();
        assert_eq!(error.status, 499);
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(!error.message.contains("secret"));
    }

    #[tokio::test]
    async fn dictation_bridge_cancel_drops_delayed_upstream_connection() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            let header_end = loop {
                if let Some(end) = find_bytes(&bytes, b"\r\n\r\n") {
                    break end;
                }
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < header_end + 4 + length {
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            started_sender.send(()).unwrap();
            // 不发送响应；只有 reqwest future 被丢弃，连接才会在两秒内结束。
            let count = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(count, 0);
            let _ = stream.shutdown().await;
        });
        let payload = bridge_payload(b"delayed-audio", "audio/wav");
        let request_payload = payload.clone();
        let settings = DictationSettings {
            enabled: true,
            base_url: format!("http://{addr}/v1"),
            timeout_seconds: 30,
            ..Default::default()
        };
        let job = tokio::spawn(async move { transcribe_bridge(&settings, &request_payload).await });
        tokio::time::timeout(Duration::from_secs(3), started_receiver)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cancel_bridge(&payload).unwrap()["cancelled"], true);
        let error = tokio::time::timeout(Duration::from_secs(1), job)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.status, 499);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dictation_bridge_errors_do_not_echo_credentials_or_audio() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_string("fake-asr-key secret-audio helper-token"),
            )
            .mount(&server)
            .await;
        let payload = bridge_payload(b"secret-audio", "audio/wav");
        let error = transcribe_bridge(&configured(&server), &payload)
            .await
            .unwrap_err();
        assert_eq!(error.status, 502);
        assert!(error.message.contains("401"));
        for secret in [
            "fake-asr-key",
            "secret-audio",
            helper_token(),
            payload["audioBase64"].as_str().unwrap(),
        ] {
            assert!(!error.message.contains(secret));
        }
        let mut invalid = bridge_payload(b"secret-audio", "audio/wav");
        invalid["helperToken"] = json!("fake-invalid-token");
        assert_eq!(
            transcribe_bridge(&configured(&server), &invalid)
                .await
                .unwrap_err()
                .status,
            403
        );
        assert_eq!(cancel_bridge(&invalid).unwrap_err().status, 403);
        invalid["helperToken"] = json!(helper_token());
        invalid["requestId"] = json!("not-a-uuid");
        assert_eq!(
            transcribe_bridge(&configured(&server), &invalid)
                .await
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(cancel_bridge(&invalid).unwrap_err().status, 400);
    }
}

#[cfg(test)]
mod enhancement_switch_tests {
    use super::*;

    #[tokio::test]
    async fn dictation_enhancement_switch_disables_runtime_and_preserves_saved_configuration() {
        let mut settings = BackendSettings::default();
        settings.enhancements_enabled = false;
        settings.dictation.enabled = true;
        settings.dictation.api_key = "fake-independent-asr-key".to_string();
        let effective = effective_settings(&settings);
        assert!(!effective.enabled);
        assert_eq!(public_status(&effective)["enabled"], false);
        let error = transcribe(&effective, b"", "").await.unwrap_err();
        assert_eq!(error.status, 403);
        assert!(settings.dictation.enabled);
        assert_eq!(settings.dictation.api_key, "fake-independent-asr-key");
        settings.enhancements_enabled = true;
        assert!(effective_settings(&settings).enabled);
    }
}
