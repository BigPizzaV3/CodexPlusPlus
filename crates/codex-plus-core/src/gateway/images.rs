//! 生图：Agent 发来的 `/images/generations`、`/images/edits` 发给哪家。
//!
//! 生图模型不参与聊天，不进 Agent 的模型列表，在「Agent 接入」里按 Agent 单独选：
//! - 自动（默认）：池里的 ChatGPT 订阅账号优先（后端自己选生图模型，订阅里就能用），
//!   其次是模型列表里有生图模型的供应商，按池里的顺序每家试一次：这家有请求里写的模型
//!   （如 Codex 的 `gpt-image-2`）就用它，没有才用这家的第一个生图模型；
//! - 指定 `<供应商 id>/<模型>`：只发给这一家，请求里的模型名换成它；
//! - `off`：关闭，生图请求直接报错。

use serde::Serialize;

use crate::gateway::store::GatewaySettings;
use crate::settings::RelayProfile;
use crate::tools::ToolId;

/// 关闭生图时存的值。
pub const IMAGE_OFF: &str = "off";

/// 生图模型的名字特征。只用来把它们挑出来：不进聊天模型列表、列在生图模型的选项里。
/// 名字里带 `image` / `imagine` 的（gpt-image-2、grok-imagine-image、qwen-image）都算。
const IMAGE_MODEL_MARKS: &[&str] = &[
    "image",
    "imagine",
    "dall-e",
    "imagen",
    "flux",
    "seedream",
    "cogview",
    "wanx",
    "kolors",
    "stable-diffusion",
    "sdxl",
    "midjourney",
];

/// 按名字判断是不是生图模型（不分大小写，忽略供应商前缀）。
pub fn is_image_model(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    let name = model.rsplit('/').next().unwrap_or(&model);
    IMAGE_MODEL_MARKS.iter().any(|mark| name.contains(mark))
}

/// ChatGPT 订阅账号：走 ChatGPT 后端的生图接口，模型由后端自己选。
fn is_chatgpt_account(relay: &RelayProfile) -> bool {
    crate::official_login::member_uses_official_login_transport(relay)
}

/// 界面上「生图模型」下拉的一项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageOption {
    /// 存进 `image_model` 的值：`<供应商 id>/<模型>`。
    pub id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    /// ChatGPT 订阅账号：后端自己选模型，名字只是个标签。
    pub vendor_picks: bool,
}

/// ChatGPT 订阅账号那一项显示的模型名：后端的模型目录里没有生图模型，这是 Codex CLI 生图时用的名字。
/// 后端自己选模型，写什么都一样。
const CHATGPT_IMAGE_LABEL: &str = "gpt-image-2";

/// 一家供应商能画图的模型：ChatGPT 账号是内置的一项，其余按名字从模型列表里挑。
pub fn provider_image_models(relay: &RelayProfile) -> Vec<String> {
    if is_chatgpt_account(relay) {
        return vec![CHATGPT_IMAGE_LABEL.to_string()];
    }
    crate::gateway::route::relay_models(relay)
        .into_iter()
        .filter(|model| is_image_model(model))
        .collect()
}

/// 能画图的选项：打开着的 ChatGPT 订阅账号各一项，再加上各家模型列表里的生图模型。
pub fn options(gateway: &GatewaySettings) -> Vec<ImageOption> {
    let mut options = Vec::new();
    for provider in gateway.providers.iter().filter(|provider| provider.enabled) {
        let relay = &provider.relay;
        if is_chatgpt_account(relay) {
            options.push(ImageOption {
                id: format!("{}/{CHATGPT_IMAGE_LABEL}", relay.id),
                provider_id: relay.id.clone(),
                provider_name: relay.name.clone(),
                model: CHATGPT_IMAGE_LABEL.to_string(),
                vendor_picks: true,
            });
            continue;
        }
        for model in crate::gateway::route::relay_models(relay) {
            if is_image_model(&model) {
                options.push(ImageOption {
                    id: format!("{}/{model}", relay.id),
                    provider_id: relay.id.clone(),
                    provider_name: relay.name.clone(),
                    model,
                    vendor_picks: false,
                });
            }
        }
    }
    options
}

/// 一个生图上游：发给哪家、请求里的模型名换成什么（None 表示保留 Agent 自己写的）。
#[derive(Debug, Clone)]
pub struct ImageTarget {
    pub relay: RelayProfile,
    pub model: Option<String>,
}

