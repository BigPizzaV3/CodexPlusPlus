use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_AD_LIST_URLS: [&str; 3] = [
    "https://ads.codexpp.cc/ads.json",
    "https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json",
    "https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json",
];

pub fn normalize_ad_payload(payload: Value) -> Value {
    let version = payload.get("version").and_then(Value::as_u64).unwrap_or(1);
    let ads = payload
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
                && url.is_some_and(is_web_url)
        })
        .cloned()
        .collect::<Vec<_>>();
    // `topAd` 是独立的置顶赞助位，**不参与** `ads` 列表的排序与过期过滤语义。
    // 同时兼容单条对象和多条数组，数组用于管理工具的横幅轮播。
    let raw_top_ads = payload
        .get("top_ad")
        .or_else(|| payload.get("topAd"));
    let top_ads = match raw_top_ads {
        Some(Value::Array(items)) => items
            .iter()
            .filter(|ad| is_usable_ad(ad))
            .cloned()
            .collect::<Vec<_>>(),
        Some(value) if is_usable_ad(value) => vec![value.clone()],
        _ => Vec::new(),
    };

    let mut normalized = json!({ "version": version, "ads": ads });
    if !top_ads.is_empty() {
        normalized["topAd"] = top_ads[0].clone();
        normalized["topAds"] = json!(top_ads);
    }
    normalized
}

/// 广告落地页只接受 http(s) 外链。
///
/// 广告清单是远程内容，`javascript:` / `data:` / `file:` 一旦漏进 `<a href>`，
/// 点击就会在 Codex 渲染进程里执行脚本。管理端打开链接的入口已经只认
/// http(s)，这里在归一层再挡一次，保证注入端拿到的数据本身就是干净的。
fn is_web_url(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"));
    rest.is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
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
        && ad.get("url").and_then(Value::as_str).is_some_and(is_web_url)
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
    fn top_ad_accepts_multiple_usable_ads_for_rotation() {
        let normalized = normalize_ad_payload(json!({
            "version": 1,
            "ads": [],
            "top_ad": [
                sponsored("premium-a"),
                { "type": "sponsor", "title": "残缺", "description": "描述" },
                sponsored("premium-b"),
            ],
        }));

        assert_eq!(normalized["topAd"]["id"], json!("premium-a"));
        assert_eq!(normalized["topAds"].as_array().unwrap().len(), 2);
        assert_eq!(normalized["topAds"][1]["id"], json!("premium-b"));
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
