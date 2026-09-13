use serde_json::{Map, Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

const VOLCENGINE_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-volcengine.png");
const PACKYCODE_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-packycode.png");
const TOKEN_BRIDGE_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-0029.svg");
const APIKEY_FUN_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-apikey-fun.png");
const RAWCHAT_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-rawchat.svg");
const RUNAPI_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-runapi.png");
const BAIKEWEI_AI_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-baikewei-ai.jpg");
const CUBENCE_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-cubence.png");
const DEEPKEY_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-deepkey.png");
const ERGOU_API_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-ergou-api.png");
const APIMART_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-apimart.png");
const FENNO_AI_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-fenno-ai.png");
const QINIU_AI_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-qiniu-ai.png");
const JOJOCODE_IMAGE: &[u8] = include_bytes!("../../../docs/images/sponsor-jojocode.png");
const BUILTIN_SPONSOR_EXPIRES_AT: &str = "2026-08-02T23:59:59+08:00";
const DEEPKEY_SPONSOR_EXPIRES_AT: &str = "2026-08-25T23:59:59+08:00";
const APIMART_SPONSOR_EXPIRES_AT: &str = "2026-09-27T23:59:59+08:00";
const NEW_SPONSOR_EXPIRES_AT: &str = "2026-11-27T23:59:59+08:00";

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
    append_builtin_sponsors(&mut ads);
    // `topAd` 是独立的置顶赞助位，**不参与** `ads` 列表的排序与过期过滤语义。
    // 它比普通推荐贵，由商务单独指定，所以不能混在推荐池里按数组顺序取。
    let top_ad = payload
        .get("top_ad")
        .or_else(|| payload.get("topAd"))
        .filter(|value| is_usable_ad(value))
        .cloned()
        // 广告源没配置顶位时用内置的贵价赞助位兜底，保证概览这块不空。
        .or_else(builtin_top_ad);
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

fn append_builtin_sponsors(ads: &mut Vec<Value>) {
    let insert_at = ads
        .iter()
        .rposition(|ad| ad.get("type").and_then(Value::as_str) == Some("sponsor"))
        .map(|index| index + 1)
        .unwrap_or(0);
    let builtins = [
        builtin_sponsor(
            "cubence",
            "Cubence",
            "稳定、高效的 API 中转服务，支持 Claude Code、Codex、Gemini 等模型，适合日常开发和团队使用。",
            "https://cubence.com?source=codexplusplus",
            CUBENCE_IMAGE,
            "image/png",
            &["Claude Code", "Codex / Gemini", "稳定接入"],
            BUILTIN_SPONSOR_EXPIRES_AT,
        ),
        builtin_sponsor(
            "quya-cloud-bridge",
            "quya.org 云桥",
            "一站式 AI 中转平台，集成 Claude Code、Codex、Gemini 等模型，提供包月和按量计费方案。",
            "https://www.quya.org/?promo=CODEX",
            TOKEN_BRIDGE_IMAGE,
            "image/svg+xml",
            &[
                "Claude Code / Codex / Gemini",
                "国内直连",
                "包月 / 按量计费",
            ],
            BUILTIN_SPONSOR_EXPIRES_AT,
        ),
        builtin_sponsor(
            "deepkey-api-key",
            "deepkey｜API KEY",
            "面向开发者与学生群体的 API KEY 服务，提供稳定接口和提示词工程交流社区。",
            "https://deepkey.top/register?aff=DNVc",
            DEEPKEY_IMAGE,
            "image/png",
            &["稳定接口", "开发者社区", "提示词交流"],
            DEEPKEY_SPONSOR_EXPIRES_AT,
        ),
        builtin_sponsor(
            "ergou-api",
            "二狗 API",
            "AI API 中转服务，覆盖 Claude、GPT、Gemini 等模型，提供低延迟线路和备用链路。",
            "https://ergouapi.com/r/gh-codexplusplus",
            ERGOU_API_IMAGE,
            "image/png",
            &["Claude / GPT / Gemini", "低延迟线路", "备用链路"],
            BUILTIN_SPONSOR_EXPIRES_AT,
        ),
        builtin_sponsor(
            "apimart",
            "API Mart",
            "专注 AI 图片和视频生成的低价 API 平台，一套异步 API 覆盖图片与视频任务，支持大批量生成和按量付费。",
            "https://go.apimart.ai/gh-codexplusplus",
            APIMART_IMAGE,
            "image/png",
            &["图片 / 视频", "异步任务 API", "按量付费"],
            APIMART_SPONSOR_EXPIRES_AT,
        ),
        builtin_sponsor(
            "fenno-ai",
            "FennoAI",
            "稳定高效的 Codex API 中转服务，兼容 OpenAI 与 Anthropic 协议，支持企业级调用、公对公结算和开票。",
            "https://api.fenno.ai/s/ZZM7",
            FENNO_AI_IMAGE,
            "image/png",
            &["Codex 中转", "企业级调用", "公对公结算"],
            NEW_SPONSOR_EXPIRES_AT,
        ),
        builtin_sponsor(
            "qiniu-ai",
            "七牛云",
            "七牛云旗下企业级大模型 MaaS 平台，一站式调用全球 150 多个主流模型，覆盖文本、图像、音频、视频等全模态能力。",
            "https://s.qiniu.com/7zUJri",
            QINIU_AI_IMAGE,
            "image/png",
            &["150+ 主流模型", "全模态能力", "企业免费额度"],
            NEW_SPONSOR_EXPIRES_AT,
        ),
    ];
    let mut cursor = insert_at;
    for sponsor in builtins {
        let id = sponsor.get("id").and_then(Value::as_str);
        if id.is_some_and(|id| {
            ads.iter()
                .any(|ad| ad.get("id").and_then(Value::as_str) == Some(id))
        }) {
            continue;
        }
        ads.insert(cursor, sponsor);
        cursor += 1;
    }
}

fn builtin_sponsor(
    id: &str,
    title: &str,
    description: &str,
    url: &str,
    image: &[u8],
    image_mime: &str,
    highlights: &[&str],
    expires_at: &str,
) -> Value {
    let mut sponsor = Map::new();
    sponsor.insert("id".to_string(), json!(id));
    sponsor.insert("type".to_string(), json!("sponsor"));
    sponsor.insert("title".to_string(), json!(title));
    sponsor.insert("description".to_string(), json!(description));
    sponsor.insert("url".to_string(), json!(url));
    sponsor.insert("expires_at".to_string(), json!(expires_at));
    sponsor.insert("image".to_string(), json!(data_uri(image_mime, image)));
    sponsor.insert("highlights".to_string(), json!(highlights));
    Value::Object(sponsor)
}

/// 赞助位是否已过期。
///
/// 支持 `2027-06-15T23:59:59+08:00` 和 `2027-06-15` 两种写法；解析不出来时
/// 按「未过期」处理 —— 宁可多显示一天，也不因为格式问题把付费位吞掉。
/// 前端也有一份同样的判断，这里是后端侧的兜底。
fn is_expired_at(expires_at: &str) -> bool {
    let Some(expiry) = ymd_to_days(expires_at.split('T').next().unwrap_or(expires_at)) else {
        return false;
    };
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.as_secs() / 86_400) as i64)
        .unwrap_or(0);
    expiry < today
}