/// 按这个 Agent 的设置排出这次生图要依次尝试的上游。`requested` 是请求里写的模型名。
/// 返回错误表示关闭了或没有能画图的供应商，错误信息说明原因。
pub fn targets(
    gateway: &GatewaySettings,
    agent: &ToolId,
    requested: Option<&str>,
) -> Result<Vec<ImageTarget>, String> {
    let wanted = gateway.image_model_for(agent).trim();
    if wanted.eq_ignore_ascii_case(IMAGE_OFF) {
        return Err(
            "Codex++ 网关里这个 Agent 的生图已关闭：在「Agent 接入」里选一个生图模型".to_string(),
        );
    }
    let target = |option: &ImageOption| {
        gateway
            .provider(&option.provider_id)
            .map(|provider| ImageTarget {
                relay: provider.relay.clone(),
                model: (!option.vendor_picks).then(|| option.model.clone()),
            })
    };
    let all = options(gateway);
    if wanted.is_empty() {
        // ChatGPT 账号在前；其余每家只试一次（同一家换个模型多半也是同样的错）。
        let mut targets: Vec<ImageTarget> = all
            .iter()
            .filter(|option| option.vendor_picks)
            .filter_map(target)
            .collect();
        let mut seen: Vec<&str> = Vec::new();
        for option in all.iter().filter(|option| !option.vendor_picks) {
            if seen.contains(&option.provider_id.as_str()) {
                continue;
            }
            seen.push(&option.provider_id);
            let asked = requested.map(str::trim).and_then(|requested| {
                all.iter().find(|candidate| {
                    candidate.provider_id == option.provider_id
                        && candidate.model.eq_ignore_ascii_case(requested)
                })
            });
            targets.extend(target(asked.unwrap_or(option)));
        }
        if targets.is_empty() {
            return Err(
                "Codex++ 网关的供应商池里没有能生图的模型：登录一个 ChatGPT 账号，或给供应商加上生图模型（如 gpt-image-1）"
                    .to_string(),
            );
        }
        return Ok(targets);
    }
    if let Some(option) = all
        .iter()
        .find(|option| option.id.eq_ignore_ascii_case(wanted))
    {
        return Ok(target(option).into_iter().collect());
    }
    // 不在列表里的（名字认不出是生图模型，或那家已关闭 / 删除）：供应商还在且打开着就照样发。
    let (provider_id, model) = wanted.split_once('/').unwrap_or(("", wanted));
    match gateway
        .providers
        .iter()
        .find(|provider| provider.enabled && provider.relay.id == provider_id)
    {
        Some(provider) => Ok(vec![ImageTarget {
            relay: provider.relay.clone(),
            model: (!is_chatgpt_account(&provider.relay)).then(|| model.to_string()),
        }]),
        None => Err(format!(
            "生图模型 {wanted} 所在的供应商已删除或关闭：在「Agent 接入」里换一个"
        )),
    }
}

/// 请求里写的模型名：JSON 的 `model` 字段，或 multipart 里 `model` 那一段的值。
pub fn requested_model(body: &[u8], content_type: &str) -> Option<String> {
    if content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        let at = find(body, b"name=\"model\"", 0)?;
        let start = find(body, b"\r\n\r\n", at)? + 4;
        let end = find(body, b"\r\n--", start)?;
        return std::str::from_utf8(&body[start..end])
            .ok()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string);
    }
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()?
        .get("model")?
        .as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
}

/// 自动时这次会先用哪一项（界面上显示「自动 · xxx」）。
pub fn auto_choice(gateway: &GatewaySettings) -> Option<ImageOption> {
    let all = options(gateway);
    all.iter()
        .find(|option| option.vendor_picks)
        .or(all.first())
        .cloned()
}

/// 请求里的模型名换成选中的生图模型。JSON（generations）改 `model` 字段；multipart（edits）
/// 改 `model` 那一段的值，没有这一段就不动（上游按自己的默认模型画）。
pub fn rewrite_model(body: &[u8], content_type: &str, model: &str) -> Vec<u8> {
    if content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return rewrite_multipart_model(body, model).unwrap_or_else(|| body.to_vec());
    }
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(mut value) if value.is_object() => {
            value["model"] = serde_json::Value::String(model.to_string());
            serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec())
        }
        _ => body.to_vec(),
    }
}

