//! Antigravity：用 Antigravity 的 Google 登录调用它的 Code Assist 端点
//! （`daily-cloudcode-pa.googleapis.com`），模型含 Gemini、Claude 等。
//!
//! - 登录：浏览器 Google 登录（Antigravity 的 OAuth 客户端，本机回调 `/oauth-callback`）；
//!   粘贴 refresh token 或凭据 JSON（Antigravity 的登录在系统钥匙串里，不做本机导入）。
//! - 项目：按 Antigravity 的流程 loadCodeAssist / onboardUser 自动分配，个人账号也可以用。
//! - 模型：fetchAvailableModels 列出这个账号能用的模型（去掉补全专用、停用的）。登录、本机适配入口
//!   拉不到时用默认列表（记诊断日志）；「从上游同步」拉不到就报错，不拿默认列表冒充上游结果。

use anyhow::bail;
use serde_json::Value;

use crate::gateway::subscriptions::google::{self, ANTIGRAVITY, GoogleCredentials, Setup};
use crate::gateway::subscriptions::{ChatStream, LoginDone, LoginMethod, LoginStart, Subscription};

pub struct Antigravity;

/// 拉不到模型列表时用的。
pub const DEFAULT_MODELS: &[&str] = &[
    "gemini-3.8-flash-high",
    "gemini-3.8-flash-medium",
    "gemini-3.8-flash-low",
    "gemini-3-flash",
    "gemini-3.5-flash-lite",
    "claude-sonnet-4-6",
    "claude-opus-4-6-thinking",
    "gpt-oss-120b-medium",
];

/// 列表里有、但不是给对话用的模型。
const HIDDEN_MODELS: &[&str] = &["gemini-2.5-flash-thinking", "gemini-2.5-pro"];

/// fetchAvailableModels 的答复 → 模型列表：去掉补全专用（`chat_*` / `tab_*`）、停用的、没有显示名的
/// 内部模型和画图模型；按 Antigravity 模型选择器的顺序排，其余按 id。
pub fn parse_available_models(value: &Value) -> Vec<String> {
    let Some(models) = value.get("models").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut rank: Vec<String> = Vec::new();
    for sort in value
        .get("agentModelSorts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for group in sort
            .get("groups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for id in group
                .get("modelIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(id) = id.as_str()
                    && !rank.iter().any(|known| known == id)
                {
                    rank.push(id.to_string());
                }
            }
        }
    }
    let mut out: Vec<String> = models
        .iter()
        .filter(|(id, info)| {
            !id.starts_with("chat_")
                && !id.starts_with("tab_")
                && !id.contains("-image")
                && !HIDDEN_MODELS.contains(&id.as_str())
                && !info
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                && info
                    .get("displayName")
                    .and_then(Value::as_str)
                    .is_some_and(|name| !name.is_empty())
        })
        .map(|(id, _)| id.clone())
        .collect();
    out.sort_by(|left, right| {
        let place = |id: &String| {
            rank.iter()
                .position(|known| known == id)
                .unwrap_or(usize::MAX)
        };
        place(left).cmp(&place(right)).then_with(|| left.cmp(right))
    });
    out
}

fn default_models() -> Vec<String> {
    DEFAULT_MODELS
        .iter()
        .map(|model| model.to_string())
        .collect()
}

/// 照实拉模型列表（`base` 是对话、模型列表走的地址，测试时指到本机）：凭据不对、没有项目、
/// 接口失败都报错，不用默认列表兜底。
async fn fetch_models(
    http: &reqwest::Client,
    base: &str,
    credentials: &Value,
) -> anyhow::Result<Vec<String>> {
    let mut credentials = GoogleCredentials::from_value(credentials)?;
    if credentials.project_id.is_empty() {
        match ANTIGRAVITY.setup(http, &credentials).await? {
            Setup::Ready { project, .. } => credentials.project_id = project,
            Setup::NeedsProject(message) => bail!(message),
        }
    }
    let reply = ANTIGRAVITY
        .call(
            http,
            base,
            "fetchAvailableModels",
            &credentials.access_token,
            &serde_json::json!({ "project": credentials.project_id }),
            &[],
        )
        .await?;
    Ok(parse_available_models(&reply))
}

