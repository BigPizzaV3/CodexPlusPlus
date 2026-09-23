//! 供应商自定义上游请求头（issue #1685）。
//!
//! 三处发往上游的请求——供应商测试连接、模型列表拉取、协议代理转发——共用本模块的
//! `apply_*`，保证用户配置的 header 在三处行为一致。
//!
//! 两条硬约束：
//! 1. 传输层/逐跳头（`Host`、`Content-Length` 等）由 HTTP 客户端按实际报文决定，
//!    用户配置一律拒绝，避免造出不合法的请求；
//! 2. 显式配置的 `Authorization` 优先于供应商 API Key —— 存在自定义 `Authorization`
//!    时不再注入 bearer，优先级只此一种，不做隐式合并。

use reqwest::RequestBuilder;
use reqwest::header::{HeaderName, HeaderValue};

use crate::settings::{RelayHeaderKeyValue, RelayProfile};

/// 传输层 / 逐跳头：这些由 reqwest 与实际连接决定，用户配置没有意义且会破坏请求。
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "upgrade",
    "expect",
];

/// 值属于凭据的头：日志与导出必须打码。
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "x-auth-token",
    "cookie",
    "set-cookie",
];

/// 是否为协议层掌控、不允许用户覆盖的传输头。
pub fn is_forbidden(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    FORBIDDEN_HEADERS.contains(&lowered.as_str())
}

/// 值是否敏感（需要脱敏）。
pub fn is_sensitive(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    SENSITIVE_HEADERS.contains(&lowered.as_str())
        || lowered.contains("token")
        || lowered.contains("secret")
}

/// 用户是否显式配置了 `Authorization`（带非空值）。
pub fn has_authorization(headers: &[RelayHeaderKeyValue]) -> bool {
    headers.iter().any(|header| {
        header.key.trim().eq_ignore_ascii_case("authorization") && !header.value.trim().is_empty()
    })
}

/// 校验自定义头列表：空行（未填写的占位行）跳过，其余非法项给出明确错误。
///
/// 报错只回显头名称，不回显值——值可能是凭据。
pub fn validate(headers: &[RelayHeaderKeyValue]) -> anyhow::Result<()> {
    let mut seen: Vec<String> = Vec::new();
    for header in headers {
        let key = header.key.trim();
        if key.is_empty() {
            // 表单里新增但还没填的行，视为未配置。
            continue;
        }
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| anyhow::anyhow!("自定义请求头「{key}」不是合法的 HTTP 头名称"))?;
        if is_forbidden(name.as_str()) {
            anyhow::bail!(
                "自定义请求头「{key}」由协议层掌控，不允许覆盖（Host、Content-Length 等传输头）"
            );
        }
        let lowered = name.as_str().to_string();
        if seen.contains(&lowered) {
            anyhow::bail!("自定义请求头「{key}」重复配置");
        }
        seen.push(lowered);
        HeaderValue::from_str(&header.value)
            .map_err(|_| anyhow::anyhow!("自定义请求头「{key}」的值包含非法字符"))?;
    }
    Ok(())
}

