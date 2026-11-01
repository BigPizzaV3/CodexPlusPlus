//! Google 登录与 Code Assist 接口（Gemini Code Assist、Antigravity 共用）。
//!
//! - 登录：Google OAuth（PKCE + 本机回调），用各家官方客户端的 OAuth 客户端 id；
//!   拿到 access / refresh token 后查邮箱，再按官方客户端的流程找到（或开通）Code Assist 的项目。
//! - 凭据：[`GoogleCredentials`]，存 access / refresh token、过期时间（毫秒）、项目 id、邮箱。
//! - 请求：`POST {base}/v1internal:<方法>`，带 Bearer token 和官方客户端的 User-Agent。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::gateway::subscriptions::code_assist::{self, Flavor};
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginStart, SubscriptionError, chat, fail, oauth,
};

const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo?alt=json";
pub const CODE_ASSIST_PROD: &str = "https://cloudcode-pa.googleapis.com";
pub const CODE_ASSIST_DAILY: &str = "https://daily-cloudcode-pa.googleapis.com";

/// access token 离过期不到这么久就刷新。
const REFRESH_AHEAD_MS: u64 = 5 * 60 * 1000;

/// 开通 Code Assist（onboardUser）最多等几轮。
const ONBOARD_ROUNDS: usize = 8;
const ONBOARD_INTERVAL: Duration = Duration::from_secs(3);

/// 借用登录的官方客户端。
pub struct GoogleApp {
    pub flavor: Flavor,
    /// 报错里用的名字。
    pub label: &'static str,
    /// 官方客户端的 OAuth 客户端（桌面应用类型，secret 随客户端分发，不是机密）。
    pub client_id: &'static str,
    pub client_secret: &'static str,
    pub scopes: &'static [&'static str],
    /// 回调地址的主机名与路径（和官方客户端一致）。
    pub callback_host: &'static str,
    pub callback_path: &'static str,
    /// 对话、模型列表走的地址。
    pub base: &'static str,
    /// 查项目（loadCodeAssist）走的地址。
    pub load_base: &'static str,
}

/// Gemini CLI 的 OAuth 客户端（与 Gemini CLI 源码里的一致）。
pub static GEMINI_CLI: GoogleApp = GoogleApp {
    flavor: Flavor::GeminiCli,
    label: "Gemini Code Assist",
    client_id: "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com",
    client_secret: "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",
    scopes: &[
        "https://www.googleapis.com/auth/cloud-platform",
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/userinfo.profile",
    ],
    callback_host: "127.0.0.1",
    callback_path: "/oauth2callback",
    base: CODE_ASSIST_PROD,
    load_base: CODE_ASSIST_PROD,
};

/// Antigravity 的 OAuth 客户端。
pub static ANTIGRAVITY: GoogleApp = GoogleApp {
    flavor: Flavor::Antigravity,
    label: "Antigravity",
    client_id: "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com",
    client_secret: "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",
    scopes: &[
        "https://www.googleapis.com/auth/cloud-platform",
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/userinfo.profile",
        "https://www.googleapis.com/auth/cclog",
        "https://www.googleapis.com/auth/experimentsandconfigs",
    ],
    callback_host: "localhost",
    callback_path: "/oauth-callback",
    base: CODE_ASSIST_DAILY,
    load_base: CODE_ASSIST_PROD,
};

/// 冒充的 Gemini CLI 版本。
const GEMINI_CLI_VERSION: &str = "0.61.0";
/// 冒充的 Antigravity 版本。
const ANTIGRAVITY_VERSION: &str = "2.9.1";

/// 存在 gateway.json 里的凭据。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleCredentials {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    /// access token 过期时间（毫秒时间戳）。
    #[serde(default)]
    pub expires_at: u64,
    /// 请求计费 / 归属的 Google Cloud 项目（Code Assist 分配的，或用户指定的）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project_id: String,
    /// 用户自己指定了项目：刷新时不再用 Code Assist 分配的覆盖。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub project_pinned: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// 套餐名（loadCodeAssist 给的 tier 名），仅展示用。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan: String,
}

