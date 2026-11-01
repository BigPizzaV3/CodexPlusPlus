use std::path::{Path, PathBuf};
use std::time::Duration;

use aes_gcm::aead::rand_core::{OsRng, RngCore};
use aes_gcm::aes::{
    Aes128,
    cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, block_padding::Pkcs7},
};
use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use md5::{Digest, Md5};
use reqwest::{Url, redirect::Policy};
use serde::Deserialize;

use super::inbox::Inbox;
use super::weixin::{WeixinMessage, WeixinMessageItem};

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;

pub const DEFAULT_CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";
pub const MAX_ATTACHMENT_BYTES: usize = 100 * 1024 * 1024;
pub(super) const MAX_REFERENCE_DEPTH: usize = 4;
const MAX_ATTACHMENTS: usize = 10;
const MEDIA_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    Image,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAttachment {
    pub path: PathBuf,
    pub kind: AttachmentKind,
}

#[derive(Debug, Default)]
pub struct DownloadedAttachments {
    pub attachments: Vec<LocalAttachment>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WeixinCdnMedia {
    #[serde(default)]
    pub encrypt_query_param: String,
    #[serde(default)]
    pub aes_key: String,
    #[serde(default)]
    pub full_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeixinImageItem {
    #[serde(default)]
    pub media: Option<WeixinCdnMedia>,
    #[serde(default)]
    pub aeskey: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeixinFileItem {
    #[serde(default)]
    pub media: Option<WeixinCdnMedia>,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub len: String,
    #[serde(default)]
    pub md5: String,
}

#[derive(Default, Deserialize)]
pub(super) struct MediaApiResponse {
    #[serde(default)]
    pub ret: i64,
    #[serde(default)]
    pub errcode: i64,
    pub upload_param: Option<String>,
    pub upload_full_url: Option<String>,
}

pub(super) struct PreparedUpload {
    pub ciphertext: Vec<u8>,
    pub plaintext_size: usize,
    pub md5: String,
    pub key_hex: String,
    pub filekey: String,
    pub filename: String,
}

impl PreparedUpload {
    pub(super) async fn read_workspace(path: &Path, workspace: &Path) -> anyhow::Result<Self> {
        let path = path.to_owned();
        let workspace = workspace.to_owned();
        tokio::task::spawn_blocking(move || {
            let (path, bytes) = super::outbound::read_workspace_file(&path, &workspace)?;
            let mut key = [0u8; 16];
            OsRng
                .try_fill_bytes(&mut key)
                .map_err(|_| anyhow::anyhow!("无法生成微信媒体加密密钥"))?;
            Ok(Self {
                plaintext_size: bytes.len(),
                md5: hex(&Md5::digest(&bytes)),
                ciphertext: ecb::Encryptor::<Aes128>::new((&key).into())
                    .encrypt_padded_vec_mut::<Pkcs7>(&bytes),
                key_hex: hex(&key),
                filekey: uuid::Uuid::new_v4().to_string(),
                filename: safe_filename(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("file.bin"),
                ),
            })
        })
        .await
        .context("读取外发附件任务失败")?
    }
}

#[derive(Debug, Clone)]
pub(super) struct MediaClient {
    client: reqwest::Client,
    base_url: Url,
    local_origin: Option<Url>,
}

impl MediaClient {
    pub(super) fn new(base_url: &str) -> anyhow::Result<Self> {
        let base_url = parse_url(base_url.trim())?;
        let local_origin = is_loopback(&base_url).then(|| base_url.clone());
        validate_url(&base_url, local_origin.as_ref())?;
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(MEDIA_TIMEOUT)
            .redirect(Policy::none());
        if local_origin.is_some() {
            builder = builder.no_proxy();
        }
        // A separate client ensures bot headers/cookies never reach media endpoints.
        let client = builder.build().context("无法创建微信媒体客户端")?;
        Ok(Self {
            client,
            base_url,
            local_origin,
        })
    }

    pub(super) async fn download_attachments(
        &self,
        message: &WeixinMessage,
        directory: &Inbox,
    ) -> DownloadedAttachments {
        let mut result = DownloadedAttachments::default();
        let (items, truncated) = attachment_items(message);
        if truncated {
            result
                .warnings
                .push("微信附件数量或引用层数超过限制".into());
        }
        let mut remaining = MAX_ATTACHMENT_BYTES;
        for (index, item) in items.into_iter().enumerate() {
            match self.download_item(item, directory, remaining).await {
                Ok((attachment, size)) => {
                    remaining -= size;
                    result.attachments.push(attachment);
                }
                Err(error) => {
                    result
                        .warnings
                        .push(format!("附件 {}：{error:#}", index + 1));
                }
            }
        }
        if directory.record(&result.attachments).is_err() {
            // 归属登记失败的文件保留供排查，但不能作为已接收附件交给会话。
            result.attachments.clear();
            result.warnings.push("无法登记微信附件归属".into());
        }
        result
    }

    async fn download_item(
        &self,
        item: &WeixinMessageItem,
        directory: &Inbox,
        max_bytes: usize,
    ) -> anyhow::Result<(LocalAttachment, usize)> {
        let (kind, media, hex_key, filename, expected_size, expected_md5, image_url) =
            match item.item_type {
                2 => {
                    let image = item.image_item.as_ref().context("缺少图片信息")?;
                    (
                        AttachmentKind::Image,
                        image.media.as_ref(),
                        image.aeskey.as_str(),
                        "",
                        None,
                        "",
                        image.url.as_str(),
                    )
                }
                4 => {
                    let file = item.file_item.as_ref().context("缺少文件信息")?;
                    let size = if file.len.is_empty() {
                        None
                    } else {
                        Some(file.len.parse::<u64>().context("附件大小格式无效")?)
                    };
                    (
                        AttachmentKind::File,
                        file.media.as_ref(),
                        "",
                        file.file_name.as_str(),
                        size,
                        file.md5.as_str(),
                        "",
                    )
                }
                _ => bail!("不支持此附件类型"),
            };
        if expected_size.is_some_and(|size| size > max_bytes as u64) || max_bytes == 0 {
            bail!("本条消息的附件总大小超过 100 MiB 限制");
        }
        let key = if !hex_key.trim().is_empty() {
            Some(parse_hex_key(hex_key)?)
        } else if let Some(media) = media.filter(|media| !media.aes_key.trim().is_empty()) {
            Some(parse_base64_key(&media.aes_key)?)
        } else {
            None
        };
        if kind == AttachmentKind::File && key.is_none() {
            bail!("文件附件缺少加密密钥");
        }
        let url = self.download_url(media, image_url)?;
        let response = self.client.get(url).send().await.map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            bail!("微信 CDN 下载失败：HTTP {status}");
        }
        let wire_limit = max_bytes + if key.is_some() { 16 } else { 0 };
        let bytes = read_bytes_limited(response, wire_limit).await?;
        let bytes = match key {
            Some(key) => decrypt(&bytes, &key)?,
            None => bytes,
        };
        if bytes.len() > max_bytes {
            bail!("本条消息的附件总大小超过 100 MiB 限制");
        }
        if expected_size.is_some_and(|size| size != bytes.len() as u64) {
            bail!("附件实际大小与文件信息不符");
        }
        if !expected_md5.is_empty()
            && !format!("{:x}", Md5::digest(&bytes)).eq_ignore_ascii_case(expected_md5)
        {
            bail!("附件 MD5 校验失败");
        }
        let filename = if kind == AttachmentKind::Image {
            format!("image.{}", image_extension(&bytes))
        } else {
            safe_filename(filename)
        };
        let path = directory.save(&filename, &bytes).await?;
        Ok((LocalAttachment { path, kind }, bytes.len()))
    }

    fn download_url(&self, media: Option<&WeixinCdnMedia>, image_url: &str) -> anyhow::Result<Url> {
        let url = if let Some(media) = media.filter(|media| !media.full_url.trim().is_empty()) {
            parse_url(media.full_url.trim())?
        } else if let Some(media) =
            media.filter(|media| !media.encrypt_query_param.trim().is_empty())
        {
            let mut url = self.endpoint("download")?;
            url.query_pairs_mut()
                .append_pair("encrypted_query_param", &media.encrypt_query_param);
            url
        } else if !image_url.trim().is_empty() {
            parse_url(image_url.trim())?
        } else {
            bail!("附件缺少 CDN 下载信息");
        };
        validate_url(&url, self.local_origin.as_ref())?;
        Ok(url)
    }

    pub(super) async fn upload(
        &self,
        upload: &PreparedUpload,
        response: &MediaApiResponse,
    ) -> anyhow::Result<String> {
        let url = if let Some(url) = response
            .upload_full_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            parse_url(url.trim())?
        } else if let Some(param) = response
            .upload_param
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            let mut url = self.endpoint("upload")?;
            url.query_pairs_mut()
                .append_pair("encrypted_query_param", param)
                .append_pair("filekey", &upload.filekey);
            url
        } else {
            bail!("微信响应缺少 CDN 上传信息");
        };
        validate_url(&url, self.local_origin.as_ref())?;
        let response = self
            .client
            .post(url)
            .header("content-type", "application/octet-stream")
            .body(upload.ciphertext.clone())
            .send()
            .await
            .map_err(network_error)?;
        if response.status() != reqwest::StatusCode::OK {
            bail!("微信 CDN 上传失败：HTTP {}", response.status());
        }
        response
            .headers()
            .get("x-encrypted-param")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .context("微信 CDN 上传响应缺少下载参数")
    }

