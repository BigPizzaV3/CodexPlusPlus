//! WorkBuddy Manager as a native gateway provider; account management remains external.
use crate::gateway::{
    apply,
    store::{GatewaySettings, GatewayStore},
};
use crate::settings::RelayProtocol;
use anyhow::{Context, ensure};
use serde_json::Value;
use std::{sync::Mutex, time::Duration};

static APPLY_LOCK: Mutex<()> = Mutex::new(());
const MAX_BODY: usize = 1024 * 1024;

pub fn is_workbuddy_provider(source: &str) -> bool {
    matches!(source, "workbuddy" | "codex:managed-workbuddy") || workbuddy_realm(source).is_some()
}

pub fn workbuddy_realm(source: &str) -> Option<&str> {
    source
        .strip_prefix("workbuddy:")
        .filter(|realm| matches!(*realm, "cn" | "global"))
}

pub fn workbuddy_url(raw: &str) -> anyhow::Result<String> {
    let raw = if raw.trim().is_empty() {
        "http://127.0.0.1:7864/v1"
    } else {
        raw.trim()
    };
    let mut url = reqwest::Url::parse(raw).context("WorkBuddy 地址格式无效")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && matches!(url.path().trim_end_matches('/'), "" | "/v1"),
        "WorkBuddy 地址仅支持本机 http/https 根路径或 /v1，不能带凭据和查询参数"
    );
    url.set_path("/v1");
    Ok(url.into())
}

fn client(timeout: u64) -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(timeout))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

async fn models(base: &str, key: &str) -> anyhow::Result<Vec<String>> {
    let value = get_json(base, key, "models").await?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .context("模型目录缺少 data 列表")?;
    let mut result = Vec::new();
    for item in data {
        if let Some(id) = item.get("id").and_then(Value::as_str).map(str::trim)
            && !id.is_empty()
            && id.len() <= 200
            && !id.chars().any(char::is_control)
            && !result.iter().any(|known| known == id)
        {
            result.push(id.to_string());
        }
    }
    ensure!(!result.is_empty(), "服务未返回可用模型，未修改供应商");
    Ok(result)
}

async fn get_json(base: &str, key: &str, path: &str) -> anyhow::Result<Value> {
    ensure!(
        !key.trim().is_empty(),
        "请先保存服务的 API Key（模型调用密钥）"
    );
    let mut response = client(10)?
        .get(format!("{}/{path}", base.trim_end_matches('/')))
        .bearer_auth(key)
        .send()
        .await
        .context("无法读取本地服务，请检查地址和运行状态")?;
    ensure!(
        response.status().is_success(),
        "本地服务查询失败：HTTP {}",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= MAX_BODY,
            "本地服务响应超过 1 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).context("本地服务没有返回有效 JSON")
}

fn lane_model(model: &str, realm: &str) -> bool {
    match (realm, model.split_once(':')) {
        ("cn", None) => true,
        (realm, Some((prefix, name))) => prefix == realm && !name.is_empty(),
        _ => false,
    }
}

fn validate_lane_usage(usage: &Value, realm: &str) -> anyhow::Result<()> {
    ensure!(
        usage.get("object").and_then(Value::as_str) == Some("key.usage"),
        "本地服务不是支持用量查询的 WorkBuddy Manager"
    );
    ensure!(
        usage.pointer("/key/realm").and_then(Value::as_str) == Some(realm),
        "请选择此版本专用的 WorkBuddy 调用 Key，不能用另一版本或不限版本的 Key"
    );
    Ok(())
}

pub async fn validate_workbuddy_key(base: &str, key: &str, realm: &str) -> anyhow::Result<()> {
    let usage = get_json(&workbuddy_url(base)?, key.trim(), "usage").await?;
    validate_lane_usage(&usage, realm)
}

/// Verify the actual Key's realm before creating a native provider; a name alone
/// cannot isolate routing, balance or billing.
pub async fn create_workbuddy_lane(
    store: &GatewayStore,
    realm: &str,
    base_url: &str,
    key: &str,
    skip_agents: Option<&[String]>,
) -> anyhow::Result<(GatewaySettings, String)> {
    let (source, name) = match realm {
        "cn" => ("workbuddy:cn", "WorkBuddy 国内版"),
        "global" => ("workbuddy:global", "WorkBuddy 国际版"),
        _ => anyhow::bail!("未知的 WorkBuddy 版本"),
    };
    let base = workbuddy_url(base_url)?;
    let key = key.trim();
    validate_workbuddy_key(&base, key, realm).await?;
    let mut ids = models(&base, key).await?;
    ids.retain(|id| lane_model(id, realm));
    let model = ids.first().context("此版本没有可用模型")?.clone();
    let input = apply::ProviderInput {
        name: name.into(),
        base_url: base,
        protocol: RelayProtocol::Responses,
        keys: vec![key.into()],
        model,
        model_list: ids.join("\n"),
        ..Default::default()
    };
    let _guard = APPLY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    ensure!(
        store
            .load()?
            .providers
            .iter()
            .all(|provider| provider.source != source),
        "这个版本已经接入，请在供应商池中编辑已有条目"
    );
    apply::create_provider(store, &input, source, skip_agents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn model_connection_checks_key_realm_and_loads_catalog() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for (path, body) in [
                (
                    "/v1/usage",
                    r#"{"object":"key.usage","key":{"realm":"cn"}}"#,
                ),
                (
                    "/v1/models",
                    r#"{"data":[{"id":"cn:model"},{"id":"global:model"}]}"#,
                ),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut data = [0; 4096];
                let count = stream.read(&mut data).unwrap();
                let request = String::from_utf8_lossy(&data[..count]);
                assert!(request.starts_with(&format!("GET {path} ")));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer test-key")
                );
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        let base = format!("http://{address}/v1");
        validate_workbuddy_key(&base, "test-key", "cn")
            .await
            .unwrap();
        let mut catalog = models(&base, "test-key").await.unwrap();
        catalog.retain(|model| lane_model(model, "cn"));
        assert_eq!(catalog, ["cn:model"]);
        server.join().unwrap();
    }
    #[test]
    fn workbuddy_only_accepts_loopback_and_normalizes_ipv6() {
        assert_eq!(
            workbuddy_url("http://[::1]:7864/").unwrap(),
            "http://[::1]:7864/v1"
        );
        for bad in [
            "http://example.com/v1",
            "http://u:p@localhost",
            "http://localhost/v1?secret=x",
            "http://localhost/api",
            "file:///tmp",
        ] {
            assert!(workbuddy_url(bad).is_err());
        }
    }

    #[test]
    fn workbuddy_lane_rejects_unscoped_and_other_realm_keys_and_models() {
        let usage = |realm| json!({"object":"key.usage","key":{"realm":realm}});
        assert!(validate_lane_usage(&usage("cn"), "cn").is_ok());
        assert!(validate_lane_usage(&usage("global"), "global").is_ok());
        assert!(validate_lane_usage(&usage("global"), "cn").is_err());
        assert!(validate_lane_usage(&usage(""), "global").is_err());
        assert!(lane_model("cn:example", "cn"));
        assert!(lane_model("example", "cn"));
        assert!(!lane_model("example", "global"));
        assert!(!lane_model("global:example", "cn"));
    }
}