impl GoogleCredentials {
    pub fn from_value(value: &Value) -> anyhow::Result<Self> {
        let credentials: Self =
            serde_json::from_value(value.clone()).context("Google 凭据格式不对")?;
        if credentials.refresh_token.is_empty() && credentials.access_token.is_empty() {
            return Err(fail(401, "凭据里没有 Google token，请重新登录"));
        }
        Ok(credentials)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// access token 没有或快过期了。
    pub fn expiring(&self) -> bool {
        self.access_token.is_empty()
            || oauth::expiring(&self.to_value(), "expires_at", REFRESH_AHEAD_MS)
    }
}

impl GoogleApp {
    /// 官方客户端发给 Code Assist 的 User-Agent。
    pub fn user_agent(&self, model: &str) -> String {
        match self.flavor {
            Flavor::GeminiCli => {
                let platform = match std::env::consts::OS {
                    "macos" => "darwin",
                    "windows" => "win32",
                    other => other,
                };
                let arch = match std::env::consts::ARCH {
                    "x86_64" => "x64",
                    "aarch64" => "arm64",
                    other => other,
                };
                let model = if model.is_empty() {
                    "gemini-2.5-pro"
                } else {
                    model
                };
                format!("GeminiCLI/{GEMINI_CLI_VERSION}/{model} ({platform}; {arch}; terminal)")
            }
            Flavor::Antigravity => {
                let os = match std::env::consts::OS {
                    "macos" => "darwin",
                    other => other,
                };
                let arch = match std::env::consts::ARCH {
                    "x86_64" => "amd64",
                    "aarch64" => "arm64",
                    other => other,
                };
                format!("antigravity/hub/{ANTIGRAVITY_VERSION} {os}/{arch}")
            }
        }
    }

    /// Google 登录页地址。
    pub fn authorize_url(&self, redirect_uri: &str, challenge: &str, state: &str) -> String {
        format!(
            "{AUTHORIZE_URL}?{}",
            oauth::query(&[
                ("client_id", self.client_id),
                ("redirect_uri", redirect_uri),
                ("response_type", "code"),
                ("scope", &self.scopes.join(" ")),
                ("access_type", "offline"),
                ("prompt", "consent"),
                ("state", state),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
            ])
        )
    }

    // ---- 登录 -------------------------------------------------------------

    /// 浏览器登录第一步：在本机监听回调，返回登录页地址。`project` 是用户指定的项目（可空）。
    pub async fn begin_browser(&self, project: &str) -> anyhow::Result<LoginStart> {
        let listener = oauth::bind_callback(0).await?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://{}:{port}{}", self.callback_host, self.callback_path);
        let (verifier, challenge) = oauth::pkce_pair();
        let state = oauth::random_state();
        let open_url = self.authorize_url(&redirect_uri, &challenge, &state);
        park_listener(&state, listener);
        let message = if project.is_empty() {
            "已在浏览器打开 Google 登录页，登录完成后回到这里。5 分钟内有效。".to_string()
        } else {
            format!(
                "已在浏览器打开 Google 登录页（项目 {project}），登录完成后回到这里。5 分钟内有效。"
            )
        };
        Ok(LoginStart {
            open_url: Some(open_url),
            user_code: None,
            message,
            state: json!({
                "state": state,
                "verifier": verifier,
                "redirect_uri": redirect_uri,
                "project": project,
            }),
        })
    }

