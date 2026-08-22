use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

const CHANNEL_VERSION: &str = "codex-plus-weixin/1.0";
const MAX_REPLY_CHARS: usize = 3_800;
const MAX_API_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SMALL_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct WeixinClient {
    base_url: String,
    token: String,
    route_tag: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WeixinQrCode {
    #[serde(rename = "qrcode")]
    pub qr_code: String,
    #[serde(rename = "qrcode_img_content")]
    pub qr_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct WeixinQrStatus {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub ilink_bot_id: String,
    #[serde(default)]
    pub baseurl: String,
    #[serde(default)]
    pub ilink_user_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeixinUpdates {
    #[serde(default)]
    pub ret: i64,
    #[serde(default)]
    pub errcode: i64,
    #[serde(default)]
    pub errmsg: String,
    #[serde(default, rename = "msgs")]
    pub messages: Vec<WeixinMessage>,
    #[serde(default)]
    pub get_updates_buf: String,
    #[serde(default)]
    pub longpolling_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeixinMessage {
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub message_id: i64,
    #[serde(default)]
    pub from_user_id: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub create_time_ms: i64,
    #[serde(default)]
    pub message_type: i64,
    #[serde(default)]
    pub message_state: i64,
    #[serde(default)]
    pub item_list: Vec<WeixinMessageItem>,
    #[serde(default)]
    pub context_token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeixinMessageItem {
    #[serde(default, rename = "type")]
    pub item_type: i64,
    #[serde(default)]
    pub text_item: Option<WeixinTextItem>,
    #[serde(default)]
    pub voice_item: Option<WeixinVoiceItem>,
    #[serde(default)]
    pub ref_msg: Option<WeixinReferenceMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeixinTextItem {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeixinVoiceItem {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeixinReferenceMessage {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub message_item: Option<Box<WeixinMessageItem>>,
}

#[derive(Debug, Deserialize)]
struct WeixinSendResponse {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    errmsg: String,
}

impl WeixinClient {
    pub fn new(base_url: &str, token: &str, route_tag: &str) -> anyhow::Result<Self> {
        let base_url = normalize_base_url(base_url);
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .context("无法创建微信 HTTP 客户端")?;
        Ok(Self {
            base_url,
            token: token.trim().to_string(),
            route_tag: route_tag.trim().to_string(),
            client,
        })
    }

    pub async fn fetch_qr_code(base_url: &str, route_tag: &str) -> anyhow::Result<WeixinQrCode> {
        let client = Self::new(base_url, "", route_tag)?;
        let mut url = client.endpoint("ilink/bot/get_bot_qrcode")?;
        url.query_pairs_mut().append_pair("bot_type", "3");
        let response = client
            .client
            .get(url)
            .headers(client.route_headers(false)?)
            .timeout(Duration::from_secs(40))
            .send()
            .await
            .context("获取微信登录二维码失败")?;
        let (status, bytes) =
            read_response_limited(response, MAX_SMALL_RESPONSE_BYTES, "微信二维码").await?;
        if !status.is_success() {
            bail!("获取微信登录二维码失败：HTTP {status}");
        }
        let qr: WeixinQrCode = serde_json::from_slice(&bytes).context("微信二维码响应格式无效")?;
        if qr.qr_code.trim().is_empty() || qr.qr_content.trim().is_empty() {
            bail!("微信二维码响应缺少必要字段");
        }
        Ok(qr)
    }

    pub async fn poll_qr_status(
        base_url: &str,
        route_tag: &str,
        qr_code: &str,
    ) -> anyhow::Result<WeixinQrStatus> {
        let client = Self::new(base_url, "", route_tag)?;
        let mut url = client.endpoint("ilink/bot/get_qrcode_status")?;
        url.query_pairs_mut().append_pair("qrcode", qr_code);
        let response = client
            .client
            .get(url)
            .headers(client.route_headers(true)?)
            .timeout(Duration::from_secs(40))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) if error.is_timeout() => {
                return Ok(WeixinQrStatus {
                    status: "wait".to_string(),
                    ..WeixinQrStatus::default()
                });
            }
            Err(error) => return Err(error).context("查询微信扫码状态失败"),
        };
        let (status, bytes) =
            read_response_limited(response, MAX_SMALL_RESPONSE_BYTES, "微信扫码状态").await?;
        if !status.is_success() {
            bail!("查询微信扫码状态失败：HTTP {status}");
        }
        serde_json::from_slice(&bytes).context("微信扫码状态响应格式无效")
    }

    pub async fn get_updates(
        &self,
        get_updates_buf: &str,
        timeout_ms: u64,
    ) -> anyhow::Result<WeixinUpdates> {
        let request_body = json!({
            "get_updates_buf": get_updates_buf,
            "base_info": { "channel_version": CHANNEL_VERSION }
        });
        let timeout_ms = timeout_ms.clamp(1_000, 60_000);
        let response = self
            .client
            .post(self.endpoint("ilink/bot/getupdates")?)
            .headers(self.auth_headers()?)
            .json(&request_body)
            .timeout(Duration::from_millis(timeout_ms + 5_000))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) if error.is_timeout() => {
                return Ok(WeixinUpdates {
                    ret: 0,
                    errcode: 0,
                    errmsg: String::new(),
                    messages: Vec::new(),
                    get_updates_buf: get_updates_buf.to_string(),
                    longpolling_timeout_ms: timeout_ms,
                });
            }
            Err(error) => return Err(error).context("微信长轮询请求失败"),
        };
        let (http_status, bytes) =
            read_response_limited(response, MAX_API_RESPONSE_BYTES, "微信长轮询").await?;
        if !http_status.is_success() {
            bail!("微信长轮询请求失败：HTTP {http_status}");
        }
        let updates: WeixinUpdates =
            serde_json::from_slice(&bytes).context("微信长轮询响应格式无效")?;
        if updates.ret != 0 || updates.errcode != 0 {
            bail!(
                "微信长轮询被拒绝：ret={} errcode={} {}",
                updates.ret,
                updates.errcode,
                updates.errmsg
            );
        }
        Ok(updates)
    }

    pub async fn send_text_chunks(
        &self,
        to_user_id: &str,
        text: &str,
        context_token: &str,
    ) -> anyhow::Result<()> {
        if context_token.trim().is_empty() {
            bail!("微信回复缺少 context_token");
        }
        for chunk in chunk_text(text, MAX_REPLY_CHARS) {
            self.send_text(to_user_id, &chunk, context_token).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    async fn send_text(
        &self,
        to_user_id: &str,
        text: &str,
        context_token: &str,
    ) -> anyhow::Result<()> {
        let request_body = json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to_user_id,
                "client_id": Uuid::new_v4().to_string(),
                "message_type": 2,
                "message_state": 2,
                "item_list": [{
                    "type": 1,
                    "text_item": { "text": text }
                }],
                "context_token": context_token
            },
            "base_info": { "channel_version": CHANNEL_VERSION }
        });
        let response = self
            .client
            .post(self.endpoint("ilink/bot/sendmessage")?)
            .headers(self.auth_headers()?)
            .json(&request_body)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .context("发送微信回复失败")?;
        let (http_status, bytes) =
            read_response_limited(response, MAX_SMALL_RESPONSE_BYTES, "微信发送响应").await?;
        if !http_status.is_success() {
            bail!("发送微信回复失败：HTTP {http_status}");
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(());
        }
        let result: WeixinSendResponse =
            serde_json::from_slice(&bytes).context("微信发送响应格式无效")?;
        if result.ret != 0 || result.errcode != 0 {
            bail!(
                "发送微信回复被拒绝：ret={} errcode={} {}",
                result.ret,
                result.errcode,
                result.errmsg
            );
        }
        Ok(())
    }

    fn endpoint(&self, path: &str) -> anyhow::Result<reqwest::Url> {
        let base = format!("{}/", self.base_url.trim_end_matches('/'));
        reqwest::Url::parse(&base)?
            .join(path.trim_start_matches('/'))
            .context("微信 API 地址无效")
    }

    fn auth_headers(&self) -> anyhow::Result<HeaderMap> {
        let mut headers = self.route_headers(false)?;
        headers.insert(
            "authorizationtype",
            HeaderValue::from_static("ilink_bot_token"),
        );
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {}", self.token))?,
        );
        headers.insert("x-wechat-uin", HeaderValue::from_str(&random_wechat_uin())?);
        Ok(headers)
    }

    fn route_headers(&self, qr_status: bool) -> anyhow::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        if qr_status {
            headers.insert("ilink-app-clientversion", HeaderValue::from_static("1"));
        }
        if !self.route_tag.is_empty() {
            headers.insert("skroutetag", HeaderValue::from_str(&self.route_tag)?);
        }
        Ok(headers)
    }
}

pub fn render_qr_svg(content: &str) -> anyhow::Result<String> {
    let code = qrcode::QrCode::new(content.trim().as_bytes()).context("无法编码微信登录二维码")?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(320, 320)
        .dark_color(qrcode::render::svg::Color("#111827"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build())
}

async fn read_response_limited(
    response: reqwest::Response,
    max_bytes: usize,
    label: &str,
) -> anyhow::Result<(reqwest::StatusCode, Vec<u8>)> {
    let status = response.status();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("读取{label}失败"))?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            bail!("{label}超过大小限制");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((status, bytes))
}

impl WeixinMessage {
    pub fn is_finished_user_message(&self) -> bool {
        self.message_type == 1 && self.message_state == 2 && !self.from_user_id.trim().is_empty()
    }

    pub fn text(&self) -> Option<String> {
        self.item_list.iter().find_map(message_item_text)
    }

    pub fn dedup_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.from_user_id.trim(),
            self.message_id,
            self.seq,
            self.create_time_ms,
            self.client_id.trim()
        )
    }

    pub fn is_older_than(&self, now_ms: u64, max_age_ms: u64) -> bool {
        self.create_time_ms > 0 && now_ms.saturating_sub(self.create_time_ms as u64) > max_age_ms
    }
}

