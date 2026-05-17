use serde_json::{Value, json};

pub const DEFAULT_AD_LIST_URLS: [&str; 2] = [
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
                && url.is_some_and(|value| !value.trim().is_empty())
        })
        .cloned()
        .collect::<Vec<_>>();
    json!({ "version": version, "ads": ads })
}

pub async fn fetch_ad_list() -> anyhow::Result<Value> {
    fetch_ad_list_from_urls(&DEFAULT_AD_LIST_URLS).await
}

pub async fn fetch_ad_list_from_urls<S>(urls: &[S]) -> anyhow::Result<Value>
where
    S: AsRef<str>,
{
    let client = reqwest::Client::builder()
        .user_agent("CodexPlusPlus")
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let mut last_error = None;
    for url in urls {
        let url = url.as_ref();
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