fn rewrite_multipart_model(body: &[u8], model: &str) -> Option<Vec<u8>> {
    let marker = b"name=\"model\"";
    let at = find(body, marker, 0)?;
    // 段头和值之间是一个空行；值一直到下一个 `\r\n--` 边界。
    let start = find(body, b"\r\n\r\n", at)? + 4;
    let end = find(body, b"\r\n--", start)?;
    let mut out = Vec::with_capacity(body.len() + model.len());
    out.extend_from_slice(&body[..start]);
    out.extend_from_slice(model.as_bytes());
    out.extend_from_slice(&body[end..]);
    Some(out)
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|index| index + from)
}

/// 生图返回里只给了链接（`url`）、没给图片数据（`b64_json`）的，把图下载下来补成 `b64_json`。
/// Codex 只认 `b64_json`（缺了会报 missing field `b64_json`），而不少中转站默认返回链接。
/// `data:` 链接直接取出里面的数据；下载失败的那张原样保留，交给 Agent 自己报错。
pub async fn inline_image_urls(body: Vec<u8>, client: &reqwest::Client) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return body;
    };
    let Some(items) = value
        .get_mut("data")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return body;
    };
    let mut changed = false;
    for item in items.iter_mut() {
        let has_data = item
            .get("b64_json")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|data| !data.is_empty());
        let Some(url) = item.get("url").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if has_data {
            continue;
        }
        let data = if let Some(rest) = url.strip_prefix("data:") {
            rest.split_once(";base64,")
                .map(|(_, data)| data.to_string())
        } else {
            download_base64(client, url).await
        };
        if let Some(data) = data {
            item["b64_json"] = serde_json::Value::String(data);
            changed = true;
        }
    }
    if !changed {
        return body;
    }
    serde_json::to_vec(&value).unwrap_or(body)
}