fn message_item_text(item: &WeixinMessageItem) -> Option<String> {
    let body = match item.item_type {
        1 => item.text_item.as_ref().map(|item| item.text.trim()),
        3 => item.voice_item.as_ref().map(|item| item.text.trim()),
        _ => None,
    }?;
    if body.is_empty() {
        return None;
    }
    let Some(reference) = item.ref_msg.as_ref() else {
        return Some(body.to_string());
    };
    let mut reference_parts = Vec::new();
    if !reference.title.trim().is_empty() {
        reference_parts.push(reference.title.trim().to_string());
    }
    if let Some(reference_item) = reference.message_item.as_deref()
        && let Some(reference_text) = message_item_text(reference_item)
    {
        reference_parts.push(reference_text);
    }
    if reference_parts.is_empty() {
        Some(body.to_string())
    } else {
        Some(format!("[引用: {}]\n{body}", reference_parts.join(" | ")))
    }
}

fn normalize_base_url(value: &str) -> String {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        super::DEFAULT_WEIXIN_BASE_URL.to_string()
    } else {
        value.to_string()
    }
}

fn random_wechat_uin() -> String {
    let bytes = Uuid::new_v4().into_bytes();
    let number = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    base64::engine::general_purpose::STANDARD.encode(number.to_string())
}

fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 || text.is_empty() {
        return Vec::new();
    }
    let chars = text.chars().collect::<Vec<_>>();
    chars
        .chunks(max_chars)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_and_voice_transcription() {
        let text: WeixinMessage = serde_json::from_value(json!({
            "message_type": 1,
            "message_state": 2,
            "from_user_id": "peer",
            "item_list": [{"type": 1, "text_item": {"text": " hello "}}]
        }))
        .unwrap();
        assert_eq!(text.text().as_deref(), Some("hello"));

        let voice: WeixinMessage = serde_json::from_value(json!({
            "message_type": 1,
            "message_state": 2,
            "from_user_id": "peer",
            "item_list": [{"type": 3, "voice_item": {"text": "语音文字"}}]
        }))
        .unwrap();
        assert_eq!(voice.text().as_deref(), Some("语音文字"));
    }

    #[test]
    fn chunks_unicode_by_character_count() {
        assert_eq!(chunk_text("一二三四五", 2), vec!["一二", "三四", "五"]);
    }

    #[test]
    fn dedup_key_distinguishes_zero_message_ids() {
        let first: WeixinMessage = serde_json::from_value(json!({
            "from_user_id": "peer",
            "message_id": 0,
            "seq": 0,
            "create_time_ms": 100,
            "client_id": "client-a"
        }))
        .unwrap();
        let second: WeixinMessage = serde_json::from_value(json!({
            "from_user_id": "peer",
            "message_id": 0,
            "seq": 0,
            "create_time_ms": 101,
            "client_id": "client-b"
        }))
        .unwrap();
        assert_ne!(first.dedup_key(), second.dedup_key());
    }

    #[test]
    fn renders_qr_as_svg_without_remote_service() {
        let svg = render_qr_svg("https://example.test/login").unwrap();
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("<svg"));
    }
}
