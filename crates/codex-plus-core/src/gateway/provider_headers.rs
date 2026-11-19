//! 供应商池的自定义请求头（#3075）。
//!
//! 存在供应商的 `relay.custom_headers`，测试连接、拉模型列表、实际转发三处都经
//! `relay_headers` 带上，规则与直连模式完全一致：传输头不许配；显式 `Authorization`
//! 优先于 API Key（不再注入 bearer，不做合并）；官方登录通道过滤掉用户的 `Authorization`。
//!
//! 和直连模式不同的是供应商池的 Key 原文不出后端（#2782），请求头同理：
//! - 敏感头（鉴权、Cookie、名字里带 token / secret / key / auth / password 的）给界面时只给打码后的样子，
//!   界面没改就回传 `keep`，后端按名字沿用已存的值；
//! - 地址换到别的站点时不许沿用敏感头的旧值，必须重新填写，免得把这个站点的凭据发到另一个站点；
//! - 订阅账号、官方登录这类凭据由 Codex++ 自己管的成员不开放编辑（它们自带的请求头不展示，也不被覆盖）。

use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::settings::{RelayHeaderKeyValue, RelayMode, RelayProfile};

/// 编辑表单提交的一条请求头。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHeaderInput {
    pub key: String,
    #[serde(default)]
    pub value: String,
    /// 已保存的敏感头没改：沿用后端存的值（界面上只看得到打码后的样子）。
    #[serde(default)]
    pub keep: bool,
}

/// 给界面的一条请求头。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHeaderView {
    pub key: String,
    /// 普通头的原值；敏感头为空串（原文不出后端）。
    pub value: String,
    /// 敏感头打码后的样子；普通头为 None。
    pub masked: Option<String>,
}

/// 值算凭据、不回显给界面的请求头。比日志打码（`relay_headers::is_sensitive`）更宽：
/// 中转站常用 `X-Relay-Key`、`X-Auth` 之类的自定义名字传凭据。
pub fn is_secret(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    crate::relay_headers::is_sensitive(&lowered)
        || ["key", "auth", "pass", "credential", "cookie", "session", "sign"]
            .iter()
            .any(|word| lowered.contains(word))
}

/// 这个成员的请求头能不能在供应商池里编辑：订阅账号、官方登录、聚合、
/// 订阅接口包出来的本地上游，请求头由 Codex++ 自己定，不开放。
pub fn editable(relay: &RelayProfile) -> bool {
    !crate::official_login::member_uses_official_login_transport(relay)
        && crate::gateway::accounts::relay_account_kind(relay).is_none()
        && relay.relay_mode != RelayMode::Aggregate
        && (relay.relay_mode != RelayMode::Official || relay.official_mix_api_key)
}

/// 界面上列出的请求头：敏感头打码；不开放编辑的成员返回空。
pub fn view(relay: &RelayProfile) -> Vec<ProviderHeaderView> {
    if !editable(relay) {
        return Vec::new();
    }
    relay
        .custom_headers
        .iter()
        .filter(|header| !header.key.trim().is_empty())
        .map(|header| {
            if is_secret(&header.key) && !header.value.trim().is_empty() {
                ProviderHeaderView {
                    key: header.key.clone(),
                    value: String::new(),
                    masked: Some(crate::gateway::apply::mask_api_key(&header.value)),
                }
            } else {
                ProviderHeaderView {
                    key: header.key.clone(),
                    value: header.value.clone(),
                    masked: None,
                }
            }
        })
        .collect()
}

/// 地址的站点（协议 + 主机 + 端口），比较「换没换站点」用。
fn origin(base_url: &str) -> Option<String> {
    reqwest::Url::parse(base_url.trim()).ok().map(|url| {
        format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str().unwrap_or_default().to_ascii_lowercase(),
            url.port_or_known_default().unwrap_or_default()
        )
    })
}

