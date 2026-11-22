use codex_plus_core::diagnostic_redaction::redact_report_value;
use codex_plus_core::settings::{BackendSettings, RelayApiKey, RelayHeaderKeyValue, RelayProfile};
use serde_json::{Value, json};

const API_KEY: &str = "sk-live-abcdefghijklmnopqrstuvwxyz0123";
const ACCESS_TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.access-payload.signature-part";
const REFRESH_TOKEN: &str = "rt_refresh-0123456789abcdefghij";
const MCP_SECRET: &str = "ghp_0123456789abcdefghijklmnopqrstuv";
const HEADER_SECRET: &str = "Bearer header-0123456789abcdefghij";

/// #3129：贴进 issue 的诊断报告里不能有任何凭据，设置里各处的 Key 和登录 token 都要打码。
#[test]
fn settings_in_the_report_carry_no_credentials() {
    let auth_contents = json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": API_KEY,
        "tokens": {
            "id_token": ACCESS_TOKEN,
            "access_token": ACCESS_TOKEN,
            "refresh_token": REFRESH_TOKEN,
            "account_id": "acct-0123456789"
        },
        "last_refresh": "2026-11-17T03:00:00Z"
    })
    .to_string();
    let config_contents = format!(
        r#"model = "gpt-5.5"
model_provider = "custom"

[model_providers.custom]
base_url = "https://relay.example.com/v1"
env_key = "OPENAI_API_KEY"
experimental_bearer_token = "{API_KEY}"

[mcp_servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[mcp_servers.github.env]
GITHUB_PERSONAL_ACCESS_TOKEN = "{MCP_SECRET}"
"#
    );
    let profile = RelayProfile {
        id: "relay-a".to_string(),
        name: "中转 A".to_string(),
        upstream_base_url: "https://relay.example.com/v1".to_string(),
        api_keys: vec![RelayApiKey {
            id: "key-1".to_string(),
            name: "主 Key".to_string(),
            api_key: API_KEY.to_string(),
        }],
        active_api_key_id: "key-1".to_string(),
        auth_contents,
        config_contents,
        vlm_api_key: API_KEY.to_string(),
        custom_headers: vec![
            RelayHeaderKeyValue {
                key: "Authorization".to_string(),
                value: HEADER_SECRET.to_string(),
            },
            RelayHeaderKeyValue {
                key: "X-Title".to_string(),
                value: "Codex++".to_string(),
            },
        ],
        ..RelayProfile::default()
    };
    let settings = BackendSettings {
        relay_profiles: vec![profile],
        active_relay_id: "relay-a".to_string(),
        weixin_connect_token: REFRESH_TOKEN.to_string(),
        ..BackendSettings::default()
    };

    let report = redact_report_value(serde_json::to_value(&settings).unwrap());
    let text = serde_json::to_string_pretty(&report).unwrap();
    for secret in [API_KEY, ACCESS_TOKEN, REFRESH_TOKEN, MCP_SECRET, HEADER_SECRET] {
        assert!(!text.contains(secret), "报告里出现了凭据 {secret}：\n{text}");
    }

    // 排查问题要用的信息都还在。
    let profile = &report["relayProfiles"][0];
    assert_eq!(profile["name"], "中转 A");
    assert_eq!(profile["upstreamBaseUrl"], "https://relay.example.com/v1");
    assert_eq!(profile["activeApiKeyId"], "key-1");
    assert_eq!(profile["apiKeys"][0]["name"], "主 Key");
    assert_eq!(profile["authContents"]["auth_mode"], "chatgpt");
    assert_eq!(profile["authContents"]["last_refresh"], "2026-11-17T03:00:00Z");
    let config = &profile["configContents"];
    assert_eq!(config["model"], "gpt-5.5");
    assert_eq!(config["model_providers"]["custom"]["base_url"], "https://relay.example.com/v1");
    assert_eq!(config["model_providers"]["custom"]["env_key"], "OPENAI_API_KEY");
    assert_eq!(config["mcp_servers"]["github"]["command"], "npx");
    assert_eq!(profile["customHeaders"][0]["key"], "Authorization");
    assert_eq!(profile["customHeaders"][1]["value"], "Codex++");
    assert_eq!(report["activeRelayId"], "relay-a");
}

#[test]
fn key_names_decide_what_is_masked() {
    let report = redact_report_value(json!({
        "apiKey": API_KEY,
        "apiKeyEnv": "OPENAI_API_KEY",
        "activeApiKeyId": "key-1",
        "anthropicAuthHeader": "x-api-key",
        "anthropicMaxTokens": 8192,
        "officialMixApiKey": true,
        "emptyToken": "",
        "model": "gpt-5.5",
        "proxy": "http://user:hunter2-password@127.0.0.1:7890",
        "endpoint": "https://mcp.example.com/sse?key=abcdef123456&mode=fast#top",
        "note": "sk-live-loose-key-in-a-plain-field"
    }));
    assert_ne!(report["apiKey"], API_KEY);
    assert_eq!(report["apiKeyEnv"], "OPENAI_API_KEY");
    assert_eq!(report["activeApiKeyId"], "key-1");
    assert_eq!(report["anthropicAuthHeader"], "x-api-key");
    assert_eq!(report["anthropicMaxTokens"], 8192);
    assert_eq!(report["officialMixApiKey"], true);
    assert_eq!(report["emptyToken"], "");
    assert_eq!(report["model"], "gpt-5.5");
    assert_eq!(report["proxy"], "http://***@127.0.0.1:7890");
    assert_eq!(report["endpoint"], "https://mcp.example.com/sse?key=***&mode=fast#top");
    assert_ne!(report["note"], "sk-live-loose-key-in-a-plain-field");
}

/// 内嵌的文件正文解析不了时整段走兜底脱敏，不原样放行。
#[test]
fn unparseable_embedded_files_fall_back_to_text_redaction() {
    let broken = format!("this is not toml [[\nOPENAI_API_KEY = \"{API_KEY}\"\n");
    let report = redact_report_value(json!({ "configContents": broken }));
    let Value::String(text) = &report["configContents"] else {
        panic!("解析失败时仍应是文本：{report}");
    };
    assert!(!text.contains(API_KEY), "{text}");
    assert!(text.contains("this is not toml"), "{text}");
}