/// 把自定义头追加到请求上；非法项直接跳过（调用方应先 `validate`）。
pub fn apply_headers(
    mut builder: RequestBuilder,
    headers: &[RelayHeaderKeyValue],
) -> RequestBuilder {
    for header in headers {
        let key = header.key.trim();
        if key.is_empty() || is_forbidden(key) {
            continue;
        }
        let Ok(name) = HeaderName::from_bytes(key.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(&header.value) else {
            continue;
        };
        builder = builder.header(name, value);
    }
    builder
}

/// 给上游请求加上认证与自定义头。
///
/// 优先级：显式配置的 `Authorization` > 供应商 API Key；`noAuth` 供应商不注入任何认证。
pub fn apply(builder: RequestBuilder, relay: &RelayProfile) -> RequestBuilder {
    let builder = if relay.uses_no_auth() || has_authorization(&relay.custom_headers) {
        builder
    } else {
        builder.bearer_auth(relay.api_key.trim())
    };
    apply_headers(builder, &relay.custom_headers)
}

/// 日志 / 导出用：敏感头的值替换成 `***`，其余原样。
pub fn redacted(headers: &[RelayHeaderKeyValue]) -> Vec<RelayHeaderKeyValue> {
    headers
        .iter()
        .map(|header| RelayHeaderKeyValue {
            key: header.key.clone(),
            value: if is_sensitive(&header.key) && !header.value.trim().is_empty() {
                "***".to_string()
            } else {
                header.value.clone()
            },
        })
        .collect()
}

/// 去掉未填写的空行，压缩首尾空白，供保存前规范化。
pub fn normalized(headers: &[RelayHeaderKeyValue]) -> Vec<RelayHeaderKeyValue> {
    headers
        .iter()
        .filter_map(|header| {
            let key = header.key.trim();
            let value = header.value.trim();
            if key.is_empty() && value.is_empty() {
                return None;
            }
            Some(RelayHeaderKeyValue {
                key: key.to_string(),
                value: value.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RelayMode;

    fn header(key: &str, value: &str) -> RelayHeaderKeyValue {
        RelayHeaderKeyValue {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn profile_with(headers: Vec<RelayHeaderKeyValue>) -> RelayProfile {
        RelayProfile {
            relay_mode: RelayMode::PureApi,
            api_key: "sk-test".to_string(),
            custom_headers: headers,
            ..RelayProfile::default()
        }
    }

    fn build(profile: &RelayProfile) -> reqwest::Request {
        apply(reqwest::Client::new().post("http://127.0.0.1:1/"), profile)
            .build()
            .unwrap()
    }

    /// 传输头由协议层掌控，不能由用户覆盖。
    #[test]
    fn rejects_transport_headers() {
        for name in ["Host", "content-length", "Transfer-Encoding", "Connection"] {
            let error = validate(&[header(name, "x")]).unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("不允许覆盖"), "{name} 应被拒绝：{message}");
        }
    }

    #[test]
    fn rejects_duplicate_and_invalid_names() {
        assert!(validate(&[header("X-A", "1"), header("x-a", "2")]).is_err());
        assert!(validate(&[header("bad name", "1")]).is_err());
    }

    #[test]
    fn skips_empty_rows_and_trims() {
        validate(&[header("", ""), header("X-Ok", "1")]).unwrap();
        let normalized = normalized(&[header("", ""), header(" X-Ok ", " 1 ")]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].key, "X-Ok");
        assert_eq!(normalized[0].value, "1");
    }

    /// 显式 Authorization 优先于 API Key：两者不能同时写入。
    #[test]
    fn custom_authorization_wins_over_api_key() {
        let profile = profile_with(vec![header("Authorization", "Bearer custom")]);
        assert!(has_authorization(&profile.custom_headers));

        let request = build(&profile);
        let values = request.headers().get_all("authorization");
        assert_eq!(
            values.iter().count(),
            1,
            "不应同时写入 API Key 与自定义 Authorization"
        );
        assert_eq!(values.iter().next().unwrap(), "Bearer custom");
    }

    /// 没有自定义 Authorization 时，API Key 仍按 bearer 注入，其余自定义头原样带上。
    #[test]
    fn applies_api_key_and_custom_headers_together() {
        let profile = profile_with(vec![header("X-Tenant", "acme")]);
        let request = build(&profile);
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer sk-test"
        );
        assert_eq!(request.headers().get("x-tenant").unwrap(), "acme");
    }

    /// 无认证供应商不注入 bearer，但自定义头照旧。
    #[test]
    fn no_auth_profile_keeps_custom_headers_without_bearer() {
        let mut profile = profile_with(vec![header("X-Tenant", "acme")]);
        profile.no_auth = true;
        let request = build(&profile);
        assert!(request.headers().get("authorization").is_none());
        assert_eq!(request.headers().get("x-tenant").unwrap(), "acme");
    }

    /// 日志 / 导出必须对敏感头脱敏，且不回显值。
    #[test]
    fn redacts_sensitive_values() {
        let redacted = redacted(&[
            header("Authorization", "Bearer secret"),
            header("X-Api-Key", "sk-secret"),
            header("X-Tenant", "acme"),
        ]);
        assert_eq!(redacted[0].value, "***");
        assert_eq!(redacted[1].value, "***");
        assert_eq!(redacted[2].value, "acme");
    }

    /// 校验失败时的报错也不能回显值。
    #[test]
    fn validation_error_does_not_echo_value() {
        let error = validate(&[header("X-Secret-Token", "sk-super-secret\n")]).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("X-Secret-Token"));
        assert!(!message.contains("sk-super-secret"));
    }

    /// apply_headers 会静默跳过非法 / 禁止项（作为第二道防线）。
    #[test]
    fn apply_headers_ignores_forbidden_entries() {
        let request = apply_headers(
            reqwest::Client::new().post("http://127.0.0.1:1/"),
            &[header("Host", "evil.example"), header("X-Ok", "1")],
        )
        .build()
        .unwrap();
        // reqwest 在发送阶段才生成 Host，这里要确保用户值没有被写进去。
        assert!(request.headers().get("host").is_none());
        assert_eq!(request.headers().get("x-ok").unwrap(), "1");
    }
}
