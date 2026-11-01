//! Gemini Code Assist：用 Gemini CLI 的 Google 登录调用 Code Assist（`cloudcode-pa.googleapis.com`）。
//!
//! - 登录：浏览器 Google 登录（Gemini CLI 的 OAuth 客户端，本机回调 `/oauth2callback`）；
//!   导入本机 Gemini CLI 的登录（`~/.gemini/oauth_creds.json`，项目读 `~/.gemini/.env` 的
//!   `GOOGLE_CLOUD_PROJECT`）；粘贴（项目 id → 带着它走浏览器登录；或 refresh token / 凭据 JSON）。
//! - 项目：Google 已不再为个人账号提供 Gemini CLI 的免费额度，Code Assist Standard / Enterprise
//!   需要一个 Google Cloud 项目。登录时按 Gemini CLI 的流程查（loadCodeAssist / onboardUser），
//!   查不到又没指定时提示用户用「粘贴凭据」填项目 id 重新登录。
//! - 模型：retrieveUserQuota 列出这个账号有额度的模型；拉不到时用默认列表。

use std::path::{Path, PathBuf};

use anyhow::bail;
use serde_json::{Value, json};

use crate::gateway::subscriptions::google::{
    self, GEMINI_CLI, GoogleCredentials, Setup, client_of_id_token, credentials_from_json,
};
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, oauth,
};

pub struct Gemini;

/// 拉不到模型列表时用的（Gemini CLI 提供的模型）。
pub const DEFAULT_MODELS: &[&str] = &[
    "gemini-3.1-pro-preview",
    "gemini-3-pro-preview",
    "gemini-3-flash-preview",
    "gemini-2.5-pro",
    "gemini-2.5-flash",
    "gemini-2.5-flash-lite",
];

fn gemini_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".gemini"))
}

/// dotenv 文件里某个键的值（忽略注释、`export ` 前缀和引号）。
fn env_file_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        let value = value.split(" #").next().unwrap_or(value).trim();
        let value = value.trim_matches(['"', '\'']).trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// 读 Gemini CLI 目录里的登录：凭据、项目（`.env`）、当前账号（`google_accounts.json`）。
pub fn import_from_dir(dir: &Path) -> anyhow::Result<GoogleCredentials> {
    let value = oauth::read_local_json(&dir.join("oauth_creds.json"))?;
    let mut credentials = credentials_from_json(&value)?;
    if credentials.refresh_token.is_empty() {
        bail!("Gemini CLI 的登录里没有 refresh token，请先运行 gemini 重新登录");
    }
    // Google 只给签发 token 的那个 OAuth 客户端刷新：别的客户端签发的用不了。
    if let Some(id_token) = value.get("id_token").and_then(Value::as_str) {
        let client = client_of_id_token(id_token);
        if !client.is_empty() && client != GEMINI_CLI.client_id {
            bail!("~/.gemini/oauth_creds.json 不是 Gemini CLI 自己的 Google 登录，无法导入");
        }
    }
    if credentials.project_id.is_empty() {
        let project = std::fs::read_to_string(dir.join(".env"))
            .ok()
            .and_then(|text| {
                env_file_value(&text, "GOOGLE_CLOUD_PROJECT")
                    .or_else(|| env_file_value(&text, "GOOGLE_CLOUD_PROJECT_ID"))
            })
            .or_else(|| {
                std::env::var("GOOGLE_CLOUD_PROJECT")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            });
        if let Some(project) = project {
            credentials.project_id = project.trim().to_string();
            credentials.project_pinned = true;
        }
    }
    if credentials.email.is_empty()
        && let Ok(accounts) = oauth::read_local_json(&dir.join("google_accounts.json"))
        && let Some(active) = accounts.get("active").and_then(Value::as_str)
    {
        credentials.email = active.to_string();
    }
    Ok(credentials)
}

/// retrieveUserQuota 的额度桶 → 模型列表（去掉 Vertex 的、去重，按默认列表的顺序排在前面）。
pub fn parse_quota_models(value: &Value) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    for bucket in value
        .get("buckets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = bucket.get("modelId").and_then(Value::as_str) else {
            continue;
        };
        if id.is_empty() || id.ends_with("_vertex") || models.iter().any(|model| model == id) {
            continue;
        }
        models.push(id.to_string());
    }
    models.sort_by_key(|model| {
        DEFAULT_MODELS
            .iter()
            .position(|known| known == model)
            .unwrap_or(DEFAULT_MODELS.len())
    });
    models
}

fn default_models() -> Vec<String> {
    DEFAULT_MODELS
        .iter()
        .map(|model| model.to_string())
        .collect()
}