    fn endpoint(&self, path: &str) -> anyhow::Result<Url> {
        let mut url = self.base_url.clone();
        url.set_path(&format!(
            "{}/{}",
            self.base_url.path().trim_end_matches('/'),
            path
        ));
        url.set_query(None);
        Ok(url)
    }
}

fn decrypt(ciphertext: &[u8], key: &[u8; 16]) -> anyhow::Result<Vec<u8>> {
    ecb::Decryptor::<Aes128>::new(key.into())
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| anyhow::anyhow!("微信媒体密文或 PKCS7 填充无效"))
}

fn parse_hex_key(encoded: &str) -> anyhow::Result<[u8; 16]> {
    let encoded = encoded.trim();
    if encoded.len() != 32 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("微信媒体十六进制密钥无效");
    }
    let mut key = [0u8; 16];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&encoded[index * 2..index * 2 + 2], 16)
            .map_err(|_| anyhow::anyhow!("微信媒体十六进制密钥无效"))?;
    }
    Ok(key)
}

fn parse_base64_key(encoded: &str) -> anyhow::Result<[u8; 16]> {
    if encoded.len() > 128 {
        bail!("微信媒体 base64 密钥无效");
    }
    let encoded: String = encoded
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    let decoded = STANDARD
        .decode(&encoded)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&encoded))
        .map_err(|_| anyhow::anyhow!("微信媒体 base64 密钥无效"))?;
    match decoded.len() {
        16 => decoded
            .try_into()
            .map_err(|_| anyhow::anyhow!("微信媒体密钥长度无效")),
        32 => parse_hex_key(
            std::str::from_utf8(&decoded).map_err(|_| anyhow::anyhow!("微信媒体密钥编码无效"))?,
        ),
        _ => bail!("微信媒体密钥长度无效"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_url(value: &str) -> anyhow::Result<Url> {
    Url::parse(value).map_err(|_| anyhow::anyhow!("微信 CDN 地址无效"))
}

fn is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

fn validate_url(url: &Url, local_origin: Option<&Url>) -> anyhow::Result<()> {
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        bail!("微信 CDN 地址不能包含登录凭据或片段标识");
    }
    let official = url.scheme() == "https"
        && url.host_str() == Some("novac2c.cdn.weixin.qq.com")
        && url.port_or_known_default() == Some(443);
    let local = local_origin.is_some_and(|base| {
        is_loopback(url)
            && matches!(url.scheme(), "http" | "https")
            && url.origin() == base.origin()
    });
    if !official && !local {
        bail!("微信媒体地址必须使用官方 CDN 或显式配置的本地回环端点");
    }
    Ok(())
}

fn network_error(error: reqwest::Error) -> anyhow::Error {
    // Do not retain reqwest's URL or server text: both can contain media keys.
    if error.is_timeout() {
        anyhow::anyhow!("微信 CDN 请求超时")
    } else {
        anyhow::anyhow!("微信 CDN 网络请求失败")
    }
}

async fn read_bytes_limited(
    response: reqwest::Response,
    max_bytes: usize,
) -> anyhow::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|size| size > max_bytes as u64)
    {
        bail!("微信 CDN 响应超过附件大小限制");
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(network_error)?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            bail!("微信 CDN 响应超过附件大小限制");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn attachment_items(message: &WeixinMessage) -> (Vec<&WeixinMessageItem>, bool) {
    let mut items = Vec::new();
    let mut truncated = false;
    for item in &message.item_list {
        let mut current = Some(item);
        let mut depth = 0;
        while let Some(item) = current {
            if matches!(item.item_type, 2 | 4) {
                if items.len() == MAX_ATTACHMENTS {
                    return (items, true);
                }
                items.push(item);
            }
            current = item
                .ref_msg
                .as_ref()
                .and_then(|reference| reference.message_item.as_deref());
            if depth == MAX_REFERENCE_DEPTH {
                truncated |= current.is_some();
                break;
            }
            depth += 1;
        }
    }
    (items, truncated)
}

pub(super) fn has_attachments(message: &WeixinMessage) -> bool {
    !attachment_items(message).0.is_empty()
}

fn safe_filename(filename: &str) -> String {
    let basename = filename.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut result = String::new();
    for ch in basename.chars() {
        if result.len() + ch.len_utf8() > 160 {
            break;
        }
        result.push(if ch.is_control() || "<>:\"|?*".contains(ch) {
            '_'
        } else {
            ch
        });
    }
    let result = result.trim_matches([' ', '.']);
    if result.is_empty() {
        "file.bin".into()
    } else {
        result.into()
    }
}

fn image_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "jpg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else if bytes.starts_with(b"II\x2a\0") || bytes.starts_with(b"MM\0\x2a") {
        "tiff"
    } else {
        "img"
    }
}