    /// 完成登录：浏览器登录等回调换 token；粘贴 / 导入的凭据直接校验。最后查邮箱和项目。
    pub async fn finish(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        let mut credentials = match start.state.get("credentials") {
            Some(value) => GoogleCredentials::from_value(value)?,
            None => self.wait_browser(http, &start.state).await?,
        };
        if credentials.expiring() {
            if credentials.refresh_token.is_empty() {
                bail!("粘贴的 access token 已过期，又没有 refresh token，请重新登录");
            }
            self.refresh_access(http, &mut credentials).await?;
        }
        if credentials.email.is_empty() {
            credentials.email = self
                .email(http, &credentials.access_token)
                .await
                .unwrap_or_default();
        }
        match self.setup(http, &credentials).await {
            Ok(Setup::Ready { project, plan }) => {
                credentials.project_id = project;
                credentials.plan = plan;
            }
            Ok(Setup::NeedsProject(message)) => bail!("{message}"),
            // 网络问题等：先保存登录，发请求前再查项目。
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "gateway.subscription_google_setup_deferred",
                    json!({ "vendor": self.label, "error": format!("{error:#}") }),
                );
            }
        }
        let account = if credentials.email.is_empty() {
            "Google".to_string()
        } else {
            credentials.email.clone()
        };
        Ok(LoginDone {
            credentials: credentials.to_value(),
            account,
        })
    }

    async fn wait_browser(
        &self,
        http: &reqwest::Client,
        state: &Value,
    ) -> anyhow::Result<GoogleCredentials> {
        let field = |key: &str| {
            state
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let expected = field("state");
        let listener = take_listener(&expected)
            .ok_or_else(|| anyhow::anyhow!("这次登录已经失效，请重新开始登录"))?;
        let params = oauth::wait_callback(&listener, self.callback_path, &expected).await?;
        drop(listener);
        let code = params.get("code").cloned().unwrap_or_default();
        if code.is_empty() {
            bail!("Google 没有返回授权码");
        }
        let redirect_uri = field("redirect_uri");
        let verifier = field("verifier");
        let value = oauth::post_form(
            http,
            TOKEN_URL,
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", &redirect_uri),
                ("client_id", self.client_id),
                ("client_secret", self.client_secret),
                ("code_verifier", &verifier),
            ],
        )
        .await
        .context("Google 登录换取 token 失败")?;
        let mut credentials = parse_token_reply(&value)?;
        if credentials.refresh_token.is_empty() {
            bail!("Google 没有返回 refresh token，请重新登录并同意授权");
        }
        let project = field("project");
        if !project.is_empty() {
            credentials.project_id = project;
            credentials.project_pinned = true;
        }
        Ok(credentials)
    }

    /// 用 refresh token 换新的 access token。refresh token 失效时报 401（网关停用账号到重新登录）。
    pub async fn refresh_access(
        &self,
        http: &reqwest::Client,
        credentials: &mut GoogleCredentials,
    ) -> anyhow::Result<()> {
        if credentials.refresh_token.is_empty() {
            return Err(fail(
                401,
                format!("{} 的登录已过期，请重新登录", self.label),
            ));
        }
        let reply = oauth::post_form(
            http,
            TOKEN_URL,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &credentials.refresh_token),
                ("client_id", self.client_id),
                ("client_secret", self.client_secret),
            ],
        )
        .await;
        let value = match reply {
            Ok(value) => value,
            Err(error) => {
                let status = error
                    .downcast_ref::<SubscriptionError>()
                    .map(|error| error.status);
                // invalid_grant（撤销 / 改密码）是 400：按登录失效处理。
                return Err(match status {
                    Some(400 | 401 | 403) => fail(
                        401,
                        format!("{} 的登录已失效，请重新登录（{error:#}）", self.label),
                    ),
                    _ => error.context(format!("刷新 {} 的 token 失败", self.label)),
                });
            }
        };
        let fresh = parse_token_reply(&value)?;
        credentials.access_token = fresh.access_token;
        credentials.expires_at = fresh.expires_at;
        if !fresh.refresh_token.is_empty() {
            credentials.refresh_token = fresh.refresh_token;
        }
        Ok(())
    }

    /// 账号邮箱。
    pub async fn email(
        &self,
        http: &reqwest::Client,
        access_token: &str,
    ) -> anyhow::Result<String> {
        let response = http
            .get(USERINFO_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .context("无法连接 Google")?;
        let value = oauth::read_json(response).await?;
        Ok(value
            .get("email")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    // ---- 刷新 -------------------------------------------------------------

    /// 请求前：token 快过期就刷新；还没有项目就查一次。有变化时返回新凭据。
    pub async fn refresh(
        &self,
        http: &reqwest::Client,
        value: &Value,
    ) -> anyhow::Result<Option<Value>> {
        let mut credentials = GoogleCredentials::from_value(value)?;
        let mut changed = false;
        if credentials.expiring() {
            self.refresh_access(http, &mut credentials).await?;
            changed = true;
        }
        if credentials.project_id.is_empty() {
            match self.setup(http, &credentials).await? {
                Setup::Ready { project, plan } => {
                    credentials.project_id = project;
                    credentials.plan = plan;
                    changed = true;
                }
                // 没有项目用不了：403 让网关停用这个账号，直到重新登录时指定项目。
                Setup::NeedsProject(message) => return Err(fail(403, message)),
            }
        }
        Ok(changed.then(|| credentials.to_value()))
    }

    // ---- Code Assist ------------------------------------------------------

    /// `POST {base}/v1internal:<method>`。
    pub async fn call(
        &self,
        http: &reqwest::Client,
        base: &str,
        method: &str,
        access_token: &str,
        body: &Value,
        extra_headers: &[(&str, &str)],
    ) -> anyhow::Result<Value> {
        let user_agent = self.user_agent("");
        let authorization = format!("Bearer {access_token}");
        let mut headers = vec![
            ("authorization", authorization.as_str()),
            ("user-agent", user_agent.as_str()),
        ];
        headers.extend_from_slice(extra_headers);
        oauth::post_json(http, &format!("{base}/v1internal:{method}"), body, &headers)
            .await
            .map_err(|error| relabel(error, self.label, method))
    }

    /// 找到这个账号请求要用的项目（第一次用时按官方客户端的流程开通 Code Assist）。
    pub async fn setup(
        &self,
        http: &reqwest::Client,
        credentials: &GoogleCredentials,
    ) -> anyhow::Result<Setup> {
        match self.flavor {
            Flavor::GeminiCli => self.setup_gemini(http, credentials).await,
            Flavor::Antigravity => self.setup_antigravity(http, credentials).await,
        }
    }

    /// Gemini CLI 的流程（它的 setupUser）：loadCodeAssist；已开通就用它给的项目或用户指定的；
    /// 没开通就按默认 tier onboardUser。个人账号（free tier）Google 已不再服务，Standard / Enterprise
    /// 需要用户指定项目。
    async fn setup_gemini(
        &self,
        http: &reqwest::Client,
        credentials: &GoogleCredentials,
    ) -> anyhow::Result<Setup> {
        let wanted = credentials.project_id.as_str();
        let mut metadata = json!({
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI",
        });
        let mut body = json!({});
        if !wanted.is_empty() {
            metadata["duetProject"] = json!(wanted);
            body["cloudaicompanionProject"] = json!(wanted);
        }
        body["metadata"] = metadata.clone();
        let load = self
            .call(
                http,
                self.load_base,
                "loadCodeAssist",
                &credentials.access_token,
                &body,
                &[],
            )
            .await?;
        let load = LoadReply::parse(&load);
        if let Some(message) = load.validation_needed() {
            return Ok(Setup::NeedsProject(message));
        }
        if load.has_current_tier {
            let project = if !credentials.project_pinned && !load.project.is_empty() {
                load.project.clone()
            } else {
                wanted.to_string()
            };
            if project.is_empty() {
                return Ok(Setup::NeedsProject(needs_project_message(
                    &load.ineligible_reason(),
                )));
            }
            return Ok(Setup::Ready {
                project,
                plan: load.plan.clone(),
            });
        }
        let tier = load.default_tier();
        if tier.user_defined_project && wanted.is_empty() {
            return Ok(Setup::NeedsProject(needs_project_message(
                &load.ineligible_reason(),
            )));
        }
        let mut request = json!({ "tierId": tier.id, "metadata": metadata });
        if tier.id != "free-tier" && !wanted.is_empty() {
            request["cloudaicompanionProject"] = json!(wanted);
        }
        let project = self
            .onboard(http, &credentials.access_token, &request, &[])
            .await?;
        let project = if project.is_empty() {
            wanted.to_string()
        } else {
            project
        };
        if project.is_empty() {
            return Ok(Setup::NeedsProject(needs_project_message(
                &load.ineligible_reason(),
            )));
        }
        Ok(Setup::Ready {
            project,
            plan: tier.name,
        })
    }

    /// Antigravity 的流程：loadCodeAssist（ideType ANTIGRAVITY）给项目就用；没有就 onboardUser。
    async fn setup_antigravity(
        &self,
        http: &reqwest::Client,
        credentials: &GoogleCredentials,
    ) -> anyhow::Result<Setup> {
        let body = json!({ "metadata": { "ideType": "ANTIGRAVITY" } });
        let load = self
            .call(
                http,
                self.load_base,
                "loadCodeAssist",
                &credentials.access_token,
                &body,
                &[],
            )
            .await;
        if credentials.project_pinned && !credentials.project_id.is_empty() {
            let plan = load
                .ok()
                .map(|value| LoadReply::parse(&value).plan)
                .unwrap_or_default();
            return Ok(Setup::Ready {
                project: credentials.project_id.clone(),
                plan,
            });
        }
        let load = LoadReply::parse(&load?);
        if !load.project.is_empty() {
            return Ok(Setup::Ready {
                project: load.project.clone(),
                plan: load.plan.clone(),
            });
        }
        let mut tier = load.default_tier();
        if tier.id.is_empty() || tier.id == "legacy-tier" {
            tier.id = "free-tier".to_string();
        }
        let request = json!({
            "tier_id": tier.id,
            "metadata": {
                "ide_type": "ANTIGRAVITY",
                "ide_version": ANTIGRAVITY_VERSION,
                "ide_name": "antigravity",
            },
        });
        let user_agent = format!("{} google-api-nodejs-client/10.3.0", self.user_agent(""));
        let headers = [
            ("user-agent", user_agent.as_str()),
            ("x-goog-api-client", "gl-node/22.21.1"),
        ];
        let project = self
            .onboard(http, &credentials.access_token, &request, &headers)
            .await?;
        if project.is_empty() {
            let reason = load.ineligible_reason();
            return Ok(Setup::NeedsProject(if reason.is_empty() {
                "Antigravity 没有给这个 Google 账号分配项目：请先用这个账号登录一次 Antigravity 应用再试。"
                    .to_string()
            } else {
                format!("Antigravity 不为这个 Google 账号提供服务：{reason}")
            }));
        }
        Ok(Setup::Ready {
            project,
            plan: tier.name,
        })
    }

    /// onboardUser：长任务，没完成时轮询（有操作名就 GET 操作，没有就再发一次）。
    async fn onboard(
        &self,
        http: &reqwest::Client,
        access_token: &str,
        request: &Value,
        headers: &[(&str, &str)],
    ) -> anyhow::Result<String> {
        let base = match self.flavor {
            Flavor::GeminiCli => self.load_base,
            Flavor::Antigravity => self.base,
        };
        let mut reply = self
            .call(http, base, "onboardUser", access_token, request, headers)
            .await?;
        for _ in 0..ONBOARD_ROUNDS {
            if reply.get("done").and_then(Value::as_bool).unwrap_or(false) {
                return Ok(project_id_of(
                    reply.pointer("/response/cloudaicompanionProject"),
                ));
            }
            tokio::time::sleep(ONBOARD_INTERVAL).await;
            let name = reply
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            reply = if name.is_empty() {
                self.call(http, base, "onboardUser", access_token, request, headers)
                    .await?
            } else {
                let response = http
                    .get(format!("{base}/v1internal/{name}"))
                    .bearer_auth(access_token)
                    .header("user-agent", self.user_agent(""))
                    .send()
                    .await
                    .with_context(|| format!("无法连接 {base}"))?;
                oauth::read_json(response)
                    .await
                    .map_err(|error| relabel(error, self.label, "getOperation"))?
            };
        }
        bail!("{} 开通 Code Assist 时间过长，请过一分钟再试", self.label)
    }

    /// 发对话：信封包好后请求 `streamGenerateContent?alt=sse`，返回 Chat SSE。
    pub async fn chat(
        &self,
        http: &reqwest::Client,
        value: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        let credentials = GoogleCredentials::from_value(value)?;
        if credentials.project_id.is_empty() {
            return Err(fail(
                403,
                format!("{} 还没有可用的 Google Cloud 项目，请重新登录", self.label),
            ));
        }
        let model = request
            .get("model")
            .and_then(Value::as_str)
            .filter(|model| !model.is_empty())
            .ok_or_else(|| fail(400, "请求里没有模型名"))?
            .to_string();
        let inner = code_assist::build_request(&request, self.flavor, &model);
        let body = code_assist::envelope(self.flavor, &model, &credentials.project_id, inner);
        let response = http
            .post(format!(
                "{}/v1internal:streamGenerateContent?alt=sse",
                self.base
            ))
            .bearer_auth(&credentials.access_token)
            .header("user-agent", self.user_agent(&model))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .with_context(|| format!("无法连接 {}", self.label))?;
        if !response.status().is_success() {
            return Err(chat::upstream_failure(response, self.label).await);
        }
        Ok(code_assist::chat_stream(response, &model))
    }
}

/// 查项目的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    Ready {
        project: String,
        plan: String,
    },
    /// 这个账号需要用户指定项目（或者 Google 不为它服务）：给用户看的说明。
    NeedsProject(String),
}