#[async_trait::async_trait]
impl Subscription for Antigravity {
    fn id(&self) -> &'static str {
        "antigravity"
    }

    fn name(&self) -> &'static str {
        "Antigravity"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[LoginMethod::Browser, LoginMethod::Paste]
    }

    fn note(&self) -> &'static str {
        "用 Antigravity 的 Google 登录调用它的 Gemini、Claude 等模型，个人账号也可以用。注意：Google 可能停用它发现在 Antigravity 之外使用的账号，请用可以承受损失的账号。也可以粘贴 refresh token（1// 开头）或凭据 JSON。"
    }

    async fn begin_login(
        &self,
        _http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::Browser => ANTIGRAVITY.begin_browser("").await,
            LoginMethod::Paste => google::begin_pasted(&ANTIGRAVITY, input).await,
            LoginMethod::ImportLocal | LoginMethod::DeviceCode => {
                bail!("Antigravity 只支持浏览器登录或粘贴凭据")
            }
        }
    }

    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        ANTIGRAVITY.finish(http, start).await
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        ANTIGRAVITY.refresh(http, credentials).await
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        // 宽松版：拉不到时用默认列表（记诊断日志）。登录、本机适配入口的 `/models` 用。
        Ok(crate::gateway::subscriptions::models_or_fallback(
            self.id(),
            fetch_models(http, ANTIGRAVITY.base, credentials).await,
            default_models,
        ))
    }

    async fn list_models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        fetch_models(http, ANTIGRAVITY.base, credentials).await
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        ANTIGRAVITY.chat(http, credentials, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn available_models_are_filtered_and_ranked() {
        let value = json!({
            "models": {
                "gemini-3-flash": { "displayName": "Gemini 3 Flash" },
                "tab_flash_lite_preview": {},
                "chat_20706": {},
                "gemini-3.8-flash-high": { "displayName": "Gemini 3.8 Flash (High)" },
                "claude-sonnet-4-6": { "displayName": "Claude Sonnet 4.6 (Thinking)" },
                "gemini-3.8-flash-tiered": {},
                "gemini-3.1-flash-image": { "displayName": "Gemini 3.1 Flash Image" },
                "gemini-2.5-flash-thinking": { "displayName": "x" },
                "gemini-4-argon-disabled": { "displayName": "Argon", "disabled": true },
                "gpt-oss-120b-medium": { "displayName": "GPT-OSS 120B (Medium)" },
            },
            "agentModelSorts": [{ "groups": [{ "modelIds": ["gemini-3.8-flash-high", "claude-sonnet-4-6", "gpt-oss-120b-medium"] }] }],
        });
        assert_eq!(
            parse_available_models(&value),
            vec![
                "gemini-3.8-flash-high",
                "claude-sonnet-4-6",
                "gpt-oss-120b-medium",
                "gemini-3-flash"
            ]
        );
        assert!(parse_available_models(&json!({})).is_empty());
    }

    #[test]
    fn note_warns_about_account_risk() {
        assert!(Antigravity.note().contains("可以承受损失的账号"));
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    /// 已有项目、token 没过期的凭据：拉模型不用再查项目、也不用刷新。
    fn ready_credentials() -> Value {
        json!({
            "access_token": "ya29.test-3221",
            "refresh_token": "1//test-3221",
            "expires_at": u64::MAX,
            "project_id": "proj-3221",
        })
    }

    #[tokio::test]
    async fn fetch_models_reads_fetch_available_models() {
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1internal:fetchAvailableModels"))
            .and(header("authorization", "Bearer ya29.test-3221"))
            .and(body_json(json!({ "project": "proj-3221" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": {
                    "gemini-3.9-pro-high": { "displayName": "Gemini 3.9 Pro (High)" },
                    "claude-opus-5-5": { "displayName": "Claude Opus 5.5" },
                    "tab_flash_lite_preview": {},
                },
                "agentModelSorts": [{ "groups": [{ "modelIds": ["gemini-3.9-pro-high", "claude-opus-5-5"] }] }],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let models = fetch_models(&http(), &server.uri(), &ready_credentials())
            .await
            .unwrap();
        assert_eq!(models, vec!["gemini-3.9-pro-high", "claude-opus-5-5"]);
    }

    /// #3221：照实拉取失败要报错（「从上游同步」据此提示原因），不能悄悄换成默认列表。
    #[tokio::test]
    async fn failed_fetch_reports_the_upstream_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1internal:fetchAvailableModels"))
            .respond_with(
                ResponseTemplate::new(503)
                    .set_body_json(json!({ "error": { "message": "backend unavailable" } })),
            )
            .mount(&server)
            .await;
        let error = fetch_models(&http(), &server.uri(), &ready_credentials())
            .await
            .unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("503"), "{text}");
        assert!(text.contains("backend unavailable"), "{text}");
    }

    #[tokio::test]
    async fn fetch_models_rejects_credentials_without_token() {
        let error = fetch_models(&http(), "http://127.0.0.1:9", &json!({}))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("重新登录"), "{error:#}");
    }
}