async fn download_base64(client: &reqwest::Client, url: &str) -> Option<String> {
    use base64::Engine;
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 这个上游答复是否该换下一家再试：鉴权失败、限流、上游出错。
pub fn should_try_next(status: u16) -> bool {
    matches!(status, 401 | 403 | 404 | 429) || status >= 500
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::store::GatewayProvider;

    fn provider(id: &str, models: &str) -> GatewayProvider {
        GatewayProvider {
            enabled: true,
            source: String::new(),
            price: None,
            model_prices: Default::default(),
            proxy: String::new(),
            upstream_new: Vec::new(),
            upstream_gone: Vec::new(),
            models_checked_at_ms: 0,
            model_windows: Default::default(),
            context_window: None,
            relay: RelayProfile {
                id: id.to_string(),
                name: id.to_string(),
                base_url: format!("https://{id}.test/v1"),
                api_key: format!("sk-{id}"),
                model: models.lines().next().unwrap_or_default().to_string(),
                model_list: models.to_string(),
                ..RelayProfile::default()
            },
            extra: Default::default(),
        }
    }

    #[test]
    fn image_models_are_told_apart_by_name() {
        for model in [
            "gpt-image-1",
            "openai/dall-e-3",
            "imagen-4.0",
            "FLUX.1-dev",
            "doubao-seedream-4-0",
        ] {
            assert!(is_image_model(model), "{model}");
        }
        for model in ["gpt-5.5", "deepseek-v4-pro", "kimi-k3", "claude-opus-5-5"] {
            assert!(!is_image_model(model), "{model}");
        }
    }

    #[test]
    fn auto_tries_every_provider_with_an_image_model_in_pool_order() {
        let gateway = GatewaySettings {
            providers: vec![
                provider("ds", "deepseek-v4-pro"),
                provider("oa", "gpt-5.5\ngpt-image-1"),
                provider("sf", "flux-1-schnell"),
            ],
            ..GatewaySettings::default()
        };
        let picked: Vec<(String, Option<String>)> = targets(&gateway, &ToolId::Codex, None)
            .unwrap()
            .into_iter()
            .map(|target| (target.relay.id, target.model))
            .collect();
        assert_eq!(
            picked,
            [
                ("oa".to_string(), Some("gpt-image-1".to_string())),
                ("sf".to_string(), Some("flux-1-schnell".to_string())),
            ]
        );
        assert_eq!(auto_choice(&gateway).unwrap().id, "oa/gpt-image-1");
    }

    #[test]
    fn picked_off_or_missing_models_explain_themselves() {
        let mut gateway = GatewaySettings {
            providers: vec![provider("ds", "deepseek-v4-pro")],
            ..GatewaySettings::default()
        };
        let codex = ToolId::Codex;
        let pick = |gateway: &mut GatewaySettings, value: &str| {
            gateway
                .image_models
                .insert("codex".to_string(), value.to_string());
        };
        assert!(
            targets(&gateway, &codex, None)
                .unwrap_err()
                .contains("没有能生图的模型")
        );
        pick(&mut gateway, IMAGE_OFF);
        assert!(
            targets(&gateway, &codex, None)
                .unwrap_err()
                .contains("已关闭")
        );
        // 关的是 Codex 的，别的 Agent 不受影响（仍是自动，池里没有就报没有）。
        assert!(
            targets(&gateway, &ToolId::Grok, None)
                .unwrap_err()
                .contains("没有能生图的模型")
        );
        pick(&mut gateway, "gone/gpt-image-1");
        assert!(
            targets(&gateway, &codex, None)
                .unwrap_err()
                .contains("已删除或关闭")
        );
        // 名字认不出，但供应商在：照样发给它。
        pick(&mut gateway, "ds/my-painter");
        let picked = targets(&gateway, &codex, None).unwrap();
        assert_eq!(picked[0].relay.id, "ds");
        assert_eq!(picked[0].model.as_deref(), Some("my-painter"));
    }

    #[test]
    fn auto_tries_each_provider_once_and_keeps_the_model_asked_for() {
        let gateway = GatewaySettings {
            providers: vec![
                provider(
                    "jo",
                    "gpt-6.1-sol\ngpt-image-2-4k\ngpt-image-2\ngpt-image-2.5-flare",
                ),
                provider("sf", "flux-1-schnell\nflux-1-dev"),
            ],
            ..GatewaySettings::default()
        };
        let picked: Vec<(String, Option<String>)> =
            targets(&gateway, &ToolId::Codex, Some("GPT-Image-2"))
                .unwrap()
                .into_iter()
                .map(|target| (target.relay.id, target.model))
                .collect();
        assert_eq!(
            picked,
            [
                ("jo".to_string(), Some("gpt-image-2".to_string())),
                ("sf".to_string(), Some("flux-1-schnell".to_string())),
            ],
            "每家一次；有请求里写的就用它，没有用这家的第一个"
        );
    }

    #[test]
    fn requested_model_is_read_from_json_and_multipart() {
        assert_eq!(
            requested_model(br#"{"model":"gpt-image-2"}"#, "application/json").as_deref(),
            Some("gpt-image-2")
        );
        let multipart = b"--b\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\ngpt-image-2\r\n--b--\r\n";
        assert_eq!(
            requested_model(multipart, "multipart/form-data; boundary=b").as_deref(),
            Some("gpt-image-2")
        );
        assert_eq!(requested_model(b"{}", "application/json"), None);
    }

    #[tokio::test]
    async fn url_only_images_get_their_data_inlined() {
        use base64::Engine;
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut stream, _) = server.accept().await.unwrap();
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPNG!")
                .await;
        });
        let body = serde_json::json!({ "data": [
            { "url": format!("http://{address}/cat.png") },
            { "url": "data:image/png;base64,QUJD" },
            { "b64_json": "already", "url": "http://unused" }
        ]});
        let out =
            inline_image_urls(serde_json::to_vec(&body).unwrap(), &reqwest::Client::new()).await;
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            value["data"][0]["b64_json"],
            base64::engine::general_purpose::STANDARD.encode(b"PNG!")
        );
        assert_eq!(value["data"][1]["b64_json"], "QUJD");
        assert_eq!(value["data"][2]["b64_json"], "already");
        // 不是 JSON 的原样返回。
        assert_eq!(
            inline_image_urls(b"oops".to_vec(), &reqwest::Client::new()).await,
            b"oops"
        );
    }

    #[test]
    fn model_is_rewritten_in_json_and_multipart_bodies() {
        let json = rewrite_model(
            br#"{"model":"gpt-image-2","prompt":"cat"}"#,
            "application/json",
            "flux-1",
        );
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["model"], "flux-1");
        assert_eq!(value["prompt"], "cat");

        let multipart = b"--b\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\ngpt-image-2\r\n--b\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\ncat\r\n--b--\r\n";
        let out = rewrite_model(multipart, "multipart/form-data; boundary=b", "flux-1");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("name=\"model\"\r\n\r\nflux-1\r\n--b"));
        assert!(text.contains("cat"));
    }
}