fn needs_project_message(reason: &str) -> String {
    let mut message = "这个 Google 账号需要指定 Google Cloud 项目 id 才能使用 Gemini Code Assist\
（个人账号 Google 已停止服务，只有 Code Assist Standard / Enterprise 可用）。\
请重新登录，登录方式选「粘贴凭据」，填入项目 id（如 my-project-123）后走浏览器登录；\
导入 Gemini CLI 登录时也可以在 ~/.gemini/.env 里写 GOOGLE_CLOUD_PROJECT。"
        .to_string();
    if !reason.is_empty() {
        message = format!("{reason}。{message}");
    }
    message
}

/// 给 Code Assist 的错误加上是哪一步。
fn relabel(error: anyhow::Error, label: &str, method: &str) -> anyhow::Error {
    match error.downcast::<SubscriptionError>() {
        Ok(error) => fail(error.status, format!("{label} {method} {}", error.message)),
        Err(error) => error.context(format!("{label} {method} 失败")),
    }
}

/// Code Assist 的项目有时是字符串，有时是 `{id, name}`。
fn project_id_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(id)) => id.clone(),
        Some(Value::Object(object)) => object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Default)]
struct Tier {
    id: String,
    name: String,
    user_defined_project: bool,
}

impl Tier {
    fn parse(value: &Value) -> Self {
        Self {
            id: value
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            user_defined_project: value
                .get("userDefinedCloudaicompanionProject")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
}

/// loadCodeAssist 的答复里用得到的部分。
#[derive(Debug, Default)]
struct LoadReply {
    has_current_tier: bool,
    project: String,
    plan: String,
    allowed: Vec<(Tier, bool)>,
    ineligible: Vec<Value>,
}

impl LoadReply {
    fn parse(value: &Value) -> Self {
        let current = value.get("currentTier").filter(|tier| tier.is_object());
        let paid = value.get("paidTier").filter(|tier| tier.is_object());
        let name_of = |tier: Option<&Value>| {
            tier.and_then(|tier| tier.get("name").or_else(|| tier.get("id")))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let plan = Some(name_of(paid))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| name_of(current));
        Self {
            has_current_tier: current.is_some(),
            project: project_id_of(value.get("cloudaicompanionProject")),
            plan,
            allowed: value
                .get("allowedTiers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|tier| {
                    (
                        Tier::parse(tier),
                        tier.get("isDefault")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    )
                })
                .collect(),
            ineligible: value
                .get("ineligibleTiers")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn default_tier(&self) -> Tier {
        self.allowed
            .iter()
            .find(|(_, default)| *default)
            .or_else(|| self.allowed.first())
            .map(|(tier, _)| tier.clone())
            .unwrap_or(Tier {
                id: "legacy-tier".to_string(),
                name: String::new(),
                user_defined_project: true,
            })
    }

    fn ineligible_reason(&self) -> String {
        self.ineligible
            .iter()
            .filter_map(|tier| tier.get("reasonMessage").and_then(Value::as_str))
            .filter(|reason| !reason.is_empty())
            .collect::<Vec<_>>()
            .join("；")
    }

    /// Google 要求先验证账号（给了验证链接）。
    fn validation_needed(&self) -> Option<String> {
        if self.has_current_tier {
            return None;
        }
        self.ineligible.iter().find_map(|tier| {
            let url = tier.get("validationUrl").and_then(Value::as_str)?;
            if tier.get("reasonCode").and_then(Value::as_str) != Some("VALIDATION_REQUIRED") {
                return None;
            }
            let reason = tier
                .get("reasonMessage")
                .and_then(Value::as_str)
                .unwrap_or("Google 要求验证账号");
            Some(format!(
                "{reason}：请先在浏览器打开 {url} 完成验证，再重新登录。"
            ))
        })
    }
}

/// token 接口的答复 → 凭据（过期时间换成毫秒时间戳）。
fn parse_token_reply(value: &Value) -> anyhow::Result<GoogleCredentials> {
    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if access_token.is_empty() {
        bail!("Google 没有返回 access token");
    }
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    let mut credentials = GoogleCredentials {
        access_token,
        refresh_token: value
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        expires_at: oauth::now_ms() + expires_in * 1000,
        ..GoogleCredentials::default()
    };
    if let Some(id_token) = value.get("id_token").and_then(Value::as_str) {
        credentials.email = email_from_id_token(id_token);
    }
    Ok(credentials)
}

fn email_from_id_token(id_token: &str) -> String {
    oauth::jwt_claims(id_token)
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// ID token 是发给哪个 OAuth 客户端的（`azp`，没有时看 `aud`）。
pub fn client_of_id_token(id_token: &str) -> String {
    let claims = oauth::jwt_claims(id_token);
    claims
        .get("azp")
        .or_else(|| claims.get("aud"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Google Cloud 项目 id 的样子：小写字母开头，字母数字和 `-`，6–30 位；也认旧的 `域名:项目`。
pub fn looks_like_project_id(text: &str) -> bool {
    let id = text.rsplit(':').next().unwrap_or(text);
    let length_ok = (6..=30).contains(&id.len());
    length_ok
        && id.starts_with(|ch: char| ch.is_ascii_lowercase())
        && !id.ends_with('-')
        && id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

/// 读官方客户端格式（Gemini CLI 的 `oauth_creds.json`）或本程序自己格式的凭据 JSON。
/// `expiry_date` / `expires_at` 都是毫秒；项目 id 认 `project_id` / `projectId` / `project` /
/// `GOOGLE_CLOUD_PROJECT`。
pub fn credentials_from_json(value: &Value) -> anyhow::Result<GoogleCredentials> {
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| {
                value
                    .get(*key)
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
            })
            .unwrap_or_default()
            .to_string()
    };
    let refresh_token = text(&["refresh_token", "refreshToken"]);
    let access_token = text(&["access_token", "accessToken"]);
    if refresh_token.is_empty() && access_token.is_empty() {
        bail!("没有找到 refresh_token / access_token");
    }
    let expires_at = ["expiry_date", "expires_at", "expiresAt", "expiry"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
        // 秒级时间戳换成毫秒。
        .map(|time| {
            if time < 100_000_000_000 {
                time * 1000
            } else {
                time
            }
        })
        .unwrap_or(0);
    let id_token = text(&["id_token"]);
    let mut email = text(&["email", "account"]);
    if email.is_empty() && !id_token.is_empty() {
        email = email_from_id_token(&id_token);
    }
    let project_id = text(&["project_id", "projectId", "project", "GOOGLE_CLOUD_PROJECT"]);
    Ok(GoogleCredentials {
        access_token,
        refresh_token,
        expires_at,
        project_pinned: !project_id.is_empty(),
        project_id,
        email,
        plan: String::new(),
    })
}

/// 粘贴的内容：凭据 JSON、refresh token（`1//` 开头）、access token（`ya29.` 开头）、项目 id
/// （可写成 `project=xxx`），用空白、逗号或分号分隔，可以组合。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pasted {
    pub credentials: Option<GoogleCredentials>,
    pub project: String,
}

pub fn parse_pasted(input: &str) -> anyhow::Result<Pasted> {
    let input = input.trim();
    if input.is_empty() {
        bail!("请粘贴凭据或项目 id");
    }
    if input.starts_with('{') {
        let value: Value = serde_json::from_str(input).context("粘贴的内容不是合法的 JSON")?;
        let credentials = credentials_from_json(&value)?;
        return Ok(Pasted {
            project: credentials.project_id.clone(),
            credentials: Some(credentials),
        });
    }
    let mut pasted = Pasted::default();
    let mut credentials = GoogleCredentials::default();
    for token in input.split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';') {
        let token = token.trim().trim_matches(['"', '\'']);
        if token.is_empty() {
            continue;
        }
        let (key, raw) = match token.split_once('=') {
            Some((key, value)) => (key.trim().to_ascii_lowercase(), value.trim()),
            None => (String::new(), token),
        };
        if raw.starts_with("1//") || key.contains("refresh") {
            credentials.refresh_token = raw.to_string();
        } else if raw.starts_with("ya29.") || key.contains("access") {
            credentials.access_token = raw.to_string();
        } else if key.is_empty() || key.contains("project") {
            if raw.chars().all(|ch| ch.is_ascii_digit()) {
                bail!("{raw} 是项目编号，请填项目 id（如 my-project-123），不是数字编号");
            }
            if !looks_like_project_id(raw) {
                bail!(
                    "看不懂粘贴的内容「{raw}」：请粘贴 Google Cloud 项目 id、refresh token（1// 开头）或凭据 JSON"
                );
            }
            pasted.project = raw.to_string();
        } else {
            bail!("看不懂粘贴的内容「{token}」");
        }
    }
    if !credentials.refresh_token.is_empty() || !credentials.access_token.is_empty() {
        if !pasted.project.is_empty() {
            credentials.project_id = pasted.project.clone();
            credentials.project_pinned = true;
        }
        pasted.credentials = Some(credentials);
    }
    Ok(pasted)
}

/// 粘贴登录的第一步：有凭据就直接交给 `finish` 校验；只有项目 id 时带着它走浏览器登录。
pub async fn begin_pasted(app: &GoogleApp, input: &str) -> anyhow::Result<LoginStart> {
    let pasted = parse_pasted(input)?;
    match pasted.credentials {
        Some(credentials) => Ok(LoginStart {
            message: "正在校验凭据…".to_string(),
            state: json!({ "credentials": credentials.to_value() }),
            ..LoginStart::default()
        }),
        None => app.begin_browser(&pasted.project).await,
    }
}

// ---- 等待中的回调监听 -------------------------------------------------------
//
// 监听要在打开浏览器前建好（begin_login），等回调在 finish_login 里；两步之间登录状态要经过界面
// 序列化，监听放不进去，所以按 state 暂存在进程里。

type Parked = HashMap<String, (Instant, tokio::net::TcpListener)>;

fn parked() -> &'static Mutex<Parked> {
    static PARKED: OnceLock<Mutex<Parked>> = OnceLock::new();
    PARKED.get_or_init(Default::default)
}

fn park_listener(state: &str, listener: tokio::net::TcpListener) {
    let mut parked = parked()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // 放弃的登录留下的监听：超过登录时限就关掉。
    parked.retain(|_, (since, _)| since.elapsed() < oauth::LOGIN_TIMEOUT);
    parked.insert(state.to_string(), (Instant::now(), listener));
}

fn take_listener(state: &str) -> Option<tokio::net::TcpListener> {
    parked()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(state)
        .map(|(_, listener)| listener)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_has_client_scopes_and_pkce() {
        let url = GEMINI_CLI.authorize_url("http://127.0.0.1:1234/oauth2callback", "CH", "ST");
        assert!(url.starts_with(AUTHORIZE_URL));
        assert!(url.contains("client_id=681255809395-"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1234%2Foauth2callback"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("code_challenge=CH"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("cloud-platform"));
        let url = ANTIGRAVITY.authorize_url("http://localhost:1/oauth-callback", "CH", "ST");
        assert!(url.contains("experimentsandconfigs"));
    }

    #[test]
    fn user_agents_follow_official_clients() {
        assert!(
            GEMINI_CLI
                .user_agent("gemini-2.5-flash")
                .starts_with("GeminiCLI/")
        );
        assert!(
            GEMINI_CLI
                .user_agent("gemini-2.5-flash")
                .contains("/gemini-2.5-flash (")
        );
        assert!(ANTIGRAVITY.user_agent("").starts_with("antigravity/hub/"));
    }

    #[test]
    fn imports_gemini_cli_oauth_creds() {
        // id_token 载荷：{"email":"a@b.com","azp":"681255809395-x"}
        let payload = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            br#"{"email":"a@b.com","azp":"681255809395-x"}"#,
        );
        let id_token = format!("e30.{payload}.sig");
        let value = json!({
            "access_token": "ya29.abc",
            "refresh_token": "1//refresh",
            "scope": "https://www.googleapis.com/auth/cloud-platform",
            "token_type": "Bearer",
            "id_token": id_token,
            "expiry_date": 1_900_000_000_000_u64,
        });
        let credentials = credentials_from_json(&value).unwrap();
        assert_eq!(credentials.access_token, "ya29.abc");
        assert_eq!(credentials.refresh_token, "1//refresh");
        assert_eq!(credentials.expires_at, 1_900_000_000_000);
        assert_eq!(credentials.email, "a@b.com");
        assert!(credentials.project_id.is_empty());
        assert_eq!(client_of_id_token(&id_token), "681255809395-x");
        // 秒级过期时间换成毫秒；项目 id 一起带上。
        let credentials =
            credentials_from_json(&json!({ "refresh_token": "1//r", "expires_at": 1_900_000_000, "projectId": "my-proj-1" }))
                .unwrap();
        assert_eq!(credentials.expires_at, 1_900_000_000_000);
        assert_eq!(credentials.project_id, "my-proj-1");
        assert!(credentials.project_pinned);
        assert!(credentials_from_json(&json!({ "token_type": "Bearer" })).is_err());
    }

    #[test]
    fn credentials_round_trip_and_expiry() {
        let credentials = GoogleCredentials {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: oauth::now_ms() + 3_600_000,
            project_id: "p-123456".into(),
            ..GoogleCredentials::default()
        };
        assert!(!credentials.expiring());
        let back = GoogleCredentials::from_value(&credentials.to_value()).unwrap();
        assert_eq!(back, credentials);
        let soon = GoogleCredentials {
            expires_at: oauth::now_ms() + 60_000,
            ..credentials.clone()
        };
        assert!(soon.expiring());
        let empty = GoogleCredentials {
            access_token: String::new(),
            ..credentials
        };
        assert!(empty.expiring());
        assert!(GoogleCredentials::from_value(&json!({})).is_err());
    }

    #[test]
    fn parses_pasted_inputs() {
        let pasted = parse_pasted("my-project-123").unwrap();
        assert_eq!(pasted.project, "my-project-123");
        assert!(pasted.credentials.is_none());
        let pasted = parse_pasted("project=acme.com:team-proj\n1//0refresh").unwrap();
        let credentials = pasted.credentials.unwrap();
        assert_eq!(credentials.refresh_token, "1//0refresh");
        assert_eq!(credentials.project_id, "acme.com:team-proj");
        assert!(credentials.project_pinned);
        let pasted = parse_pasted(r#"{"refresh_token":"1//x","project_id":"proj-abc"}"#).unwrap();
        assert_eq!(pasted.project, "proj-abc");
        assert!(
            parse_pasted("123456789")
                .unwrap_err()
                .to_string()
                .contains("项目编号")
        );
        assert!(parse_pasted("Hello World").is_err());
        assert!(parse_pasted("  ").is_err());
    }

    #[test]
    fn load_reply_picks_project_tier_and_reasons() {
        let reply = LoadReply::parse(&json!({
            "currentTier": { "id": "standard-tier", "name": "Gemini Code Assist Standard" },
            "cloudaicompanionProject": { "id": "proj-x", "name": "x" },
        }));
        assert!(reply.has_current_tier);
        assert_eq!(reply.project, "proj-x");
        assert_eq!(reply.plan, "Gemini Code Assist Standard");
        let reply = LoadReply::parse(&json!({
            "allowedTiers": [
                { "id": "free-tier", "userDefinedCloudaicompanionProject": false },
                { "id": "standard-tier", "isDefault": true, "userDefinedCloudaicompanionProject": true },
            ],
            "ineligibleTiers": [{ "reasonCode": "INELIGIBLE_ACCOUNT", "reasonMessage": "不再提供个人版" }],
        }));
        let tier = reply.default_tier();
        assert_eq!(tier.id, "standard-tier");
        assert!(tier.user_defined_project);
        assert_eq!(reply.ineligible_reason(), "不再提供个人版");
        assert!(needs_project_message(&reply.ineligible_reason()).starts_with("不再提供个人版。"));
        assert!(reply.validation_needed().is_none());
        let reply = LoadReply::parse(&json!({
            "ineligibleTiers": [{ "reasonCode": "VALIDATION_REQUIRED", "reasonMessage": "验证", "validationUrl": "https://v.test" }],
        }));
        assert!(
            reply
                .validation_needed()
                .unwrap()
                .contains("https://v.test")
        );
        assert_eq!(
            LoadReply::parse(&json!({})).default_tier().id,
            "legacy-tier"
        );
        assert_eq!(project_id_of(Some(&json!("plain"))), "plain");
    }

    #[test]
    fn token_reply_sets_expiry_in_ms() {
        let before = oauth::now_ms();
        let credentials = parse_token_reply(
            &json!({ "access_token": "a", "expires_in": 3599, "refresh_token": "r" }),
        )
        .unwrap();
        assert!(credentials.expires_at >= before + 3_599_000);
        assert_eq!(credentials.refresh_token, "r");
        assert!(parse_token_reply(&json!({ "error": "x" })).is_err());
    }

    #[test]
    fn project_id_shapes() {
        assert!(looks_like_project_id("my-project-123"));
        assert!(looks_like_project_id("example.com:my-proj"));
        assert!(!looks_like_project_id("My-Project"));
        assert!(!looks_like_project_id("short"));
        assert!(!looks_like_project_id("ends-with-"));
    }
}