#[async_trait::async_trait]
impl Subscription for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }

    fn name(&self) -> &'static str {
        "Gemini Code Assist"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[
            LoginMethod::Browser,
            LoginMethod::ImportLocal,
            LoginMethod::Paste,
        ]
    }

    fn note(&self) -> &'static str {
        "用 Gemini CLI 的 Google 登录调用 Gemini Code Assist。Google 已停止为个人账号提供服务，只有 Code Assist Standard / Enterprise 可用，且需要 Google Cloud 项目 id：选「粘贴凭据」填入项目 id（如 my-project-123）会带着它走浏览器登录；也可以粘贴 refresh token 或 oauth_creds.json 内容（可加 project=项目id）。导入本机 Gemini CLI 登录时项目读 ~/.gemini/.env 的 GOOGLE_CLOUD_PROJECT。"
    }

    fn detect_local(&self) -> Option<String> {
        import_from_dir(&gemini_dir()?).ok().map(|credentials| credentials.email)
    }

    async fn begin_login(
        &self,
        _http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::Browser => GEMINI_CLI.begin_browser("").await,
            LoginMethod::Paste => google::begin_pasted(&GEMINI_CLI, input).await,
            LoginMethod::ImportLocal => {
                let dir = gemini_dir().ok_or_else(|| anyhow::anyhow!("找不到用户目录"))?;
                let credentials = import_from_dir(&dir)?;
                Ok(LoginStart {
                    message: "正在导入 Gemini CLI 的登录…".to_string(),
                    state: json!({ "credentials": credentials.to_value() }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::DeviceCode => bail!("Gemini Code Assist 不支持设备码登录"),
        }
    }

    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        GEMINI_CLI.finish(http, start).await
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        GEMINI_CLI.refresh(http, credentials).await
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        let Ok(mut credentials) = GoogleCredentials::from_value(credentials) else {
            return Ok(default_models());
        };
        if credentials.project_id.is_empty() {
            match GEMINI_CLI.setup(http, &credentials).await {
                Ok(Setup::Ready { project, .. }) => credentials.project_id = project,
                _ => return Ok(default_models()),
            }
        }
        let reply = GEMINI_CLI
            .call(
                http,
                GEMINI_CLI.base,
                "retrieveUserQuota",
                &credentials.access_token,
                &json!({ "project": credentials.project_id }),
                &[],
            )
            .await;
        let models = reply
            .map(|value| parse_quota_models(&value))
            .unwrap_or_default();
        Ok(if models.is_empty() {
            default_models()
        } else {
            models
        })
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        GEMINI_CLI.chat(http, credentials, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_env_file_values() {
        let text = "# 注释\nexport GOOGLE_CLOUD_PROJECT=\"team-proj-1\"  # 行尾注释\nOTHER=x\n";
        assert_eq!(
            env_file_value(text, "GOOGLE_CLOUD_PROJECT").as_deref(),
            Some("team-proj-1")
        );
        assert_eq!(env_file_value(text, "MISSING"), None);
        assert_eq!(
            env_file_value("GOOGLE_CLOUD_PROJECT=", "GOOGLE_CLOUD_PROJECT"),
            None
        );
    }

    #[test]
    fn imports_gemini_cli_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("oauth_creds.json"),
            json!({
                "access_token": "ya29.a",
                "refresh_token": "1//r",
                "expiry_date": 1_900_000_000_000_u64,
                "token_type": "Bearer",
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".env"),
            "GOOGLE_CLOUD_PROJECT=my-team-proj\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("google_accounts.json"),
            json!({ "active": "me@corp.test", "old": [] }).to_string(),
        )
        .unwrap();
        let credentials = import_from_dir(dir.path()).unwrap();
        assert_eq!(credentials.refresh_token, "1//r");
        assert_eq!(credentials.expires_at, 1_900_000_000_000);
        assert_eq!(credentials.project_id, "my-team-proj");
        assert!(credentials.project_pinned);
        assert_eq!(credentials.email, "me@corp.test");
    }

    #[test]
    fn import_rejects_other_clients_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(import_from_dir(dir.path()).is_err());
        let payload = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            br#"{"azp":"someone-else.apps.googleusercontent.com"}"#,
        );
        std::fs::write(
            dir.path().join("oauth_creds.json"),
            json!({ "refresh_token": "1//r", "id_token": format!("e30.{payload}.s") }).to_string(),
        )
        .unwrap();
        assert!(
            import_from_dir(dir.path())
                .unwrap_err()
                .to_string()
                .contains("不是 Gemini CLI")
        );
    }

    #[test]
    fn quota_buckets_become_models() {
        let value = json!({ "buckets": [
            { "modelId": "gemini-2.5-flash", "tokenType": "REQUESTS" },
            { "modelId": "gemini-2.5-pro", "tokenType": "REQUESTS" },
            { "modelId": "gemini-2.5-pro", "tokenType": "INPUT_TOKENS" },
            { "modelId": "gemini-2.5-pro_vertex" },
            { "modelId": "gemini-3-flash" },
        ] });
        assert_eq!(
            parse_quota_models(&value),
            vec!["gemini-2.5-pro", "gemini-2.5-flash", "gemini-3-flash"]
        );
        assert!(parse_quota_models(&json!({})).is_empty());
    }
}