/// 把表单提交的请求头换成要保存 / 发送的列表。
///
/// `stored` 是已保存的头，`stored_base_url` 是它们原来发往的地址，`base_url` 是这次的地址。
/// `keep` 的行按名字（不分大小写）沿用 `stored` 里的值；敏感头的旧值只在站点没变时沿用。
/// 结果去掉空行、校验格式（与直连模式同一套 `relay_headers` 规则）。报错只回显头名称。
pub fn resolve(
    stored: &[RelayHeaderKeyValue],
    stored_base_url: &str,
    base_url: &str,
    input: &[ProviderHeaderInput],
) -> anyhow::Result<Vec<RelayHeaderKeyValue>> {
    let same_site = origin(stored_base_url).is_some() && origin(stored_base_url) == origin(base_url);
    let mut headers = Vec::new();
    for row in input {
        let key = row.key.trim();
        if !row.keep {
            headers.push(RelayHeaderKeyValue {
                key: key.to_string(),
                value: row.value.clone(),
            });
            continue;
        }
        let Some(saved) = stored
            .iter()
            .find(|header| header.key.trim().eq_ignore_ascii_case(key))
        else {
            bail!("请求头「{key}」没有已保存的值，请重新填写");
        };
        if is_secret(key) && !same_site {
            bail!("地址换到了别的站点，请重新填写请求头「{key}」的值（不把原站点的凭据发到新站点）");
        }
        headers.push(RelayHeaderKeyValue {
            key: key.to_string(),
            value: saved.value.clone(),
        });
    }
    let headers = crate::relay_headers::normalized(&headers);
    if headers.len() > MAX_HEADERS {
        bail!("自定义请求头最多 {MAX_HEADERS} 条");
    }
    crate::relay_headers::validate(&headers)?;
    Ok(headers)
}

/// 与直连模式保存时的上限一致。
const MAX_HEADERS: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    fn header(key: &str, value: &str) -> RelayHeaderKeyValue {
        RelayHeaderKeyValue {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn row(key: &str, value: &str, keep: bool) -> ProviderHeaderInput {
        ProviderHeaderInput {
            key: key.to_string(),
            value: value.to_string(),
            keep,
        }
    }

    #[test]
    fn secret_names_cover_custom_credential_headers() {
        for name in ["Authorization", "X-Api-Key", "X-Relay-Key", "x-auth", "Cookie", "X-Access-Token", "X-Client-Secret"] {
            assert!(is_secret(name), "{name}");
        }
        for name in ["X-Tenant", "HTTP-Referer", "X-Title", "originator"] {
            assert!(!is_secret(name), "{name}");
        }
    }

    #[test]
    fn view_masks_secrets_and_keeps_plain_values() {
        let relay = RelayProfile {
            relay_mode: RelayMode::PureApi,
            custom_headers: vec![
                header("X-Tenant", "acme"),
                header("X-Relay-Key", "relay-secret-0123456789abcdef"),
            ],
            ..RelayProfile::default()
        };
        let shown = view(&relay);
        assert_eq!(shown[0].value, "acme");
        assert_eq!(shown[0].masked, None);
        assert_eq!(shown[1].value, "");
        let masked = shown[1].masked.clone().unwrap();
        assert!(!masked.contains("0123456789"));
        let text = serde_json::to_string(&shown).unwrap();
        assert!(!text.contains("relay-secret-0123456789abcdef"));
        // 纯官方成员不开放：自带的头不列出。
        let official = RelayProfile {
            relay_mode: RelayMode::Official,
            ..relay.clone()
        };
        assert!(view(&official).is_empty());
    }

    #[test]
    fn keep_reuses_saved_value_only_on_the_same_site() {
        let stored = vec![header("X-Relay-Key", "secret"), header("X-Tenant", "acme")];
        let input = vec![row("x-relay-key", "", true), row("X-Tenant", "acme2", false)];
        let same = resolve(&stored, "https://relay.test/v1", "https://relay.test/v2", &input).unwrap();
        assert_eq!(same, vec![header("x-relay-key", "secret"), header("X-Tenant", "acme2")]);
        let error = resolve(&stored, "https://relay.test/v1", "https://other.test/v1", &input)
            .unwrap_err()
            .to_string();
        assert!(error.contains("X-Relay-Key") || error.contains("x-relay-key"));
        assert!(!error.contains("secret\""));
        // 非敏感头换站点照样沿用。
        let plain = resolve(&stored, "https://relay.test/v1", "https://other.test/v1", &[row("X-Tenant", "", true)]).unwrap();
        assert_eq!(plain, vec![header("X-Tenant", "acme")]);
    }

    #[test]
    fn rejects_transport_headers_and_unknown_keep_rows() {
        assert!(resolve(&[], "", "https://a.test", &[row("Host", "evil", false)]).is_err());
        assert!(resolve(&[], "", "https://a.test", &[row("X-New", "", true)]).is_err());
        // 空行去掉。
        assert!(resolve(&[], "", "https://a.test", &[row(" ", " ", false)]).unwrap().is_empty());
    }
}
