use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

const VOLCENGINE_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-volcengine.png");
const PACKYCODE_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-packycode.png");
const TOKEN_BRIDGE_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-0029.svg");
const APIKEY_FUN_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-apikey-fun.png");
const RAWCHAT_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-rawchat.svg");
const RUNAPI_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-runapi.png");
const BAIKEWEI_AI_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-baikewei-ai.jpg");
const DEEPKEY_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-deepkey.png");
const APIMART_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-apimart.png");
const FENNO_AI_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-fenno-ai.png");
const QINIU_AI_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-qiniu-ai.png");

pub const DEFAULT_AD_LIST_URLS: [&str; 2] = [
    "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
    "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json",
];

pub fn normalize_ad_payload(payload: Value) -> Value {
    let version = payload.get("version").and_then(Value::as_u64).unwrap_or(1);
    let mut ads = payload
        .get("ads")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|ad| {
            let ad_type = ad.get("type").and_then(Value::as_str);
            let title = ad.get("title").and_then(Value::as_str);
            let description = ad.get("description").and_then(Value::as_str);
            let url = ad.get("url").and_then(Value::as_str);
            matches!(ad_type, Some("sponsor" | "normal"))
                && title.is_some_and(|value| !value.trim().is_empty())
                && description.is_some_and(|value| !value.trim().is_empty())
                && url.is_some_and(|value| !value.trim().is_empty())
        })
        .cloned()
        .collect::<Vec<_>>();
    fill_known_remote_logos(&mut ads);
    // `topAd` 是独立的置顶赞助位，**不参与** `ads` 列表的排序与过期过滤语义。
    // 它比普通推荐贵，由商务单独指定，所以不能混在推荐池里按数组顺序取。
    let top_ad = payload
        .get("top_ad")
        .or_else(|| payload.get("topAd"))
        .filter(|value| is_usable_ad(value))
        .cloned();
    match top_ad {
        Some(top_ad) => json!({ "version": version, "ads": ads, "topAd": top_ad }),
        None => json!({ "version": version, "ads": ads }),
    }
}

/// 一条广告是否可用（类型、标题、描述、URL 齐全）。
fn is_usable_ad(ad: &Value) -> bool {
    let ad_type = ad.get("type").and_then(Value::as_str);
    let field = |key: &str| {
        ad.get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    };
    matches!(ad_type, Some("sponsor" | "normal"))
        && field("title")
        && field("description")
        && field("url")
}

fn fill_known_remote_logos(ads: &mut [Value]) {
    for ad in ads {
        let Some(object) = ad.as_object_mut() else {
            continue;
        };
        let has_image = object
            .get("image")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        if has_image {
            continue;
        }
        let Some(id) = object.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some((mime, image)) = known_remote_logo(id) else {
            continue;
        };
        object.insert("image".to_string(), json!(data_uri(mime, image)));
    }
}

fn known_remote_logo(id: &str) -> Option<(&'static str, &'static [u8])> {
    match id {
        "volcengine-ark-agent-plan" => Some(("image/png", VOLCENGINE_IMAGE)),
        "0029-token-bridge" => Some(("image/png", PACKYCODE_IMAGE)),
        "0055-token-bridge" => Some(("image/svg+xml", TOKEN_BRIDGE_IMAGE)),
        "apikey-fun-ai-relay" => Some(("image/png", APIKEY_FUN_IMAGE)),
        "rawchat-codex-relay" => Some(("image/svg+xml", RAWCHAT_IMAGE)),
        "runapi-openrouter-alternative" => Some(("image/png", RUNAPI_IMAGE)),
        "baikewei-ai" => Some(("image/jpeg", BAIKEWEI_AI_IMAGE)),
        "deepkey-api-key" => Some(("image/png", DEEPKEY_IMAGE)),
        "apimart" => Some(("image/png", APIMART_IMAGE)),
        "fenno-ai" => Some(("image/png", FENNO_AI_IMAGE)),
        "qiniu-ai" => Some(("image/png", QINIU_AI_IMAGE)),
        _ => None,
    }
}

fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64_encode(bytes))
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }
    encoded
}

pub async fn fetch_ad_list() -> anyhow::Result<Value> {
    fetch_ad_list_from_urls(&DEFAULT_AD_LIST_URLS).await
}

pub fn cache_busted_ad_url(url: &str, version: u128) -> String {
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}v={version}")
}

pub async fn fetch_ad_list_from_urls<S>(urls: &[S]) -> anyhow::Result<Value>
where
    S: AsRef<str>,
{
    let client = crate::http_client::proxied_client("CodexPlusPlus")?;
    let cache_bust = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let mut last_error = None;
    for url in urls {
        let url = cache_busted_ad_url(url.as_ref(), cache_bust);
        let result = async {
            let response = client.get(url).send().await?.error_for_status()?;
            let payload = response.json::<Value>().await?;
            Ok::<_, anyhow::Error>(normalize_ad_payload(payload))
        }
        .await;
        match result {
            Ok(payload) => return Ok(payload),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("ad list unavailable")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sponsored(id: &str) -> Value {
        json!({
            "id": id,
            "type": "sponsor",
            "title": id,
            "description": "描述",
            "url": "https://example.com/",
        })
    }

    #[test]
    fn top_ad_is_kept_out_of_the_recommendation_pool() {
        let normalized = normalize_ad_payload(json!({
            "version": 1,
            "ads": [sponsored("pool-a")],
            "top_ad": sponsored("premium"),
        }));

        // 贵价置顶位有自己的字段，不能混进 ads 数组参与排序。
        assert_eq!(normalized["topAd"]["id"], json!("premium"));
        let pool = normalized["ads"].as_array().unwrap();
        assert!(
            !pool.iter().any(|ad| ad["id"] == json!("premium")),
            "置顶位不应出现在推荐池里"
        );
    }

    #[test]
    fn top_ad_accepts_camel_case() {
        let camel = normalize_ad_payload(json!({
            "version": 1,
            "ads": [],
            "topAd": sponsored("premium"),
        }));
        assert_eq!(camel["topAd"]["id"], json!("premium"));
    }

    #[test]
    fn missing_or_unusable_top_ad_is_omitted() {
        for payload in [
            json!({ "version": 1, "ads": [] }),
            json!({
                "version": 1,
                "ads": [],
                "top_ad": { "type": "sponsor", "title": "残缺", "description": "描述" },
            }),
        ] {
            let normalized = normalize_ad_payload(payload);
            assert!(normalized.get("topAd").is_none());
        }
    }
}
