use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::RelayProfile;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sub2ApiBillingInfo {
    pub endpoint: String,
    pub group_rate_multiplier: f64,
    pub user_rate_multiplier: Option<f64>,
    pub resolved_rate_multiplier: f64,
    pub peak_rate_enabled: bool,
    pub peak_rate_multiplier: Option<f64>,
    pub applied_peak_multiplier: Option<f64>,
    pub effective_rate_multiplier: f64,
    pub observed_at: String,
}

#[derive(Debug, Deserialize)]
struct Sub2ApiBillingResponse {
    object: String,
    schema_version: u8,
    billing_scope: String,
    group_rate_multiplier: f64,
    #[serde(default)]
    user_rate_multiplier: Option<f64>,
    resolved_rate_multiplier: f64,
    peak_rate_enabled: bool,
    #[serde(default)]
    peak_rate_multiplier: Option<f64>,
    #[serde(default)]
    applied_peak_multiplier: Option<f64>,
    effective_rate_multiplier: f64,
    observed_at: String,
}

pub async fn fetch_sub2api_billing_info(
    profile: &RelayProfile,
) -> anyhow::Result<Sub2ApiBillingInfo> {
    let base_url = if profile.upstream_base_url.trim().is_empty() {
        profile.base_url.trim()
    } else {
        profile.upstream_base_url.trim()
    };
    if base_url.is_empty() {
        anyhow::bail!("Base URL 不能为空");
    }
    let api_key = profile.api_key.trim();
    if api_key.is_empty() {
        anyhow::bail!("API Key 不能为空");
    }

    let endpoint = sub2api_billing_endpoint(base_url);
    let client = crate::http_client::proxied_client(&profile.user_agent)?;
    let response = client
        .get(&endpoint)
        .bearer_auth(api_key)
        .header("x-api-key", api_key)
        .send()
        .await
        .with_context(|| format!("请求 {endpoint} 失败"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "HTTP {}：{}",
            status.as_u16(),
            sub2api_error_message(&body).unwrap_or_else(|| "上游拒绝读取倍率".to_string())
        );
    }
    let parsed: Sub2ApiBillingResponse =
        serde_json::from_str(&body).context("倍率接口没有返回有效 JSON")?;
    validate_sub2api_billing_response(&parsed)?;
    Ok(Sub2ApiBillingInfo {
        endpoint,
        group_rate_multiplier: parsed.group_rate_multiplier,
        user_rate_multiplier: parsed.user_rate_multiplier,
        resolved_rate_multiplier: parsed.resolved_rate_multiplier,
        peak_rate_enabled: parsed.peak_rate_enabled,
        peak_rate_multiplier: parsed.peak_rate_multiplier,
        applied_peak_multiplier: parsed.applied_peak_multiplier,
        effective_rate_multiplier: parsed.effective_rate_multiplier,
        observed_at: parsed.observed_at,
    })
}

pub fn sub2api_billing_endpoint(base_url: &str) -> String {
    let cleaned = base_url.trim().trim_end_matches('/');
    if cleaned.is_empty() {
        return String::new();
    }
    if cleaned.ends_with("/sub2api/billing") {
        return cleaned.to_string();
    }
    if crate::protocol_proxy::has_version_suffix(cleaned) {
        return format!("{cleaned}/sub2api/billing");
    }
    format!("{cleaned}/v1/sub2api/billing")
}

fn validate_sub2api_billing_response(response: &Sub2ApiBillingResponse) -> anyhow::Result<()> {
    if response.object != "sub2api.key_billing"
        || response.schema_version != 1
        || response.billing_scope != "token"
    {
        anyhow::bail!("倍率接口返回结构不是 sub2api.key_billing");
    }
    for value in [
        response.group_rate_multiplier,
        response.resolved_rate_multiplier,
        response.effective_rate_multiplier,
    ] {
        if !value.is_finite() || value < 0.0 {
            anyhow::bail!("倍率接口返回了无效倍率");
        }
    }
    if let Some(value) = response.user_rate_multiplier {
        if !value.is_finite() || value < 0.0 {
            anyhow::bail!("倍率接口返回了无效用户倍率");
        }
    }
    if response.observed_at.trim().is_empty() {
        anyhow::bail!("倍率接口缺少观测时间");
    }
    Ok(())
}

fn sub2api_error_message(body: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(body).ok()?;
    for path in [&["error", "message"][..], &["message"][..], &["error"][..]] {
        let mut current = &payload;
        for key in path {
            current = current.get(*key)?;
        }
        if let Some(message) = current
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(message.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn builds_sub2api_billing_endpoint_from_common_base_urls() {
        assert_eq!(
            sub2api_billing_endpoint("https://example.com"),
            "https://example.com/v1/sub2api/billing"
        );
        assert_eq!(
            sub2api_billing_endpoint("https://example.com/v1"),
            "https://example.com/v1/sub2api/billing"
        );
        assert_eq!(
            sub2api_billing_endpoint("https://example.com/v1/sub2api/billing"),
            "https://example.com/v1/sub2api/billing"
        );
    }

    #[tokio::test]
    async fn fetch_sub2api_billing_info_reads_effective_multiplier() {
        let server = spawn_billing_server(
            200,
            r#"{
                "object":"sub2api.key_billing",
                "schema_version":1,
                "billing_scope":"token",
                "group_rate_multiplier":0.8,
                "user_rate_multiplier":0.6,
                "resolved_rate_multiplier":0.6,
                "peak_rate_enabled":true,
                "peak_rate_multiplier":1.5,
                "applied_peak_multiplier":1.5,
                "effective_rate_multiplier":0.9,
                "observed_at":"2026-07-30T10:00:00Z"
            }"#,
        );
        let profile = RelayProfile {
            base_url: server.base_url.clone(),
            api_key: "sk-test".to_string(),
            ..RelayProfile::default()
        };

        let info = fetch_sub2api_billing_info(&profile).await.unwrap();
        let request = server.finish();

        assert_eq!(request.path, "/v1/sub2api/billing");
        assert_eq!(request.authorization, "Bearer sk-test");
        assert_eq!(request.x_api_key, "sk-test");
        assert_eq!(info.group_rate_multiplier, 0.8);
        assert_eq!(info.user_rate_multiplier, Some(0.6));
        assert_eq!(info.effective_rate_multiplier, 0.9);
    }

    struct BillingServer {
        base_url: String,
        handle: thread::JoinHandle<BillingRequest>,
    }

    impl BillingServer {
        fn finish(self) -> BillingRequest {
            self.handle.join().unwrap()
        }
    }

    struct BillingRequest {
        path: String,
        authorization: String,
        x_api_key: String,
    }

    fn spawn_billing_server(status: u16, body: &'static str) -> BillingServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let base_url = format!("http://{address}");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or_default()
                .to_string();
            let authorization = header_value(&request, "authorization");
            let x_api_key = header_value(&request, "x-api-key");
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            BillingRequest {
                path,
                authorization,
                x_api_key,
            }
        });
        BillingServer { base_url, handle }
    }

    fn header_value(request: &str, name: &str) -> String {
        request
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                if key.eq_ignore_ascii_case(name) {
                    Some(value.trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    }
}