/// `YYYY-MM-DD` → 自 1970-01-01 起的天数（Howard Hinnant 的 days_from_civil）。
fn ymd_to_days(date: &str) -> Option<i64> {
    let trimmed = date.trim();
    let mut parts = trimmed.split('-');
    let year: i64 = parts.next()?.trim().parse().ok()?;
    let month: i64 = parts.next()?.trim().parse().ok()?;
    let day: i64 = parts.next()?.trim().parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// 内置的置顶赞助位（贵价位，单独指定，不参与推荐池排序）。
///
/// 有效期由 `JOJOCODE_TOP_AD_EXPIRES_AT` 控制；过期后概览页会自动回落到
/// 广告源或内置兜底，不会一直挂着一个已经结束的赞助。
fn builtin_top_ad() -> Option<Value> {
    let expires_at = "2027-06-15T23:59:59+08:00";
    if is_expired_at(expires_at) {
        return None;
    }
    Some(json!({
        "id": "jojocode-top",
        "type": "sponsor",
        "title": "JOJO Code",
        "description": "JOJO Code 提供稳定、价格合理的 API 中转服务，支持 GPT-5.6 全系列、Fable 5、Sonnet 5、GPT-5.5、GPT-5.4、Claude Opus 4.8、Claude Opus 4.7、gpt-image-2 等模型与图像能力。",
        "url": "https://jojocode.com/",
        "image": data_uri("image/png", JOJOCODE_IMAGE),
        "highlights": [
            "GPT-5.6 全系列",
            "Fable 5",
            "Sonnet 5",
            "GPT-5.5",
            "GPT-5.4",
            "Opus 4.8",
            "Opus 4.7",
            "gpt-image-2",
        ],
    }))
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
    fn missing_or_unusable_top_ad_falls_back_to_the_builtin_slot() {
        // 广告源没配顶位、或配了一条缺 url 的残缺条目时，都不该让概览这块空着：
        // 回落到内置的贵价赞助位。
        for payload in [
            json!({ "version": 1, "ads": [] }),
            json!({
                "version": 1,
                "ads": [],
                "top_ad": { "type": "sponsor", "title": "残缺", "description": "描述" },
            }),
        ] {
            let normalized = normalize_ad_payload(payload);
            let top = normalized.get("topAd").expect("应当回落到内置置顶位");
            assert_eq!(top["id"], json!("jojocode-top"));
        }
    }
}

#[cfg(test)]
mod expiry_tests {
    use super::*;

    #[test]
    fn ymd_converts_to_epoch_days() {
        assert_eq!(ymd_to_days("1970-01-01"), Some(0));
        assert_eq!(ymd_to_days("1970-01-02"), Some(1));
        assert_eq!(ymd_to_days("2000-03-01"), Some(11_017));
    }

    #[test]
    fn malformed_dates_are_treated_as_not_expired() {
        // 宁可多显示，也不能因为格式问题把付费位吞掉。
        for bad in ["", "not-a-date", "2027-13-01", "2027-01-99", "2027"] {
            assert!(!is_expired_at(bad), "{bad} 不应被判定为过期");
        }
    }

    #[test]
    fn builtin_top_ad_is_available_and_well_formed() {
        let top = builtin_top_ad().expect("内置置顶位在有效期内应当可用");
        assert_eq!(top["type"], json!("sponsor"));
        assert!(
            top["url"]
                .as_str()
                .is_some_and(|u| u.starts_with("https://"))
        );
        assert!(
            top["image"]
                .as_str()
                .is_some_and(|i| i.starts_with("data:image/")),
            "内置 logo 应当内联成 data URI，避免离线时裂图"
        );
    }
}
