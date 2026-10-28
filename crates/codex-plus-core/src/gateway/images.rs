//! 生图：Agent 发来的 `/images/generations`、`/images/edits` 发给哪家。
//!
//! 生图模型不参与聊天，不进 Agent 的模型列表，在「网关设置」里单独选：
//! - 自动（默认）：池里的 ChatGPT 订阅账号优先（后端自己选生图模型，订阅里就能用），
//!   其次是模型列表里有生图模型的供应商，按池里的顺序依次试；
//! - 指定 `<供应商 id>/<模型>`：只发给这一家，请求里的模型名换成它；
//! - `off`：关闭，生图请求直接报错。

use serde::Serialize;

use crate::gateway::store::GatewaySettings;
use crate::settings::RelayProfile;

/// 关闭生图时存的值。
pub const IMAGE_OFF: &str = "off";

/// 生图模型的名字特征。只用来把它们挑出来：不进聊天模型列表、列在生图模型的选项里。
const IMAGE_MODEL_MARKS: &[&str] = &[
    "gpt-image",
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
    "qwen-image",
    "hunyuan-image",
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

/// ChatGPT 订阅账号那一项显示的模型名。
const CHATGPT_IMAGE_LABEL: &str = "gpt-image";

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

/// 按设置排出这次生图要依次尝试的上游。返回空表示关闭了或没有能画图的供应商，
/// 错误信息说明原因。
pub fn targets(gateway: &GatewaySettings) -> Result<Vec<ImageTarget>, String> {
    let wanted = gateway.image_model.trim();
    if wanted.eq_ignore_ascii_case(IMAGE_OFF) {
        return Err("Codex++ 网关的生图已关闭：在「网关设置」里选一个生图模型".to_string());
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
        let targets: Vec<ImageTarget> = all
            .iter()
            .filter(|option| option.vendor_picks)
            .chain(all.iter().filter(|option| !option.vendor_picks))
            .filter_map(target)
            .collect();
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
            "生图模型 {wanted} 所在的供应商已删除或关闭：在「网关设置」里换一个"
        )),
    }
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
        let picked: Vec<(String, Option<String>)> = targets(&gateway)
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
        assert!(targets(&gateway).unwrap_err().contains("没有能生图的模型"));
        gateway.image_model = IMAGE_OFF.into();
        assert!(targets(&gateway).unwrap_err().contains("已关闭"));
        gateway.image_model = "gone/gpt-image-1".into();
        assert!(targets(&gateway).unwrap_err().contains("已删除或关闭"));
        // 名字认不出，但供应商在：照样发给它。
        gateway.image_model = "ds/my-painter".into();
        let picked = targets(&gateway).unwrap();
        assert_eq!(picked[0].relay.id, "ds");
        assert_eq!(picked[0].model.as_deref(), Some("my-painter"));
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
